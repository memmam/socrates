//! GPU compute for the `gpu` builtin namespace.
//!
//! This module is **always compiled**; only its internals are gated on the
//! `gpu` cargo feature (the project's sole optional dependency, wgpu +
//! pollster). Without the feature every entry point degrades gracefully:
//! [`available`] is `false`, [`adapter_info`] and [`run`] report that gpu
//! support is not compiled in. The default build therefore stays
//! zero-dependency.
//!
//! # Shader ABI (the `gpu.run` contract)
//!
//! [`run`] executes one dispatch of a WGSL compute shader. The shader must
//! define:
//!
//! ```wgsl
//! @group(0) @binding(0) var<storage, read> input: array<u32>;   // or array<f32>, ...
//! @group(0) @binding(1) var<storage, read_write> output: array<u32>;
//!
//! @compute @workgroup_size(1)          // any workgroup size
//! fn main(@builtin(global_invocation_id) gid: vec3<u32>) { ... }
//! ```
//!
//! - entry point named `main`, marked `@compute`;
//! - binding 0 of group 0: a read-only storage buffer, initialized with the
//!   caller's input bytes (input must be non-empty and a multiple of 4 bytes,
//!   per WebGPU storage-binding rules);
//! - binding 1 of group 0: a read-write storage buffer of `out_len` bytes
//!   (`out_len` must be positive, a multiple of 4, and at most 256 MiB),
//!   zero-initialized, copied back to the caller after the dispatch;
//! - the dispatch is `(wx, wy, wz)` workgroups (each in `1..=65535`).
//!
//! Every failure — no adapter, shader compile/validation errors, device loss,
//! bad sizes — returns `Err(String)` with a human-readable message; nothing
//! in here panics the interpreter.

/// Upper bound on the output buffer size (and on input, symmetrically).
pub const MAX_BUFFER_BYTES: usize = 256 * 1024 * 1024;

/// Largest workgroup count per dimension (the WebGPU default limit).
pub const MAX_WORKGROUPS_PER_DIM: u32 = 65_535;

