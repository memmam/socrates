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

/// Which implementation `gpu.run` dispatches to in THIS build: `"metal"`
/// (native raw-FFI, macOS/Apple Silicon with `--features metal` — takes
/// precedence over wgpu when both are compiled in, per CLAUDE.md's
/// native-backends-first roadmap), `"wgpu"` (the feature-gated portable
/// fallback, which remains until full native coverage retires it), or
/// `"none"`. The `gpu` analog of `win.backend_name()`: programs branch on
/// it to pick the shader dialect `gpu.run` expects (MSL vs. WGSL).
#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn backend() -> &'static str {
    "metal"
}
#[cfg(all(
    feature = "gpu",
    not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))
))]
pub fn backend() -> &'static str {
    "wgpu"
}
#[cfg(all(
    not(feature = "gpu"),
    not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))
))]
pub fn backend() -> &'static str {
    "none"
}

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
#[cfg(all(not(feature = "gpu"), not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
pub fn available() -> bool {
    false
}

/// Describe the adapter `gpu.run` would use.
#[cfg(all(not(feature = "gpu"), not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
pub fn adapter_info() -> String {
    "gpu support not compiled in".to_string()
}

/// Run a compute shader (see the module docs for the ABI). Always an error
/// without the `gpu` feature.
#[cfg(all(not(feature = "gpu"), not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
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
    Err("gpu support not compiled in (build with --features gpu, or --features metal on Apple Silicon macOS)".to_string())
}

// ---------------------------------------------------------------------------
// Feature ON: wgpu implementation
// ---------------------------------------------------------------------------

#[cfg(all(feature = "gpu", not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
fn request_adapter() -> Result<wgpu::Adapter, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))
}

/// Is a GPU adapter available (any backend, software rasterizers included)?
#[cfg(all(feature = "gpu", not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
pub fn available() -> bool {
    request_adapter().is_ok()
}

/// Describe the adapter `gpu.run` would use: `"<name> (<backend>)"`, or
/// `"no adapter"` when none can be acquired.
#[cfg(all(feature = "gpu", not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
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
#[cfg(all(feature = "gpu", not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
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

// ---------------------------------------------------------------------------
// Native Metal backend (macOS/Apple Silicon with --features metal): raw FFI
// over crate::mtl/crate::objc, no wgpu involved. Takes precedence over the
// wgpu path when both are compiled in — the roadmap's native-backends-first
// rule (CLAUDE.md); wgpu remains the fallback elsewhere until full native
// coverage retires it.
//
// # The MSL `gpu.run` ABI (mirrors the WGSL ABI in the module docs)
//
// ```msl
// #include <metal_stdlib>
// using namespace metal;
// kernel void compute_main(device const uint* input  [[buffer(0)]],
//                          device uint*       output [[buffer(1)]],
//                          uint3 gid [[thread_position_in_grid]]) { ... }
// ```
//
// - entry point named `compute_main` (MSL reserves `main`; this matches the
//   graphics backend's `vertex_main`/`fragment_main` convention);
// - `[[buffer(0)]]`: the caller's input bytes; `[[buffer(1)]]`: `out_len`
//   zero-initialized output bytes, copied back after the dispatch — the
//   same two-binding contract as WGSL's `@binding(0)`/`@binding(1)`;
// - the dispatch is `(wx, wy, wz)` *threadgroups of one thread each*, so
//   `thread_position_in_grid` covers exactly the same index space as a
//   WGSL shader with `@workgroup_size(1)` — the shape every `gpu.run`
//   kernel to date uses. (Larger threadgroups are an API-side parameter in
//   Metal, not a shader-side declaration like WGSL's; if a future need
//   arises it becomes an explicit new argument, not a silent change.)
// - argument validation, size caps, and workgroup limits are `validate`,
//   identical across all three builds.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn available() -> bool {
    metal_native::available()
}
#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn adapter_info() -> String {
    metal_native::adapter_info()
}
#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn run(
    msl: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    metal_native::run(msl, input, out_len, wx, wy, wz)
}

