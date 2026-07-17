//! GPU compute for the `gpu` builtin namespace.
//!
//! This module is **always compiled**; the backends behind it are the four
//! **native, zero-dependency** paths of CLAUDE.md's roadmap — Metal (MSL
//! source via [`run`]; `--features metal`, Apple Silicon macOS), Vulkan and
//! OpenCL (SPIR-V binaries via [`run_spirv`], each in its own profile;
//! `--features vulkan` / `opencl`, Linux/Windows), and CUDA (PTX source via
//! [`run`]; `--features cuda`, Linux/Windows). Where several are compiled
//! in, the precedence is vulkan > cuda > opencl (metal is alone on macOS).
//! wgpu, once the interpreter's only
//! dependency, is **gone**: the native coverage condition (Metal + Vulkan +
//! one of OpenCL/DirectX) was met and the roadmap retired it — every build
//! of Fable, not just the default, is now zero-dependency. Without a
//! backend every entry point degrades gracefully: [`available`] is `false`,
//! [`adapter_info`] and [`run`] report that gpu support is not compiled in.
//!
//! # The `gpu.run` / `gpu.run_spirv` contract (shared across backends)
//!
//! One dispatch of a compute kernel, whose dialect the active backend fixes
//! (branch on [`backend`]): MSL source for `run` on metal, PTX source for
//! `run` on cuda; a Vulkan-profile or OpenCL-profile SPIR-V binary for
//! `run_spirv` (SPEC § 7.2 documents all four ABIs). Whatever the dialect:
//!
//! - two buffers: the caller's input bytes (non-empty, a multiple of 4),
//!   and `out_len` output bytes (positive, a multiple of 4, at most
//!   256 MiB), **zero-initialized** and copied back after the dispatch;
//! - the dispatch covers the `(wx, wy, wz)` index space (each count in
//!   `1..=65535`), with per-backend workgroup semantics documented at each
//!   backend's section;
//! - argument validation is [`validate`], shared so bad calls fail with
//!   byte-identical messages in every build.
//!
//! Every failure — no adapter, shader compile/validation errors, device loss,
//! bad sizes — returns `Err(String)` with a human-readable message; nothing
//! in here panics the interpreter.

/// Which implementation the `gpu` namespace dispatches to in THIS build:
/// `"metal"` (native raw-FFI, macOS/Apple Silicon with `--features
/// metal`), `"vulkan"` / `"cuda"` / `"opencl"` (native raw-dlopen FFI on
/// Linux/Windows — that order is the precedence when several are compiled
/// in: vulkan is the CI-proven universal path, and the vendor GPU path
/// beats the often-CPU OpenCL), or `"none"`. The `gpu` analog of
/// `win.backend_name()`: programs branch on it to pick the shader dialect
/// and entry point — and, for `gpu.run_spirv`, the SPIR-V *profile* the
/// binary must use (Vulkan's GLCompute/Logical vs. OpenCL's
/// Kernel/Physical64; see SPEC § 7.2).
#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn backend() -> &'static str {
    "metal"
}
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn backend() -> &'static str {
    "vulkan"
}
#[cfg(all(
    feature = "cuda",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn backend() -> &'static str {
    "cuda"
}
#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    not(feature = "cuda"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn backend() -> &'static str {
    "opencl"
}
#[cfg(all(
    not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")),
    not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows")))
))]
pub fn backend() -> &'static str {
    "none"
}

/// Upper bound on the output buffer size (and on input, symmetrically).
pub const MAX_BUFFER_BYTES: usize = 256 * 1024 * 1024;

/// Largest workgroup count per dimension (the WebGPU default limit).
pub const MAX_WORKGROUPS_PER_DIM: u32 = 65_535;

/// Validate the buffer-size and dispatch-size arguments. Shared by every
/// backend (and the stub) so argument errors are byte-identical in every
/// build.
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
// No backend compiled in: graceful stubs
// ---------------------------------------------------------------------------

