//! Raw-FFI OpenCL compute for the `gpu` namespace — the roadmap's third
//! native compute backend (after Metal and Vulkan), and the second SPIR-V
//! consumer: `gpu.run_spirv` takes a SPIR-V *binary* (`Bytes`) that
//! `clCreateProgramWithIL` (OpenCL 2.1+ core) ingests directly — no
//! compiler, no translator, no dependency. SPIR-V is the lingua-franca
//! *format*, but compute kernels come in two dialects: Vulkan's profile
//! (GLCompute execution model, Logical addressing, GLSL450 memory model,
//! descriptor-set storage buffers) and OpenCL's (Kernel execution model,
//! Physical64 addressing, OpenCL memory model, buffers as CrossWorkgroup
//! pointer kernel arguments). Each backend ingests only its own profile —
//! `gpu.backend()` is the branch point, the same rule that picks GLSL vs.
//! MSL for source-text shaders (SPEC § 7.2 documents both ABIs).
//!
//! **Zero dependencies**: the OpenCL ICD loader is resolved at runtime
//! with `dlopen("libOpenCL.so.1")` on Linux / `LoadLibraryA("OpenCL.dll")`
//! on Windows — the same dynamic-resolution strategy `vk.rs` uses for
//! `libvulkan`, and for the same reason: the loader ships with GPU drivers
//! (and with CPU implementations like pocl), not with the OS's link-time
//! SDK. Unlike Vulkan there is no `GetProcAddr` indirection — every entry
//! point is a direct export of the ICD loader, resolved once per process
//! with `dlsym`. macOS is deliberately absent: Apple's OpenCL is frozen at
//! 1.2 (no IL ingestion), and the native Metal backend covers that
//! platform.
//!
//! **Per-call lifecycle, deliberately**: each `run_spirv` builds and tears
//! down the whole context→queue→program→kernel chain, exactly like the
//! Vulkan compute path — leak-free by construction and thread-safe without
//! shared-handle reasoning (worker isolates can call it concurrently). Per
//! CLAUDE.md's efficiency-pass rule, a cached-context idiom can later
//! become the primitive underneath this exact surface once measured.
//!
//! Cleanup is a `Drop` guard ([`Run`]) releasing in reverse creation
//! order — the Rust-native spelling of the no-partial-leaks discipline —
//! so every early `return Err(...)` tears down whatever exists so far.

#![cfg_attr(
    not(all(
        feature = "opencl",
        not(feature = "vulkan"),
        not(feature = "cuda"),
        any(target_os = "linux", target_os = "windows")
    )),
    allow(dead_code)
)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Loader resolution (dlopen / LoadLibrary), mirroring vk.rs's strategy.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod libloading {
    use std::ffi::{c_char, c_int, c_void};
    #[link(name = "dl")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;
    pub(super) unsafe fn open() -> *mut c_void {
        dlopen(c"libOpenCL.so.1".as_ptr(), RTLD_NOW)
    }
    pub(super) unsafe fn sym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        dlsym(handle, name)
    }
}

#[cfg(target_os = "windows")]
mod libloading {
    use std::ffi::{c_char, c_void};
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub(super) unsafe fn open() -> *mut c_void {
        LoadLibraryA(c"OpenCL.dll".as_ptr())
    }
    pub(super) unsafe fn sym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        GetProcAddress(handle, name)
    }
}

// ---------------------------------------------------------------------------
// Handles, scalars, and constants (OpenCL 1.2/2.1 core, CL/cl.h widths).
// ---------------------------------------------------------------------------

// Every OpenCL handle is an opaque pointer (`struct _cl_platform_id *`...).
type ClPlatformId = *mut c_void;
type ClDeviceId = *mut c_void;
type ClContext = *mut c_void;
type ClCommandQueue = *mut c_void;
type ClProgram = *mut c_void;
type ClKernel = *mut c_void;
type ClMem = *mut c_void;

type ClInt = i32;
type ClUint = u32;
/// `cl_bitfield` (`cl_device_type`, `cl_mem_flags`, `cl_command_queue_properties`).
type ClBitfield = u64;