#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
mod metal_native {
    use std::sync::OnceLock;

    use crate::mtl::{
        new_library, send_dispatch_threadgroups, send_new_buffer, send_new_buffer_len,
        send_new_compute_pipeline, send_set_buffer, MtlSize, MTLCreateSystemDefaultDevice,
        MTL_RESOURCE_STORAGE_MODE_SHARED,
    };
    use crate::objc::{
        ns_string, nsstring_to_owned, sel, send0, send0_void, send1_obj, send1_obj_obj,
        AutoreleasePool, Object,
    };

    /// The process-lifetime device + queue, created once. Both are
    /// documented thread-safe in Metal (unlike encoders), so sharing them
    /// across the interpreter thread and worker isolates is sound; they're
    /// stored as `usize` because raw pointers aren't `Send`/`Sync` even
    /// when the objects they name are.
    fn device_and_queue() -> Option<(*mut Object, *mut Object)> {
        static CELL: OnceLock<(usize, usize)> = OnceLock::new();
        let (d, q) = *CELL.get_or_init(|| unsafe {
            let device = MTLCreateSystemDefaultDevice();
            if device.is_null() {
                return (0, 0);
            }
            let queue = send0(device, sel("newCommandQueue"));
            if queue.is_null() {
                send0_void(device, sel("release"));
                return (0, 0);
            }
            (device as usize, queue as usize)
        });
        if d == 0 {
            None
        } else {
            Some((d as *mut Object, q as *mut Object))
        }
    }

    pub(super) fn available() -> bool {
        device_and_queue().is_some()
    }

    /// `"<name> (metal)"` — same `"<name> (<backend>)"` shape as the wgpu
    /// path's adapter string, or `"no adapter"` when no device exists.
    pub(super) fn adapter_info() -> String {
        match device_and_queue() {
            Some((device, _)) => {
                let _pool = AutoreleasePool::push();
                let name = unsafe { nsstring_to_owned(send0(device, sel("name"))) };
                format!("{name} (metal)")
            }
            None => "no adapter".to_string(),
        }
    }

