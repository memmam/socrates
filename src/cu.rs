//! Raw-FFI CUDA compute for the `gpu` namespace — the roadmap's fourth
//! native compute backend (after Metal, Vulkan, and OpenCL). CUDA's kernel
//! input is **PTX**, NVIDIA's textual virtual ISA, which the driver JITs
//! for the resident GPU at `cuModuleLoadData` time — so, like MSL on the
//! metal backend (and unlike the SPIR-V binaries the vulkan/opencl
//! backends take through `gpu.run_spirv`), PTX rides `gpu.run`'s `String`
//! argument. `gpu.backend()` is the dialect branch point, as everywhere.
//!
//! **Zero dependencies**: the driver library is resolved at runtime with
//! `dlopen("libcuda.so.1")` on Linux / `LoadLibraryA("nvcuda.dll")` on
//! Windows — it ships with NVIDIA's driver, not with any SDK, and no CUDA
//! toolkit is involved (the driver API loads PTX directly; nvcc never
//! enters the picture). Every entry point is a direct export resolved once
//! per process, the `cl.rs` shape. The `_v2` symbol variants are used
//! throughout: they are the real ABI of every driver since CUDA 3.2 (2010)
//! — the unsuffixed names are the frozen 32-bit-era compatibility shims.
//!
//! **Blind-dev honesty**: unlike lavapipe (Vulkan) and the Intel CPU
//! runtime (OpenCL), no software CUDA implementation exists — this backend
//! cannot execute anywhere without an NVIDIA GPU, which neither the dev
//! container nor any CI runner has. CI proves compilation, clippy, the
//! graceful no-driver paths, and zero-dependency-ness; the battery asset
//! (`docs/assets/cuda_compute.soc`) hard-asserts its bytes whenever real
//! hardware eventually runs it.
//!
//! **Per-call lifecycle, deliberately**: each `run` creates and destroys
//! its own context (`cuCtxCreate_v2` binds it to the calling thread, so
//! worker isolates are independent by construction), loads the module,
//! launches, and tears everything down through a `Drop` guard releasing in
//! reverse creation order — the `vk.rs`/`cl.rs` discipline exactly.

#![cfg_attr(
    not(all(
        feature = "cuda",
        not(feature = "vulkan"),
        not(all(feature = "d3d12", target_os = "windows")),
        any(target_os = "linux", target_os = "windows")
    )),
    allow(dead_code)
)]

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Loader resolution (dlopen / LoadLibrary), mirroring cl.rs's strategy.
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
        dlopen(c"libcuda.so.1".as_ptr(), RTLD_NOW)
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
        LoadLibraryA(c"nvcuda.dll".as_ptr())
    }
    pub(super) unsafe fn sym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        GetProcAddress(handle, name)
    }
}

// ---------------------------------------------------------------------------
// Handles, scalars, and constants (CUDA driver API, cuda.h widths).
// ---------------------------------------------------------------------------

type CuResult = i32;
/// `CUdevice` is a plain ordinal, not a pointer.
type CuDevice = i32;
type CuContext = *mut c_void;
type CuModule = *mut c_void;
type CuFunction = *mut c_void;
type CuStream = *mut c_void;
/// `CUdeviceptr` in the `_v2` ABI: an unsigned 64-bit device address on
/// every platform (the unsuffixed 32-bit legacy ABI is never used here).
type CuDeviceptr = u64;

const CUDA_SUCCESS: CuResult = 0;

// ---------------------------------------------------------------------------
// Function-pointer types and the once-per-process resolved table.
// ---------------------------------------------------------------------------

type FnInit = unsafe extern "system" fn(u32) -> CuResult;
type FnDeviceGetCount = unsafe extern "system" fn(*mut i32) -> CuResult;
type FnDeviceGet = unsafe extern "system" fn(*mut CuDevice, i32) -> CuResult;
type FnDeviceGetName = unsafe extern "system" fn(*mut c_char, i32, CuDevice) -> CuResult;
type FnCtxCreate = unsafe extern "system" fn(*mut CuContext, u32, CuDevice) -> CuResult;
type FnCtxDestroy = unsafe extern "system" fn(CuContext) -> CuResult;
type FnCtxSynchronize = unsafe extern "system" fn() -> CuResult;
type FnModuleLoadData = unsafe extern "system" fn(*mut CuModule, *const c_void) -> CuResult;
type FnModuleUnload = unsafe extern "system" fn(CuModule) -> CuResult;
type FnModuleGetFunction =
    unsafe extern "system" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult;