/// Is a GPU adapter available? Always `false` without a backend.
#[cfg(all(not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")), not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows")))))]
pub fn available() -> bool {
    false
}

/// Describe the adapter `gpu.run` would use.
#[cfg(all(not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")), not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows")))))]
pub fn adapter_info() -> String {
    "gpu support not compiled in".to_string()
}

/// Run a compute shader (see the module docs for the ABI). Always an error
/// without a backend.
#[cfg(all(not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")), not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows")))))]
pub fn run(
    _src: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    // Argument errors first, so bad calls fail identically in every build.
    validate(input, out_len, wx, wy, wz)?;
    Err("gpu support not compiled in (build with --features metal on Apple Silicon macOS, or \
         --features vulkan, cuda, or opencl on Linux/Windows)"
        .to_string())
}

// ---------------------------------------------------------------------------
// Native Metal backend (macOS/Apple Silicon with --features metal): raw FFI
// over crate::mtl/crate::objc — the roadmap's first native compute backend
// (CLAUDE.md).
//
// # The MSL `gpu.run` ABI (the module-docs contract, in MSL)
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
//   shared two-buffer contract from the module docs;
// - the dispatch is `(wx, wy, wz)` *threadgroups of one thread each*, so
//   `thread_position_in_grid` covers exactly the `(wx, wy, wz)` index
//   space — the shape every `gpu.run` kernel to date uses. (Larger
//   threadgroups are an API-side parameter in Metal, not a shader-side
//   declaration; if a future need arises it becomes an explicit new
//   argument, not a silent change.)
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

    /// `"<name> (metal)"` — the `"<name> (<backend>)"` shape every gpu
    /// backend reports, or `"no adapter"` when no device exists.
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
            // zeroed contents, and the zeroed-output contract means every
            // backend must agree on bytes the kernel never wrote.
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
            // as Errs, like every other failure mode here.
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

// ---------------------------------------------------------------------------
// Native Vulkan backend (Linux/Windows with --features vulkan): raw dlopen
// FFI over src/vk.rs — the first consumer of the SPIR-V lingua-franca
// decision. `gpu.run_spirv` takes the SPIR-V *binary* as `Bytes` (a
// sibling entry point rather than an overload of `gpu.run`, for the same
// no-default-params/no-overloading reason window.create_metal is a sibling
// of window.create); its ABI is the module-docs contract: entry point
// `main` (SPIR-V has no reserved names, and every GLSL toolchain emits
// `main`), storage buffers at set 0 bindings 0/1, `(wx, wy, wz)` workgroups
// whose size the SPIR-V module itself declares (LocalSize), shared
// `validate` for identical argument errors.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn available() -> bool {
    crate::vk::available()
}
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn adapter_info() -> String {
    crate::vk::adapter_info()
}
/// On the vulkan backend, `gpu.run`'s source-text entry point has no
/// meaning — SPIR-V is binary, and pretending otherwise (base64 in a
/// String, say) would launder the type instead of admitting the format.
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn run(
    _src: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    Err("gpu.run: the vulkan backend takes SPIR-V binaries via gpu.run_spirv (gpu.backend() \
         == \"vulkan\")"
        .to_string())
}

/// One compute dispatch of a SPIR-V binary — `gpu.run`'s `Bytes`-shader
/// sibling. Only the vulkan backend ingests SPIR-V today (OpenCL 2.1+ and
/// GL 4.6 join it later, per CLAUDE.md's roadmap); every other build
/// reports which entry point its backend actually wants.
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn run_spirv(
    spirv: &[u8],
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    crate::vk::run_spirv(spirv, input, out_len, wx, wy, wz)
}
// ---------------------------------------------------------------------------
// Native CUDA backend (Linux/Windows with --features cuda, when vulkan is
// not also compiled in — the native precedence order is metal > vulkan >
// cuda > opencl): raw dlopen FFI over src/cu.rs. CUDA's kernel input is
// PTX, NVIDIA's *textual* virtual ISA, which the driver JITs at module
// load — so it rides `gpu.run`'s String argument exactly like MSL does on
// metal, with its own entry-point convention (`.visible .entry main`,
// two `.param .u64` global pointers) and `(wx, wy, wz)` as a grid of
// single-thread blocks (`%ctaid` spans the index space — the Metal
// threadgroup shape restated). `gpu.run_spirv` on this backend redirects:
// the driver API has no SPIR-V ingestion.
// ---------------------------------------------------------------------------