const CL_SUCCESS: ClInt = 0;
const CL_TRUE: ClUint = 1;
const CL_DEVICE_TYPE_ALL: ClBitfield = 0xFFFF_FFFF;
const CL_DEVICE_TYPE_GPU: ClBitfield = 1 << 2;
const CL_MEM_READ_WRITE: ClBitfield = 1 << 0;
const CL_MEM_READ_ONLY: ClBitfield = 1 << 2;
const CL_MEM_COPY_HOST_PTR: ClBitfield = 1 << 5;
// clGetDeviceInfo / clGetProgramBuildInfo parameter names.
const CL_DEVICE_TYPE: ClUint = 0x1000;
const CL_DEVICE_NAME: ClUint = 0x102B;
/// `CL_DEVICE_IL_VERSION` (2.1+): a space-separated list like
/// `"SPIR-V_1.2"`; empty or unqueryable on runtimes without IL ingestion.
const CL_DEVICE_IL_VERSION: ClUint = 0x105B;
const CL_PROGRAM_BUILD_LOG: ClUint = 0x1183;
/// `cl_context_properties` key binding a context to one platform.
const CL_CONTEXT_PLATFORM: isize = 0x1084;

// ---------------------------------------------------------------------------
// Function-pointer types and the once-per-process resolved table.
// ---------------------------------------------------------------------------

type FnGetPlatformIds =
    unsafe extern "system" fn(ClUint, *mut ClPlatformId, *mut ClUint) -> ClInt;
type FnGetDeviceIds = unsafe extern "system" fn(
    ClPlatformId,
    ClBitfield,
    ClUint,
    *mut ClDeviceId,
    *mut ClUint,
) -> ClInt;
type FnGetDeviceInfo =
    unsafe extern "system" fn(ClDeviceId, ClUint, usize, *mut c_void, *mut usize) -> ClInt;
type FnCreateContext = unsafe extern "system" fn(
    *const isize,
    ClUint,
    *const ClDeviceId,
    *mut c_void, // pfn_notify (never installed: null)
    *mut c_void,
    *mut ClInt,
) -> ClContext;
type FnReleaseContext = unsafe extern "system" fn(ClContext) -> ClInt;
/// OpenCL 2.0+ (`properties` is a zero-terminated key/value list; null = defaults).
type FnCreateCommandQueueWithProperties =
    unsafe extern "system" fn(ClContext, ClDeviceId, *const ClBitfield, *mut ClInt) -> ClCommandQueue;
/// OpenCL 1.x (deprecated but universally exported) — the fallback when a
/// runtime predates 2.0.
type FnCreateCommandQueue =
    unsafe extern "system" fn(ClContext, ClDeviceId, ClBitfield, *mut ClInt) -> ClCommandQueue;
type FnReleaseCommandQueue = unsafe extern "system" fn(ClCommandQueue) -> ClInt;
/// OpenCL 2.1+ core — absent on older loaders, so resolved as optional and
/// reported with a version-aware message at call time.
type FnCreateProgramWithIl =
    unsafe extern "system" fn(ClContext, *const c_void, usize, *mut ClInt) -> ClProgram;
type FnBuildProgram = unsafe extern "system" fn(
    ClProgram,
    ClUint,
    *const ClDeviceId,
    *const c_char,
    *mut c_void, // pfn_notify (never installed: null)
    *mut c_void,
) -> ClInt;
type FnGetProgramBuildInfo =
    unsafe extern "system" fn(ClProgram, ClDeviceId, ClUint, usize, *mut c_void, *mut usize) -> ClInt;
type FnReleaseProgram = unsafe extern "system" fn(ClProgram) -> ClInt;
type FnCreateKernel = unsafe extern "system" fn(ClProgram, *const c_char, *mut ClInt) -> ClKernel;
type FnReleaseKernel = unsafe extern "system" fn(ClKernel) -> ClInt;
type FnCreateBuffer =
    unsafe extern "system" fn(ClContext, ClBitfield, usize, *mut c_void, *mut ClInt) -> ClMem;
type FnReleaseMemObject = unsafe extern "system" fn(ClMem) -> ClInt;
type FnSetKernelArg = unsafe extern "system" fn(ClKernel, ClUint, usize, *const c_void) -> ClInt;
type FnEnqueueNdRangeKernel = unsafe extern "system" fn(
    ClCommandQueue,
    ClKernel,
    ClUint,
    *const usize,
    *const usize,
    *const usize,
    ClUint,
    *const c_void,
    *mut c_void,
) -> ClInt;
type FnFinish = unsafe extern "system" fn(ClCommandQueue) -> ClInt;
type FnEnqueueReadBuffer = unsafe extern "system" fn(
    ClCommandQueue,
    ClMem,
    ClUint,
    usize,
    usize,
    *mut c_void,
    ClUint,
    *const c_void,
    *mut c_void,
) -> ClInt;