type FnMemAlloc = unsafe extern "system" fn(*mut CuDeviceptr, usize) -> CuResult;
type FnMemFree = unsafe extern "system" fn(CuDeviceptr) -> CuResult;
type FnMemcpyHtoD = unsafe extern "system" fn(CuDeviceptr, *const c_void, usize) -> CuResult;
type FnMemcpyDtoH = unsafe extern "system" fn(*mut c_void, CuDeviceptr, usize) -> CuResult;
type FnMemsetD8 = unsafe extern "system" fn(CuDeviceptr, u8, usize) -> CuResult;
#[allow(clippy::type_complexity)] // cuLaunchKernel's own 11-parameter shape
type FnLaunchKernel = unsafe extern "system" fn(
    CuFunction,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    CuStream,
    *mut *mut c_void,
    *mut *mut c_void,
) -> CuResult;
type FnGetErrorName = unsafe extern "system" fn(CuResult, *mut *const c_char) -> CuResult;

/// The resolved entry-point table (fn pointers are `Send + Sync`).
struct CuFns {
    init: FnInit,
    device_get_count: FnDeviceGetCount,
    device_get: FnDeviceGet,
    device_get_name: FnDeviceGetName,
    ctx_create: FnCtxCreate,
    ctx_destroy: FnCtxDestroy,
    ctx_synchronize: FnCtxSynchronize,
    module_load_data: FnModuleLoadData,
    module_unload: FnModuleUnload,
    module_get_function: FnModuleGetFunction,
    mem_alloc: FnMemAlloc,
    mem_free: FnMemFree,
    memcpy_htod: FnMemcpyHtoD,
    memcpy_dtoh: FnMemcpyDtoH,
    memset_d8: FnMemsetD8,
    launch_kernel: FnLaunchKernel,
    /// Error-naming pair (CUDA 6.0+, 2014). Optional defensively: a
    /// missing pair degrades messages to numeric codes, not a failure.
    get_error_name: Option<FnGetErrorName>,
    get_error_string: Option<FnGetErrorName>,
}

/// Resolve the driver's exports once per process. `Err` carries what's
/// missing so every entry point can report a diagnosable message.
fn fns() -> Result<&'static CuFns, String> {
    static CELL: OnceLock<Result<CuFns, String>> = OnceLock::new();
    CELL.get_or_init(|| unsafe { resolve() })
        .as_ref()
        .map_err(|e| e.clone())
}