#[cfg(all(
    feature = "cuda",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn available() -> bool {
    crate::cu::available()
}
#[cfg(all(
    feature = "cuda",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn adapter_info() -> String {
    crate::cu::adapter_info()
}
/// One dispatch of a PTX kernel (see the section comment above and SPEC
/// § 7.2 for the PTX ABI).
#[cfg(all(
    feature = "cuda",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn run(
    ptx: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    crate::cu::run(ptx, input, out_len, wx, wy, wz)
}
/// On the cuda backend, SPIR-V has no ingestion path — the driver API
/// takes PTX text, so `run_spirv` redirects to `gpu.run`.
#[cfg(all(
    feature = "cuda",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn run_spirv(
    _spirv: &[u8],
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    Err("gpu.run_spirv: the cuda backend takes PTX source via gpu.run (gpu.backend() == \
         \"cuda\")"
        .to_string())
}

// ---------------------------------------------------------------------------
// Native OpenCL backend (Linux/Windows with --features opencl, when neither
// vulkan nor cuda is also compiled in — the native precedence order is
// metal > vulkan > cuda > opencl): raw dlopen FFI over src/cl.rs — the
// SECOND SPIR-V consumer, and the one that forced the profile distinction:
// SPIR-V is the lingua-franca *format*, but `clCreateProgramWithIL` ingests
// only the OpenCL dialect (Kernel execution model, Physical64 addressing,
// OpenCL memory model, buffers as CrossWorkgroup pointer kernel arguments),
// not Vulkan's (GLCompute/Logical/GLSL450/descriptor sets). gpu.run_spirv's
// contract is therefore: the blob matches the active backend's profile, and
// gpu.backend() is the branch point — the same rule that picks GLSL vs.
// MSL for source-text shaders. The ABI is otherwise the established
// one: entry point `main`, input/output as the two kernel arguments,
// `(wx, wy, wz)` covering the same invocation index space (the global work
// size — a Vulkan module with LocalSize 1 1 1 dispatches identically),
// shared `validate` for identical argument errors.
// ---------------------------------------------------------------------------

#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    not(feature = "cuda"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn available() -> bool {
    crate::cl::available()
}
#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    not(feature = "cuda"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn adapter_info() -> String {
    crate::cl::adapter_info()
}
/// On the opencl backend, `gpu.run`'s source-text entry point has no
/// meaning — the same binary-format honesty as the vulkan arm above.
#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    not(feature = "cuda"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn run(
    _src: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    Err("gpu.run: the opencl backend takes SPIR-V binaries via gpu.run_spirv (gpu.backend() \
         == \"opencl\")"
        .to_string())
}

/// One dispatch of an OpenCL-profile SPIR-V kernel (see the section comment
/// above and SPEC § 7.2 for the profile ABI).
#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    not(feature = "cuda"),
    any(target_os = "linux", target_os = "windows")
))]
pub fn run_spirv(
    spirv: &[u8],
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    validate(input, out_len, wx, wy, wz)?;
    crate::cl::run_spirv(spirv, input, out_len, wx, wy, wz)
}