/// Validate the buffer-size and dispatch-size arguments. Shared by both
/// builds so argument errors are identical with and without the feature.
fn validate(input: &[u8], out_len: usize, wx: u32, wy: u32, wz: u32) -> Result<(), String> {
    if input.is_empty() {
        return Err("gpu.run: input must not be empty (storage buffers cannot be zero-sized)"
            .to_string());
    }
    if !input.len().is_multiple_of(4) {
        return Err(format!(
            "gpu.run: input length must be a multiple of 4 bytes, got {}",
            input.len()
        ));
    }
    if input.len() > MAX_BUFFER_BYTES {
        return Err(format!(
            "gpu.run: input length {} exceeds the {} MiB cap",
            input.len(),
            MAX_BUFFER_BYTES / (1024 * 1024)
        ));
    }
    if out_len == 0 {
        return Err("gpu.run: out_len must be positive".to_string());
    }
    if !out_len.is_multiple_of(4) {
        return Err(format!("gpu.run: out_len must be a multiple of 4 bytes, got {out_len}"));
    }
    if out_len > MAX_BUFFER_BYTES {
        return Err(format!(
            "gpu.run: out_len {} exceeds the {} MiB cap",
            out_len,
            MAX_BUFFER_BYTES / (1024 * 1024)
        ));
    }
    for (n, name) in [(wx, "x"), (wy, "y"), (wz, "z")] {
        if n == 0 || n > MAX_WORKGROUPS_PER_DIM {
            return Err(format!(
                "gpu.run: workgroup count {name} must be in 1..={MAX_WORKGROUPS_PER_DIM}, got {n}"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Feature OFF: graceful stubs
// ---------------------------------------------------------------------------

/// Is a GPU adapter available? Always `false` without the `gpu` feature.
#[cfg(not(feature = "gpu"))]
pub fn available() -> bool {
    false
}

/// Describe the adapter `gpu.run` would use.
#[cfg(not(feature = "gpu"))]
pub fn adapter_info() -> String {
    "gpu support not compiled in".to_string()
}

/// Run a compute shader (see the module docs for the ABI). Always an error
/// without the `gpu` feature.
#[cfg(not(feature = "gpu"))]
pub fn run(
    _wgsl: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    // Argument errors first, so bad calls fail identically in both builds.
    validate(input, out_len, wx, wy, wz)?;
    Err("gpu support not compiled in (build with --features gpu)".to_string())
}

// ---------------------------------------------------------------------------
// Feature ON: wgpu implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "gpu")]
fn request_adapter() -> Result<wgpu::Adapter, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))
}

/// Is a GPU adapter available (any backend, software rasterizers included)?
#[cfg(feature = "gpu")]
pub fn available() -> bool {
    request_adapter().is_ok()
}

/// Describe the adapter `gpu.run` would use: `"<name> (<backend>)"`, or
/// `"no adapter"` when none can be acquired.
#[cfg(feature = "gpu")]
pub fn adapter_info() -> String {
    match request_adapter() {
        Ok(adapter) => {
            let info = adapter.get_info();
            format!("{} ({})", info.name, info.backend)
        }
        Err(_) => "no adapter".to_string(),
    }
}

/// Run one dispatch of a WGSL compute shader (module docs describe the ABI):
/// upload `input` to binding 0, dispatch `(wx, wy, wz)` workgroups of the
/// `main` entry point, and read back `out_len` bytes from binding 1.
#[cfg(feature = "gpu")]
pub fn run(
    wgsl: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;

    let adapter = request_adapter()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("fable gpu.run"),
        ..Default::default()
    }))
    .map_err(|e| format!("failed to acquire device: {e}"))?;

    // Route errors to us instead of wgpu's default panicking handler, and
    // wrap all device work in error scopes: wgpu reports validation failures
    // (shader compile errors included) asynchronously, not as Results.
    device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
        eprintln!("gpu.run: uncaptured device error: {e}");
    }));
    let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gpu.run shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(wgsl)),
    });

    let input_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu.run input"),
        size: input.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    input_buf
        .slice(..)
        .get_mapped_range_mut()
        .map_err(|e| format!("gpu.run: failed to write input buffer: {e}"))?
        .copy_from_slice(input);
    input_buf.unmap();

    let output_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu.run output"),
        size: out_len as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu.run staging"),
        size: out_len as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("gpu.run pipeline"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gpu.run bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: input_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: output_buf.as_entire_binding() },
        ],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu.run pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(wx, wy, wz);
    }
    encoder.copy_buffer_to_buffer(&output_buf, 0, &staging_buf, 0, out_len as u64);
    queue.submit(Some(encoder.finish()));

    // Surface any validation / OOM error recorded above (scopes pop in
    // reverse push order). Shader compile errors land here: with
    // `layout: None`, pipeline creation validates the WGSL and its entry
    // point/bindings.
    let validation = pollster::block_on(validation_scope.pop());
    let oom = pollster::block_on(oom_scope.pop());
    if let Some(e) = validation {
        return Err(format!("gpu.run: {e}"));
    }
    if let Some(e) = oom {
        return Err(format!("gpu.run: out of GPU memory: {e}"));
    }

    // Map the staging buffer and copy the result out.
    let slice = staging_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("gpu.run: device poll failed: {e}"))?;
    match rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("gpu.run: failed to map output buffer: {e}")),
        Err(_) => return Err("gpu.run: device dropped the map request".to_string()),
    }
    let data = slice
        .get_mapped_range()
        .map_err(|e| format!("gpu.run: failed to read output buffer: {e}"))?
        .to_vec();
    staging_buf.unmap();
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build-independent: argument validation fires before any device work.
    #[test]
    fn run_validates_arguments() {
        assert!(run("", &[], 4, 1, 1, 1).unwrap_err().contains("input must not be empty"));
        assert!(run("", &[0; 3], 4, 1, 1, 1).unwrap_err().contains("multiple of 4"));
        assert!(run("", &[0; 4], 0, 1, 1, 1).unwrap_err().contains("out_len must be positive"));
        assert!(run("", &[0; 4], 6, 1, 1, 1).unwrap_err().contains("multiple of 4"));
        assert!(run("", &[0; 4], MAX_BUFFER_BYTES + 4, 1, 1, 1)
            .unwrap_err()
            .contains("cap"));
        assert!(run("", &[0; 4], 4, 0, 1, 1).unwrap_err().contains("workgroup count x"));
        assert!(run("", &[0; 4], 4, 1, 1, 70_000).unwrap_err().contains("workgroup count z"));
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn stubs_without_feature() {
        assert!(!available());
        assert_eq!(adapter_info(), "gpu support not compiled in");
        let err = run("@compute fn main() {}", &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert_eq!(err, "gpu support not compiled in (build with --features gpu)");
    }

    // With the feature on, run() must fail gracefully (Err, not panic) even
    // when no adapter exists, and report shader errors when one does.
    #[cfg(feature = "gpu")]
    #[test]
    fn run_never_panics_without_adapter_or_with_bad_shader() {
        let r = run("not wgsl at all", &[0; 4], 4, 1, 1, 1);
        assert!(r.is_err(), "bad shader or missing adapter must be an Err");
    }
}