unsafe fn resolve() -> Result<CuFns, String> {
    let lib = libloading::open();
    if lib.is_null() {
        return Err("no CUDA driver (libcuda) on this system".to_string());
    }
    macro_rules! req {
        ($name:literal, $ty:ty) => {{
            let p = libloading::sym(lib, concat!($name, "\0").as_ptr() as *const c_char);
            if p.is_null() {
                return Err(format!("CUDA driver has no {}", $name));
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
    Ok(CuFns {
        init: req!("cuInit", FnInit),
        device_get_count: req!("cuDeviceGetCount", FnDeviceGetCount),
        device_get: req!("cuDeviceGet", FnDeviceGet),
        device_get_name: req!("cuDeviceGetName", FnDeviceGetName),
        ctx_create: req!("cuCtxCreate_v2", FnCtxCreate),
        ctx_destroy: req!("cuCtxDestroy_v2", FnCtxDestroy),
        ctx_synchronize: req!("cuCtxSynchronize", FnCtxSynchronize),
        module_load_data: req!("cuModuleLoadData", FnModuleLoadData),
        module_unload: req!("cuModuleUnload", FnModuleUnload),
        module_get_function: req!("cuModuleGetFunction", FnModuleGetFunction),
        mem_alloc: req!("cuMemAlloc_v2", FnMemAlloc),
        mem_free: req!("cuMemFree_v2", FnMemFree),
        memcpy_htod: req!("cuMemcpyHtoD_v2", FnMemcpyHtoD),
        memcpy_dtoh: req!("cuMemcpyDtoH_v2", FnMemcpyDtoH),
        memset_d8: req!("cuMemsetD8_v2", FnMemsetD8),
        launch_kernel: req!("cuLaunchKernel", FnLaunchKernel),
        get_error_name: opt!("cuGetErrorName", FnGetErrorName),
        get_error_string: opt!("cuGetErrorString", FnGetErrorName),
    })
}

/// `"cuLaunchKernel failed (CUDA_ERROR_INVALID_VALUE: invalid argument)"`
/// -shaped detail, degrading to the numeric code when the driver can't
/// name it.
fn describe(f: &CuFns, what: &str, code: CuResult) -> String {
    unsafe {
        let name = f.get_error_name.and_then(|get| {
            let mut p: *const c_char = std::ptr::null();
            if get(code, &mut p) == CUDA_SUCCESS && !p.is_null() {
                Some(CStr::from_ptr(p).to_string_lossy().into_owned())
            } else {
                None
            }
        });
        let desc = f.get_error_string.and_then(|get| {
            let mut p: *const c_char = std::ptr::null();
            if get(code, &mut p) == CUDA_SUCCESS && !p.is_null() {
                Some(CStr::from_ptr(p).to_string_lossy().into_owned())
            } else {
                None
            }
        });
        match (name, desc) {
            (Some(n), Some(d)) => format!("gpu.run: {what} failed ({n}: {d})"),
            (Some(n), None) => format!("gpu.run: {what} failed ({n})"),
            _ => format!("gpu.run: {what} failed (CUresult {code})"),
        }
    }
}

/// `cuInit` + first-device lookup, shared by every entry point. `cuInit`
/// is idempotent and cheap after the first call.
unsafe fn first_device(f: &CuFns) -> Result<CuDevice, String> {
    let r = (f.init)(0);
    if r != CUDA_SUCCESS {
        return Err(describe(f, "cuInit", r));
    }
    let mut count: i32 = 0;
    let r = (f.device_get_count)(&mut count);
    if r != CUDA_SUCCESS {
        return Err(describe(f, "cuDeviceGetCount", r));
    }
    if count == 0 {
        return Err("gpu.run: no CUDA devices".to_string());
    }
    let mut dev: CuDevice = 0;
    let r = (f.device_get)(&mut dev, 0);
    if r != CUDA_SUCCESS {
        return Err(describe(f, "cuDeviceGet", r));
    }
    Ok(dev)
}

/// Is a CUDA device reachable? (Driver present, `cuInit` succeeds, at
/// least one device.)
pub(crate) fn available() -> bool {
    let Ok(f) = fns() else {
        return false;
    };
    unsafe { first_device(f).is_ok() }
}

/// `"<device name> (cuda)"` — the `"<name> (<backend>)"` shape every gpu
/// backend reports, or `"no adapter"`.
pub(crate) fn adapter_info() -> String {
    let Ok(f) = fns() else {
        return "no adapter".to_string();
    };
    unsafe {
        match first_device(f) {
            Ok(dev) => {
                let mut buf = [0 as c_char; 256];
                if (f.device_get_name)(buf.as_mut_ptr(), buf.len() as i32, dev) != CUDA_SUCCESS {
                    return "no adapter".to_string();
                }
                let name = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                format!("{name} (cuda)")
            }
            Err(_) => "no adapter".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// The Drop guard and the dispatch itself.
// ---------------------------------------------------------------------------

/// Everything `run` creates, released in reverse creation order by `Drop`.
/// The context is destroyed last (device pointers and the module belong to
/// it), and `Drop` runs on the creating thread, where the context is
/// current — the requirement `cuMemFree`/`cuModuleUnload` have.
struct Run {
    f: &'static CuFns,
    ctx: CuContext,
    module: CuModule,
    inbuf: CuDeviceptr,
    outbuf: CuDeviceptr,
}

impl Run {
    fn new(f: &'static CuFns) -> Run {
        Run {
            f,
            ctx: std::ptr::null_mut(),
            module: std::ptr::null_mut(),
            inbuf: 0,
            outbuf: 0,
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        unsafe {
            if self.inbuf != 0 {
                (self.f.mem_free)(self.inbuf);
            }
            if self.outbuf != 0 {
                (self.f.mem_free)(self.outbuf);
            }
            if !self.module.is_null() {
                (self.f.module_unload)(self.module);
            }
            if !self.ctx.is_null() {
                (self.f.ctx_destroy)(self.ctx);
            }
        }
    }
}

/// One dispatch of a PTX kernel — see `gpu.rs`'s module docs and SPEC
/// § 7.2 for the ABI (entry `main` taking two global pointer parameters;
/// the grid is `(wx, wy, wz)` blocks of one thread each, so `%ctaid`
/// spans the index space; the caller has already validated sizes/counts
/// via `gpu::validate`).
pub(crate) fn run(
    ptx: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    // cuModuleLoadData takes a NUL-terminated image for PTX text.
    if ptx.as_bytes().contains(&0) {
        return Err("gpu.run: PTX source contains a NUL byte".to_string());
    }
    let mut image = Vec::with_capacity(ptx.len() + 1);
    image.extend_from_slice(ptx.as_bytes());
    image.push(0);

    let f = fns().map_err(|e| format!("gpu.run: {e}"))?;
    unsafe {
        let dev = first_device(f)?;
        let mut run = Run::new(f);

        let r = (f.ctx_create)(&mut run.ctx, 0, dev);
        if r != CUDA_SUCCESS || run.ctx.is_null() {
            return Err(describe(f, "cuCtxCreate", r));
        }

        let r = (f.module_load_data)(&mut run.module, image.as_ptr() as *const c_void);
        if r != CUDA_SUCCESS || run.module.is_null() {
            return Err(format!(
                "{} — is the source PTX (gpu.backend() == \"cuda\" takes NVIDIA's textual \
                 virtual ISA through gpu.run, not GLSL/MSL/SPIR-V)?",
                describe(f, "cuModuleLoadData", r)
            ));
        }

        let mut func: CuFunction = std::ptr::null_mut();
        let r = (f.module_get_function)(&mut func, run.module, c"main".as_ptr());
        if r != CUDA_SUCCESS || func.is_null() {
            return Err(format!(
                "{} — the PTX must declare `.visible .entry main` (the ABI gpu.run fixes, \
                 see SPEC § 7.2)",
                describe(f, "cuModuleGetFunction(\"main\")", r)
            ));
        }

        let r = (f.mem_alloc)(&mut run.inbuf, input.len());
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuMemAlloc(input)", r));
        }
        let r = (f.memcpy_htod)(run.inbuf, input.as_ptr() as *const c_void, input.len());
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuMemcpyHtoD", r));
        }
        let r = (f.mem_alloc)(&mut run.outbuf, out_len);
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuMemAlloc(output)", r));
        }
        // The zeroed-output contract every backend honors.
        let r = (f.memset_d8)(run.outbuf, 0, out_len);
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuMemsetD8", r));
        }

        // kernelParams: an array of pointers, each to one argument value.
        let mut params: [*mut c_void; 2] = [
            &mut run.inbuf as *mut CuDeviceptr as *mut c_void,
            &mut run.outbuf as *mut CuDeviceptr as *mut c_void,
        ];
        let r = (f.launch_kernel)(
            func,
            wx,
            wy,
            wz,
            1,
            1,
            1,
            0,
            std::ptr::null_mut(),
            params.as_mut_ptr(),
            std::ptr::null_mut(),
        );
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuLaunchKernel", r));
        }
        // The launch is asynchronous; execution errors surface here.
        let r = (f.ctx_synchronize)();
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuCtxSynchronize", r));
        }

        let mut out = vec![0u8; out_len];
        let r = (f.memcpy_dtoh)(out.as_mut_ptr() as *mut c_void, run.outbuf, out_len);
        if r != CUDA_SUCCESS {
            return Err(describe(f, "cuMemcpyDtoH", r));
        }
        Ok(out)
    }
}