#[cfg(not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows"))))]
pub fn run_spirv(
    _spirv: &[u8],
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    // Argument errors first, identical to the vulkan build.
    validate(input, out_len, wx, wy, wz)?;
    Err(match backend() {
        "metal" => "gpu.run_spirv: the metal backend takes MSL source via gpu.run".to_string(),
        _ => "gpu.run_spirv: no SPIR-V backend compiled in (build with --features vulkan or \
              opencl on Linux/Windows)"
            .to_string(),
    })
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

    #[cfg(all(not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")), not(all(any(feature = "vulkan", feature = "opencl", feature = "cuda"), any(target_os = "linux", target_os = "windows")))))]
    #[test]
    fn stubs_without_backend() {
        assert!(!available());
        assert_eq!(backend(), "none");
        assert_eq!(adapter_info(), "gpu support not compiled in");
        let err = run("kernel", &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert_eq!(
            err,
            "gpu support not compiled in (build with --features metal on Apple Silicon macOS, \
             or --features vulkan, cuda, or opencl on Linux/Windows)"
        );
    }

    // With a backend on, run() must fail gracefully (Err, not panic) even
    // when no adapter exists, and report shader errors when one does.
    #[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn run_never_panics_without_adapter_or_with_bad_shader() {
        let r = run("not wgsl at all", &[0; 4], 4, 1, 1, 1);
        assert!(r.is_err(), "bad shader or missing adapter must be an Err");
    }

    // The native precedence order: vulkan beats opencl when both are
    // compiled in.
    #[cfg(all(
        feature = "vulkan",
        feature = "opencl",
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn opencl_defers_to_vulkan() {
        assert_eq!(backend(), "vulkan");
    }

    #[cfg(all(
        feature = "opencl",
        not(feature = "vulkan"),
        not(feature = "cuda"),
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn opencl_run_redirects_to_run_spirv() {
        assert_eq!(backend(), "opencl");
        let err = run("kernel", &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert!(err.contains("gpu.run_spirv"), "got: {err}");
    }

    // The full precedence lattice around cuda.
    #[cfg(all(
        feature = "vulkan",
        feature = "cuda",
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn cuda_defers_to_vulkan() {
        assert_eq!(backend(), "vulkan");
    }
    #[cfg(all(
        feature = "cuda",
        feature = "opencl",
        not(feature = "vulkan"),
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn opencl_defers_to_cuda() {
        assert_eq!(backend(), "cuda");
    }

    // On the cuda backend: run_spirv redirects to PTX-via-run, a NUL byte
    // in the PTX errs before any FFI, and a driverless machine (this dev
    // container, every CI runner) errs cleanly — never panics.
    #[cfg(all(
        feature = "cuda",
        not(feature = "vulkan"),
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn cuda_run_spirv_redirects_and_run_never_panics() {
        assert_eq!(backend(), "cuda");
        let err = run_spirv(&[0u8; 20], &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert!(err.contains("gpu.run"), "got: {err}");
        let err = run("bad\0ptx", &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert!(err.contains("NUL"), "got: {err}");
        let r = run(".version 7.0", &[0; 4], 4, 1, 1, 1);
        assert!(r.is_err(), "a driverless or PTX-rejecting machine must Err");
    }

    // Never panics, whatever the machine has: a non-SPIR-V blob errs on the
    // magic check; a valid-magic header errs on the missing runtime (this
    // dev container), a build failure (a header is not a module), or worse
    // — but always an Err, never a crash.
    #[cfg(all(
        feature = "opencl",
        not(feature = "vulkan"),
        not(feature = "cuda"),
        any(target_os = "linux", target_os = "windows")
    ))]
    #[test]
    fn opencl_run_spirv_never_panics() {
        let bad_magic = run_spirv(&[0u8; 20], &[0; 4], 4, 1, 1, 1).unwrap_err();
        assert!(bad_magic.contains("not a SPIR-V binary"), "got: {bad_magic}");
        let header_only: Vec<u8> = [0x0723_0203u32, 0x0001_0000, 0, 1, 0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let r = run_spirv(&header_only, &[0; 4], 4, 1, 1, 1);
        assert!(r.is_err(), "a bare header must never dispatch");
    }
}