    pub(super) fn run(
        msl: &str,
        input: &[u8],
        out_len: usize,
        wx: u32,
        wy: u32,
        wz: u32,
    ) -> Result<Vec<u8>, String> {
        super::validate(input, out_len, wx, wy, wz)?;
        let Some((device, queue)) = device_and_queue() else {
            return Err(
                "gpu.run: no adapter (MTLCreateSystemDefaultDevice returned nil)".to_string(),
            );
        };
        let _pool = AutoreleasePool::push();
        // Safety: every fallible step (a nil return) is checked, and every
        // +1 object created before a failure is released on that path —
        // the same no-partial-leaks discipline the graphics backend holds.
        unsafe {
            let lib = new_library(device, msl).map_err(|e| format!("gpu.run: {e}"))?;
            let fun = send1_obj_obj(lib, sel("newFunctionWithName:"), ns_string("compute_main"));
            send0_void(lib, sel("release")); // the function retains its library
            if fun.is_null() {
                return Err("gpu.run: no kernel function named `compute_main` (the Metal \
                            backend's fixed entry-point convention — see SPEC § 7.2)"
                    .to_string());
            }
            let mut err: *mut Object = std::ptr::null_mut();
            let pso = send_new_compute_pipeline(
                device,
                sel("newComputePipelineStateWithFunction:error:"),
                fun,
                &mut err,
            );
            send0_void(fun, sel("release"));
            if pso.is_null() {
                let msg = if err.is_null() {
                    "unknown pipeline-state error".to_string()
                } else {
                    nsstring_to_owned(send0(err, sel("localizedDescription")))
                };
                return Err(format!("gpu.run: {msg}"));
            }

            let inbuf = send_new_buffer(
                device,
                sel("newBufferWithBytes:length:options:"),
                input.as_ptr() as *const std::ffi::c_void,
                input.len() as u64,
                MTL_RESOURCE_STORAGE_MODE_SHARED,
            );
            if inbuf.is_null() {
                send0_void(pso, sel("release"));
                return Err("gpu.run: input buffer allocation failed".to_string());
            }
            let outbuf = send_new_buffer_len(
                device,
                sel("newBufferWithLength:options:"),
                out_len as u64,
                MTL_RESOURCE_STORAGE_MODE_SHARED,
            );
            if outbuf.is_null() {
                send0_void(inbuf, sel("release"));
                send0_void(pso, sel("release"));
                return Err("gpu.run: output buffer allocation failed".to_string());
            }
            // Zero-initialize: `newBufferWithLength:` does not guarantee
            // zeroed contents, and the wgpu path's output starts zeroed —
            // the two backends must agree on bytes the kernel never wrote.
            let out_ptr = send0(outbuf, sel("contents")) as *mut u8;
            if out_ptr.is_null() {
                send0_void(outbuf, sel("release"));
                send0_void(inbuf, sel("release"));
                send0_void(pso, sel("release"));
                return Err("gpu.run: output buffer has no CPU mapping".to_string());
            }
            std::ptr::write_bytes(out_ptr, 0, out_len);

            let cmd = send0(queue, sel("commandBuffer"));
            let enc = if cmd.is_null() {
                std::ptr::null_mut()
            } else {
                send0(cmd, sel("computeCommandEncoder"))
            };
            if enc.is_null() {
                send0_void(outbuf, sel("release"));
                send0_void(inbuf, sel("release"));
                send0_void(pso, sel("release"));
                return Err("gpu.run: failed to create a command encoder".to_string());
            }
            send1_obj(enc, sel("setComputePipelineState:"), pso);
            send_set_buffer(enc, sel("setBuffer:offset:atIndex:"), inbuf, 0, 0);
            send_set_buffer(enc, sel("setBuffer:offset:atIndex:"), outbuf, 0, 1);
            send_dispatch_threadgroups(
                enc,
                sel("dispatchThreadgroups:threadsPerThreadgroup:"),
                MtlSize {
                    width: wx as u64,
                    height: wy as u64,
                    depth: wz as u64,
                },
                MtlSize {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
            );
            send0_void(enc, sel("endEncoding"));
            send0_void(cmd, sel("commit"));
            send0_void(cmd, sel("waitUntilCompleted"));

            // Surface execution failures (device loss, invalid dispatch)
            // the way the wgpu path surfaces its error-scope results.
            let cmd_err = send0(cmd, sel("error"));
            if !cmd_err.is_null() {
                let msg = nsstring_to_owned(send0(cmd_err, sel("localizedDescription")));
                send0_void(outbuf, sel("release"));
                send0_void(inbuf, sel("release"));
                send0_void(pso, sel("release"));
                return Err(format!("gpu.run: {msg}"));
            }

            let mut out = vec![0u8; out_len];
            std::ptr::copy_nonoverlapping(out_ptr, out.as_mut_ptr(), out_len);
            send0_void(outbuf, sel("release"));
            send0_void(inbuf, sel("release"));
            send0_void(pso, sel("release"));
            Ok(out)
        }
    }
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

    #[cfg(all(not(feature = "gpu"), not(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))))]
    #[test]
    fn stubs_without_feature() {
        assert!(!available());
        assert_eq!(adapter_info(), "gpu support not compiled in");
        let err = run("@compute fn main() {}", &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert_eq!(err, "gpu support not compiled in (build with --features gpu, or --features metal on Apple Silicon macOS)");
    }

    // With the feature on, run() must fail gracefully (Err, not panic) even
    // when no adapter exists, and report shader errors when one does.
    #[cfg(any(feature = "gpu", all(feature = "metal", target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn run_never_panics_without_adapter_or_with_bad_shader() {
        let r = run("not wgsl at all", &[0; 4], 4, 1, 1, 1);
        assert!(r.is_err(), "bad shader or missing adapter must be an Err");
    }
}