/// The resolved entry-point table. Function pointers are `Send + Sync`
/// (plain code addresses), so one table serves the whole process.
struct ClFns {
    get_platform_ids: FnGetPlatformIds,
    get_device_ids: FnGetDeviceIds,
    get_device_info: FnGetDeviceInfo,
    create_context: FnCreateContext,
    release_context: FnReleaseContext,
    create_queue_props: Option<FnCreateCommandQueueWithProperties>,
    create_queue_legacy: Option<FnCreateCommandQueue>,
    release_queue: FnReleaseCommandQueue,
    create_program_with_il: Option<FnCreateProgramWithIl>,
    build_program: FnBuildProgram,
    get_program_build_info: FnGetProgramBuildInfo,
    release_program: FnReleaseProgram,
    create_kernel: FnCreateKernel,
    release_kernel: FnReleaseKernel,
    create_buffer: FnCreateBuffer,
    release_mem_object: FnReleaseMemObject,
    set_kernel_arg: FnSetKernelArg,
    enqueue_ndrange_kernel: FnEnqueueNdRangeKernel,
    finish: FnFinish,
    enqueue_read_buffer: FnEnqueueReadBuffer,
}

/// Resolve the loader's exports once per process. `Err` carries what's
/// missing (no library, or a required core-1.0 symbol) so every entry
/// point can report a diagnosable message.
fn fns() -> Result<&'static ClFns, String> {
    static CELL: OnceLock<Result<ClFns, String>> = OnceLock::new();
    CELL.get_or_init(|| unsafe { resolve() })
        .as_ref()
        .map_err(|e| e.clone())
}

unsafe fn resolve() -> Result<ClFns, String> {
    let lib = libloading::open();
    if lib.is_null() {
        return Err("no OpenCL runtime (libOpenCL) on this system".to_string());
    }
    // A core-1.0 export whose absence means a broken loader — the name
    // makes that diagnosable from the message alone (the vk.rs `load!`
    // discipline).
    macro_rules! req {
        ($name:literal, $ty:ty) => {{
            let p = libloading::sym(lib, concat!($name, "\0").as_ptr() as *const c_char);
            if p.is_null() {
                return Err(format!("OpenCL loader has no {}", $name));
            }
            std::mem::transmute::<*mut c_void, $ty>(p)
        }};
    }
    macro_rules! opt {
        ($name:literal, $ty:ty) => {{
            let p = libloading::sym(lib, concat!($name, "\0").as_ptr() as *const c_char);
            if p.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*mut c_void, $ty>(p))
            }
        }};
    }
    let f = ClFns {
        get_platform_ids: req!("clGetPlatformIDs", FnGetPlatformIds),
        get_device_ids: req!("clGetDeviceIDs", FnGetDeviceIds),
        get_device_info: req!("clGetDeviceInfo", FnGetDeviceInfo),
        create_context: req!("clCreateContext", FnCreateContext),
        release_context: req!("clReleaseContext", FnReleaseContext),
        create_queue_props: opt!(
            "clCreateCommandQueueWithProperties",
            FnCreateCommandQueueWithProperties
        ),
        create_queue_legacy: opt!("clCreateCommandQueue", FnCreateCommandQueue),
        release_queue: req!("clReleaseCommandQueue", FnReleaseCommandQueue),
        create_program_with_il: opt!("clCreateProgramWithIL", FnCreateProgramWithIl),
        build_program: req!("clBuildProgram", FnBuildProgram),
        get_program_build_info: req!("clGetProgramBuildInfo", FnGetProgramBuildInfo),
        release_program: req!("clReleaseProgram", FnReleaseProgram),
        create_kernel: req!("clCreateKernel", FnCreateKernel),
        release_kernel: req!("clReleaseKernel", FnReleaseKernel),
        create_buffer: req!("clCreateBuffer", FnCreateBuffer),
        release_mem_object: req!("clReleaseMemObject", FnReleaseMemObject),
        set_kernel_arg: req!("clSetKernelArg", FnSetKernelArg),
        enqueue_ndrange_kernel: req!("clEnqueueNDRangeKernel", FnEnqueueNdRangeKernel),
        finish: req!("clFinish", FnFinish),
        enqueue_read_buffer: req!("clEnqueueReadBuffer", FnEnqueueReadBuffer),
    };
    if f.create_queue_props.is_none() && f.create_queue_legacy.is_none() {
        return Err("OpenCL loader has no clCreateCommandQueue(WithProperties)".to_string());
    }
    Ok(f)
}

// ---------------------------------------------------------------------------
// Device selection, shared by available/adapter_info/run_spirv.
// ---------------------------------------------------------------------------

struct Picked {
    platform: ClPlatformId,
    device: ClDeviceId,
    /// `CL_DEVICE_IL_VERSION`, or `""` when unqueryable (pre-2.1 runtime).
    il: String,
}

/// A string-valued `clGetDeviceInfo` query (size probe, then bytes, then
/// trailing-nul trim). `None` on any failure — pre-2.1 runtimes reject
/// `CL_DEVICE_IL_VERSION` with `CL_INVALID_VALUE`, which is not an error
/// here, just an empty answer.
unsafe fn device_info_string(f: &ClFns, device: ClDeviceId, param: ClUint) -> Option<String> {
    let mut size: usize = 0;
    if (f.get_device_info)(device, param, 0, std::ptr::null_mut(), &mut size) != CL_SUCCESS
        || size == 0
    {
        return None;
    }
    let mut buf = vec![0u8; size];
    if (f.get_device_info)(
        device,
        param,
        size,
        buf.as_mut_ptr() as *mut c_void,
        std::ptr::null_mut(),
    ) != CL_SUCCESS
    {
        return None;
    }
    while buf.last() == Some(&0) {
        buf.pop();
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Best `(platform, device)` pair across every platform: prefer a device
/// whose `CL_DEVICE_IL_VERSION` advertises SPIR-V (the whole point of this
/// backend), then a GPU over a CPU; first found wins ties — so a machine
/// with both a vendor GPU driver and pocl picks the GPU, and a pocl-only
/// machine (CI) still resolves.
unsafe fn pick_device(f: &ClFns) -> Result<Picked, String> {
    let mut nplat: ClUint = 0;
    let r = (f.get_platform_ids)(0, std::ptr::null_mut(), &mut nplat);
    if r != CL_SUCCESS || nplat == 0 {
        return Err("no OpenCL platforms".to_string());
    }
    let mut platforms: Vec<ClPlatformId> = vec![std::ptr::null_mut(); nplat as usize];
    let r = (f.get_platform_ids)(nplat, platforms.as_mut_ptr(), &mut nplat);
    if r != CL_SUCCESS {
        return Err(format!("clGetPlatformIDs failed (CL error {r})"));
    }
    let mut best: Option<(u32, Picked)> = None;
    for &platform in &platforms {
        let mut ndev: ClUint = 0;
        let r = (f.get_device_ids)(
            platform,
            CL_DEVICE_TYPE_ALL,
            0,
            std::ptr::null_mut(),
            &mut ndev,
        );
        if r != CL_SUCCESS || ndev == 0 {
            continue; // a platform with no devices is normal, not an error
        }
        let mut devices: Vec<ClDeviceId> = vec![std::ptr::null_mut(); ndev as usize];
        if (f.get_device_ids)(
            platform,
            CL_DEVICE_TYPE_ALL,
            ndev,
            devices.as_mut_ptr(),
            &mut ndev,
        ) != CL_SUCCESS
        {
            continue;
        }
        for &device in &devices {
            let il = device_info_string(f, device, CL_DEVICE_IL_VERSION).unwrap_or_default();
            let mut dtype: ClBitfield = 0;
            let _ = (f.get_device_info)(
                device,
                CL_DEVICE_TYPE,
                std::mem::size_of::<ClBitfield>(),
                &mut dtype as *mut ClBitfield as *mut c_void,
                std::ptr::null_mut(),
            );
            let mut score = 0;
            if il.contains("SPIR-V") {
                score += 2;
            }
            if dtype & CL_DEVICE_TYPE_GPU != 0 {
                score += 1;
            }
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((
                    score,
                    Picked {
                        platform,
                        device,
                        il,
                    },
                ));
            }
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| "no OpenCL devices".to_string())
}

/// Is an OpenCL device reachable? (Loader present, some platform exposes a
/// device.) IL support is checked at `run_spirv` time with a message that
/// names what the runtime is missing.
pub(crate) fn available() -> bool {
    let Ok(f) = fns() else {
        return false;
    };
    unsafe { pick_device(f).is_ok() }
}

/// `"<CL_DEVICE_NAME> (opencl)"` — the same `"<name> (<backend>)"` shape
/// the other gpu backends report, or `"no adapter"`.
pub(crate) fn adapter_info() -> String {
    let Ok(f) = fns() else {
        return "no adapter".to_string();
    };
    unsafe {
        match pick_device(f) {
            Ok(picked) => {
                let name = device_info_string(f, picked.device, CL_DEVICE_NAME)
                    .unwrap_or_else(|| "unknown".to_string());
                format!("{name} (opencl)")
            }
            Err(_) => "no adapter".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// The Drop guard and the dispatch itself.
// ---------------------------------------------------------------------------

/// Everything `run_spirv` creates, released in reverse creation order by
/// `Drop` (OpenCL handles are reference-counted; each `clCreate*` here is
/// the sole reference).
struct Run {
    f: &'static ClFns,
    context: ClContext,
    queue: ClCommandQueue,
    program: ClProgram,
    kernel: ClKernel,
    inbuf: ClMem,
    outbuf: ClMem,
}

impl Run {
    fn new(f: &'static ClFns) -> Run {
        Run {
            f,
            context: std::ptr::null_mut(),
            queue: std::ptr::null_mut(),
            program: std::ptr::null_mut(),
            kernel: std::ptr::null_mut(),
            inbuf: std::ptr::null_mut(),
            outbuf: std::ptr::null_mut(),
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        unsafe {
            if !self.kernel.is_null() {
                (self.f.release_kernel)(self.kernel);
            }
            if !self.program.is_null() {
                (self.f.release_program)(self.program);
            }
            if !self.outbuf.is_null() {
                (self.f.release_mem_object)(self.outbuf);
            }
            if !self.inbuf.is_null() {
                (self.f.release_mem_object)(self.inbuf);
            }
            if !self.queue.is_null() {
                (self.f.release_queue)(self.queue);
            }
            if !self.context.is_null() {
                (self.f.release_context)(self.context);
            }
        }
    }
}

/// One dispatch of an OpenCL-profile SPIR-V kernel — see `gpu.rs`'s module
/// docs and SPEC § 7.2 for the ABI (entry point `main`, two CrossWorkgroup
/// pointer arguments, `(wx, wy, wz)` as the global work size; the caller
/// has already validated sizes/counts via `gpu::validate`).
pub(crate) fn run_spirv(
    spirv: &[u8],
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    if spirv.len() < 20 || !spirv.len().is_multiple_of(4) {
        return Err(format!(
            "gpu.run_spirv: a SPIR-V binary is a sequence of 4-byte words with a 5-word \
             header, got {} bytes",
            spirv.len()
        ));
    }
    // Copy into aligned words (Bytes gives no alignment guarantee; SPIR-V
    // words are little-endian on disk, and every supported target is LE).
    let words: Vec<u32> = spirv
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if words[0] != 0x0723_0203 {
        return Err(format!(
            "gpu.run_spirv: not a SPIR-V binary (magic 0x{:08x}, expected 0x07230203)",
            words[0]
        ));
    }

    let f = fns().map_err(|e| format!("gpu.run_spirv: {e}"))?;
    unsafe {
        let picked = pick_device(f).map_err(|e| format!("gpu.run_spirv: {e}"))?;
        let Some(create_program_with_il) = f.create_program_with_il else {
            return Err(
                "gpu.run_spirv: this OpenCL runtime predates IL ingestion (no \
                 clCreateProgramWithIL — OpenCL 2.1+ core)"
                    .to_string(),
            );
        };

        let mut run = Run::new(f);
        let mut err: ClInt = 0;

        // Context bound to the picked platform (required for correctness on
        // multi-platform systems: a device is only valid in a context whose
        // platform owns it).
        let props: [isize; 3] = [CL_CONTEXT_PLATFORM, picked.platform as isize, 0];
        run.context = (f.create_context)(
            props.as_ptr(),
            1,
            &picked.device,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut err,
        );
        if run.context.is_null() {
            return Err(format!("gpu.run_spirv: clCreateContext failed (CL error {err})"));
        }

        run.queue = if let Some(create) = f.create_queue_props {
            create(run.context, picked.device, std::ptr::null(), &mut err)
        } else {
            // resolve() guarantees one of the two exists.
            (f.create_queue_legacy.unwrap())(run.context, picked.device, 0, &mut err)
        };
        if run.queue.is_null() {
            return Err(format!(
                "gpu.run_spirv: command-queue creation failed (CL error {err})"
            ));
        }

        // The program, straight from the caller's words — same no-compiler
        // chain as the Vulkan path, but through the OpenCL profile.
        run.program = create_program_with_il(
            run.context,
            words.as_ptr() as *const c_void,
            words.len() * 4,
            &mut err,
        );
        if run.program.is_null() {
            return Err(format!(
                "gpu.run_spirv: clCreateProgramWithIL rejected the binary (CL error {err}; \
                 device IL support: \"{}\") — is the module in the OpenCL SPIR-V profile \
                 (Kernel execution model, Physical64/OpenCL memory model)? A Vulkan-profile \
                 blob is a different dialect; gpu.backend() is the branch point",
                picked.il
            ));
        }
        let r = (f.build_program)(
            run.program,
            1,
            &picked.device,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if r != CL_SUCCESS {
            // Fetch the build log — the only real diagnostic an OpenCL
            // implementation gives for a rejected module.
            let mut log_size: usize = 0;
            let mut log = String::new();
            if (f.get_program_build_info)(
                run.program,
                picked.device,
                CL_PROGRAM_BUILD_LOG,
                0,
                std::ptr::null_mut(),
                &mut log_size,
            ) == CL_SUCCESS
                && log_size > 1
            {
                let mut buf = vec![0u8; log_size];
                if (f.get_program_build_info)(
                    run.program,
                    picked.device,
                    CL_PROGRAM_BUILD_LOG,
                    log_size,
                    buf.as_mut_ptr() as *mut c_void,
                    std::ptr::null_mut(),
                ) == CL_SUCCESS
                {
                    while buf.last() == Some(&0) {
                        buf.pop();
                    }
                    log = String::from_utf8_lossy(&buf).trim().to_string();
                }
            }
            return Err(if log.is_empty() {
                format!("gpu.run_spirv: clBuildProgram failed (CL error {r})")
            } else {
                format!("gpu.run_spirv: clBuildProgram failed (CL error {r}): {log}")
            });
        }

        run.kernel = (f.create_kernel)(run.program, c"main".as_ptr(), &mut err);
        if run.kernel.is_null() {
            return Err(format!(
                "gpu.run_spirv: no kernel entry point named `main` (CL error {err}) — the \
                 ABI gpu.run_spirv fixes, see SPEC § 7.2"
            ));
        }

        // Buffers: input copied in at creation; output created from zeroed
        // host bytes (the zeroed-output contract every backend honors).
        run.inbuf = (f.create_buffer)(
            run.context,
            CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
            input.len(),
            input.as_ptr() as *mut c_void,
            &mut err,
        );
        if run.inbuf.is_null() {
            return Err(format!(
                "gpu.run_spirv: input buffer creation failed (CL error {err})"
            ));
        }
        let zeros = vec![0u8; out_len];
        run.outbuf = (f.create_buffer)(
            run.context,
            CL_MEM_READ_WRITE | CL_MEM_COPY_HOST_PTR,
            out_len,
            zeros.as_ptr() as *mut c_void,
            &mut err,
        );
        if run.outbuf.is_null() {
            return Err(format!(
                "gpu.run_spirv: output buffer creation failed (CL error {err})"
            ));
        }

        for (index, buf) in [(0, &run.inbuf), (1, &run.outbuf)] {
            let r = (f.set_kernel_arg)(
                run.kernel,
                index,
                std::mem::size_of::<ClMem>(),
                buf as *const ClMem as *const c_void,
            );
            if r != CL_SUCCESS {
                return Err(format!(
                    "gpu.run_spirv: clSetKernelArg({index}) failed (CL error {r}) — does the \
                     kernel take exactly two global pointer arguments (input, output)?"
                ));
            }
        }

        // `(wx, wy, wz)` is the *global work size* (total work-items), the
        // local size left to the implementation — the same index space a
        // Vulkan module with LocalSize 1 1 1 dispatches.
        let global: [usize; 3] = [wx as usize, wy as usize, wz as usize];
        let r = (f.enqueue_ndrange_kernel)(
            run.queue,
            run.kernel,
            3,
            std::ptr::null(),
            global.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        if r != CL_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: clEnqueueNDRangeKernel failed (CL error {r})"
            ));
        }
        let r = (f.finish)(run.queue);
        if r != CL_SUCCESS {
            return Err(format!("gpu.run_spirv: clFinish failed (CL error {r})"));
        }

        let mut out = vec![0u8; out_len];
        let r = (f.enqueue_read_buffer)(
            run.queue,
            run.outbuf,
            CL_TRUE,
            0,
            out_len,
            out.as_mut_ptr() as *mut c_void,
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        if r != CL_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: clEnqueueReadBuffer failed (CL error {r})"
            ));
        }
        Ok(out)
    }
}
