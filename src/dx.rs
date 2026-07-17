//! Raw-FFI Direct3D 12 compute for the `gpu` namespace — the roadmap's
//! fifth native compute backend, and the one that brings Windows its
//! always-available device: WARP, Microsoft's software D3D12 adapter,
//! ships with the OS, so (like lavapipe for Vulkan and the Intel CPU
//! runtime for OpenCL) the battery hard-asserts real dispatched bytes on
//! plain CI runners with no GPU.
//!
//! **Kernel input is HLSL source through `gpu.run`** (the MSL/PTX
//! pattern): `d3dcompiler_47.dll` ships in Windows System32, so
//! `D3DCompile` (target `cs_5_0`, entry `main`) is the OS's own compiler
//! — no SDK, no toolkit, no dependency. The DXBC blob it produces feeds
//! `CreateComputePipelineState` directly.
//!
//! **Zero dependencies**: `d3d12.dll`, `dxgi.dll`, and
//! `d3dcompiler_47.dll` are all `LoadLibraryA`'d at runtime and every
//! export resolved with `GetProcAddress` — the `cl.rs`/`cu.rs` shape. COM
//! interfaces are called through hand-transcribed vtable indices
//! (`extern "system"` fn pointers at fixed slots), with a tiny `Com`
//! guard giving every interface pointer scope-tied `IUnknown::Release`.
//!
//! **Design choices that keep this small**:
//! - a DIRECT command queue/list (supports both copies and dispatch);
//! - **no descriptor heaps**: the root signature is two *root UAV*
//!   parameters (`u0` input, `u1` output), bound by GPU virtual address
//!   with `SetComputeRootUnorderedAccessView`;
//! - one UPLOAD-heap staging buffer holding `[input bytes | out_len
//!   zeros]`, copied into the two DEFAULT-heap UAV buffers (D3D12 does
//!   not guarantee zeroed resources — the zeroed-output contract is made
//!   true by copying zeros);
//! - dispatch is `(wx, wy, wz)` thread groups of whatever the HLSL's
//!   `[numthreads]` declares — the battery uses `(1,1,1)` with
//!   `SV_GroupID`, spanning the same index space as every other backend;
//! - synchronization is one fence + a Win32 event, then a READBACK-heap
//!   copy is mapped and read.
//!
//! Device acquisition tries the default adapter first, then falls back to
//! `IDXGIFactory4::EnumWarpAdapter` explicitly — headless CI machines
//! often have no default adapter, and the deterministic WARP path is
//! exactly what makes this backend CI-provable.

#![cfg_attr(
    not(all(
        feature = "d3d12",
        not(feature = "vulkan"),
        target_os = "windows"
    )),
    allow(dead_code)
)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Library resolution (LoadLibrary/GetProcAddress; Windows-only module).
// ---------------------------------------------------------------------------

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(name: *const c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn CreateEventA(
        attrs: *mut c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const c_char,
    ) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

type Hresult = i32;
const S_OK: Hresult = 0;
/// 16-byte COM interface id.
#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_IDXGI_FACTORY4: Guid = Guid {
    data1: 0x1bc6_ea02,
    data2: 0xef36,
    data3: 0x464f,
    data4: [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a],
};
const IID_IDXGI_ADAPTER: Guid = Guid {
    data1: 0x2411_e7e1,
    data2: 0x12ac,
    data3: 0x4ccf,
    data4: [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0],
};
const IID_ID3D12_DEVICE: Guid = Guid {
    data1: 0x189c_f0aa,
    data2: 0xaf88,
    data3: 0x4e2c,
    data4: [0xb1, 0x0d, 0xa8, 0xff, 0xfa, 0x06, 0x8f, 0x27],
};
const IID_ID3D12_COMMAND_QUEUE: Guid = Guid {
    data1: 0x0ec8_70a6,
    data2: 0x5d7e,
    data3: 0x4c22,
    data4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed],
};
const IID_ID3D12_COMMAND_ALLOCATOR: Guid = Guid {
    data1: 0x6102_dee4,
    data2: 0xaf59,
    data3: 0x4b09,
    data4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24],
};
const IID_ID3D12_GRAPHICS_COMMAND_LIST: Guid = Guid {
    data1: 0x5b16_0d0f,
    data2: 0xac1b,
    data3: 0x4185,
    data4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55],
};
const IID_ID3D12_ROOT_SIGNATURE: Guid = Guid {
    data1: 0xc54a_6b66,
    data2: 0x72df,
    data3: 0x4ee8,
    data4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14],
};
const IID_ID3D12_PIPELINE_STATE: Guid = Guid {
    data1: 0x765a_30f3,
    data2: 0xf624,
    data3: 0x4c6f,
    data4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
};
const IID_ID3D12_RESOURCE: Guid = Guid {
    data1: 0x6964_42be,
    data2: 0xa9f8,
    data3: 0x4fd8,
    data4: [0xaa, 0x8c, 0xbe, 0xc4, 0xc4, 0xf8, 0x06, 0xda],
};
const IID_ID3D12_FENCE: Guid = Guid {
    data1: 0x0a75_3dcf,
    data2: 0xc4d8,
    data3: 0x4b91,
    data4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76],
};

// ---------------------------------------------------------------------------
// COM plumbing: vtable call-by-index + a Release-on-drop guard.
// ---------------------------------------------------------------------------

/// Fetch slot `$idx` of `$obj`'s vtable as a fn pointer of type `$ty` and
/// call it with `$obj` as `this`. Safety rests on the transcribed vtable
/// layouts below matching d3d12.h/dxgi.h exactly.
macro_rules! com_call {
    ($obj:expr, $idx:expr, $ty:ty, $($arg:expr),* $(,)?) => {{
        let this = $obj;
        let vtbl = *(this as *const *const *const c_void);
        let f = std::mem::transmute::<*const c_void, $ty>(*vtbl.add($idx));
        f(this, $($arg),*)
    }};
}

/// A non-null COM interface pointer released on drop (`IUnknown::Release`
/// is always vtable slot 2).
struct Com(*mut c_void);
impl Com {
    fn ptr(&self) -> *mut c_void {
        self.0
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;
            let _ = com_call!(self.0, 2, FnRelease,);
        }
    }
}

/// A Win32 event handle closed on drop.
struct Event(*mut c_void);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Structs (d3d12.h, exact MSVC x64 layout).
// ---------------------------------------------------------------------------

const D3D12_COMMAND_LIST_TYPE_DIRECT: u32 = 0;
const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
const D3D_ROOT_SIGNATURE_VERSION_1: u32 = 1;
const D3D12_ROOT_PARAMETER_TYPE_UAV: u32 = 4;
const D3D12_SHADER_VISIBILITY_ALL: u32 = 0;
const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;
const D3D12_HEAP_TYPE_UPLOAD: u32 = 2;
const D3D12_HEAP_TYPE_READBACK: u32 = 3;
const D3D12_RESOURCE_DIMENSION_BUFFER: u32 = 1;
const D3D12_TEXTURE_LAYOUT_ROW_MAJOR: u32 = 1;
const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;
const D3D12_RESOURCE_STATE_UNORDERED_ACCESS: u32 = 0x8;
const D3D12_RESOURCE_STATE_COPY_DEST: u32 = 0x400;
const D3D12_RESOURCE_STATE_COPY_SOURCE: u32 = 0x800;
const D3D12_RESOURCE_STATE_GENERIC_READ: u32 = 0xAC3;
const D3D12_RESOURCE_BARRIER_TYPE_TRANSITION: u32 = 0;
const D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES: u32 = 0xffff_ffff;
/// `D3DCOMPILE_OPTIMIZATION_LEVEL3`.
const D3DCOMPILE_OPT3: u32 = 1 << 15;
const INFINITE: u32 = 0xffff_ffff;

#[repr(C)]
struct CommandQueueDesc {
    ty: u32,
    priority: i32,
    flags: u32,
    node_mask: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct RootDescriptor {
    shader_register: u32,
    register_space: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct RootDescriptorTable {
    num_ranges: u32,
    p_ranges: *const c_void,
}
/// The anonymous union in `D3D12_ROOT_PARAMETER`; the descriptor-table
/// member is the largest (16 bytes on x64), which sizes the union.
#[repr(C)]
union RootParameterU {
    descriptor: RootDescriptor,
    table: RootDescriptorTable,
}
#[repr(C)]
struct RootParameter {
    parameter_type: u32,
    u: RootParameterU,
    shader_visibility: u32,
}
#[repr(C)]
struct RootSignatureDesc {
    num_parameters: u32,
    p_parameters: *const RootParameter,
    num_static_samplers: u32,
    p_static_samplers: *const c_void,
    flags: u32,
}
#[repr(C)]
struct ShaderBytecode {
    p_bytecode: *const c_void,
    length: usize,
}
#[repr(C)]
struct CachedPipelineState {
    p_blob: *const c_void,
    length: usize,
}
#[repr(C)]
struct ComputePipelineStateDesc {
    p_root_signature: *mut c_void,
    cs: ShaderBytecode,
    node_mask: u32,
    cached_pso: CachedPipelineState,
    flags: u32,
}
#[repr(C)]
struct HeapProperties {
    ty: u32,
    cpu_page_property: u32,
    memory_pool_preference: u32,
    creation_node_mask: u32,
    visible_node_mask: u32,
}
#[repr(C)]
struct SampleDesc {
    count: u32,
    quality: u32,
}
#[repr(C)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth_or_array_size: u16,
    mip_levels: u16,
    format: u32,
    sample_desc: SampleDesc,
    layout: u32,
    flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceTransitionBarrier {
    p_resource: *mut c_void,
    subresource: u32,
    state_before: u32,
    state_after: u32,
}
#[repr(C)]
union ResourceBarrierU {
    transition: ResourceTransitionBarrier,
}
#[repr(C)]
struct ResourceBarrier {
    ty: u32,
    flags: u32,
    u: ResourceBarrierU,
}
#[repr(C)]
struct Range {
    begin: usize,
    end: usize,
}

// ---------------------------------------------------------------------------
// Exported entry points (resolved once per process) and vtable fn types.
// ---------------------------------------------------------------------------

type FnD3D12CreateDevice =
    unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> Hresult;
type FnSerializeRootSignature = unsafe extern "system" fn(
    *const RootSignatureDesc,
    u32,
    *mut *mut c_void,
    *mut *mut c_void,
) -> Hresult;
type FnCreateDxgiFactory1 =
    unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> Hresult;
#[allow(clippy::type_complexity)] // D3DCompile's own 11-parameter shape
type FnD3DCompile = unsafe extern "system" fn(
    *const c_void,
    usize,
    *const c_char,
    *const c_void,
    *const c_void,
    *const c_char,
    *const c_char,
    u32,
    u32,
    *mut *mut c_void,
    *mut *mut c_void,
) -> Hresult;

struct DxFns {
    create_device: FnD3D12CreateDevice,
    serialize_root_signature: FnSerializeRootSignature,
    create_dxgi_factory1: FnCreateDxgiFactory1,
    compile: FnD3DCompile,
}

fn fns() -> Result<&'static DxFns, String> {
    static CELL: OnceLock<Result<DxFns, String>> = OnceLock::new();
    CELL.get_or_init(|| unsafe { resolve() })
        .as_ref()
        .map_err(|e| e.clone())
}

unsafe fn resolve() -> Result<DxFns, String> {
    let d3d12 = LoadLibraryA(c"d3d12.dll".as_ptr());
    if d3d12.is_null() {
        return Err("no d3d12.dll on this system".to_string());
    }
    let dxgi = LoadLibraryA(c"dxgi.dll".as_ptr());
    if dxgi.is_null() {
        return Err("no dxgi.dll on this system".to_string());
    }
    let compiler = LoadLibraryA(c"d3dcompiler_47.dll".as_ptr());
    if compiler.is_null() {
        return Err("no d3dcompiler_47.dll on this system".to_string());
    }
    macro_rules! req {
        ($lib:expr, $name:literal, $ty:ty) => {{
            let p = GetProcAddress($lib, concat!($name, "\0").as_ptr() as *const c_char);
            if p.is_null() {
                return Err(format!("system library has no {}", $name));
            }
            std::mem::transmute::<*mut c_void, $ty>(p)
        }};
    }
    Ok(DxFns {
        create_device: req!(d3d12, "D3D12CreateDevice", FnD3D12CreateDevice),
        serialize_root_signature: req!(
            d3d12,
            "D3D12SerializeRootSignature",
            FnSerializeRootSignature
        ),
        create_dxgi_factory1: req!(dxgi, "CreateDXGIFactory1", FnCreateDxgiFactory1),
        compile: req!(compiler, "D3DCompile", FnD3DCompile),
    })
}

// Vtable slot indices (transcribed from d3d12.h / dxgi.h; IUnknown is
// always 0..=2).
const VT_FACTORY4_ENUM_WARP_ADAPTER: usize = 27;
const VT_DEVICE_CREATE_COMMAND_QUEUE: usize = 8;
const VT_DEVICE_CREATE_COMMAND_ALLOCATOR: usize = 9;
const VT_DEVICE_CREATE_COMPUTE_PIPELINE_STATE: usize = 11;
const VT_DEVICE_CREATE_COMMAND_LIST: usize = 12;
const VT_DEVICE_CREATE_ROOT_SIGNATURE: usize = 16;
const VT_DEVICE_CREATE_COMMITTED_RESOURCE: usize = 27;
const VT_DEVICE_CREATE_FENCE: usize = 36;
const VT_QUEUE_EXECUTE_COMMAND_LISTS: usize = 10;
const VT_QUEUE_SIGNAL: usize = 14;
const VT_LIST_CLOSE: usize = 9;
const VT_LIST_DISPATCH: usize = 14;
const VT_LIST_COPY_BUFFER_REGION: usize = 15;
const VT_LIST_SET_PIPELINE_STATE: usize = 25;
const VT_LIST_RESOURCE_BARRIER: usize = 26;
const VT_LIST_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const VT_LIST_SET_COMPUTE_ROOT_UAV: usize = 41;
const VT_RESOURCE_MAP: usize = 8;
const VT_RESOURCE_UNMAP: usize = 9;
const VT_RESOURCE_GET_GPU_VIRTUAL_ADDRESS: usize = 11;
const VT_FENCE_SET_EVENT_ON_COMPLETION: usize = 9;
const VT_BLOB_GET_BUFFER_POINTER: usize = 3;
const VT_BLOB_GET_BUFFER_SIZE: usize = 4;

type FnCreateWithDesc =
    unsafe extern "system" fn(*mut c_void, *const c_void, *const Guid, *mut *mut c_void) -> Hresult;
type FnEnumWarpAdapter =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;
type FnCreateCommandAllocator =
    unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> Hresult;
type FnCreateCommandList = unsafe extern "system" fn(
    *mut c_void,
    u32,
    u32,
    *mut c_void,
    *mut c_void,
    *const Guid,
    *mut *mut c_void,
) -> Hresult;
type FnCreateRootSignature = unsafe extern "system" fn(
    *mut c_void,
    u32,
    *const c_void,
    usize,
    *const Guid,
    *mut *mut c_void,
) -> Hresult;
type FnCreateCommittedResource = unsafe extern "system" fn(
    *mut c_void,
    *const HeapProperties,
    u32,
    *const ResourceDesc,
    u32,
    *const c_void,
    *const Guid,
    *mut *mut c_void,
) -> Hresult;
type FnCreateFence =
    unsafe extern "system" fn(*mut c_void, u64, u32, *const Guid, *mut *mut c_void) -> Hresult;
type FnExecuteCommandLists = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);
type FnQueueSignal = unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> Hresult;
type FnListClose = unsafe extern "system" fn(*mut c_void) -> Hresult;
type FnListDispatch = unsafe extern "system" fn(*mut c_void, u32, u32, u32);
type FnListCopyBufferRegion =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64);
type FnListSetPtr = unsafe extern "system" fn(*mut c_void, *mut c_void);
type FnListResourceBarrier = unsafe extern "system" fn(*mut c_void, u32, *const ResourceBarrier);
type FnListSetComputeRootUav = unsafe extern "system" fn(*mut c_void, u32, u64);
type FnResourceMap =
    unsafe extern "system" fn(*mut c_void, u32, *const Range, *mut *mut c_void) -> Hresult;
type FnResourceUnmap = unsafe extern "system" fn(*mut c_void, u32, *const Range);
type FnResourceGetGpuVa = unsafe extern "system" fn(*mut c_void) -> u64;
type FnFenceSetEventOnCompletion =
    unsafe extern "system" fn(*mut c_void, u64, *mut c_void) -> Hresult;
type FnBlobGetPointer = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
type FnBlobGetSize = unsafe extern "system" fn(*mut c_void) -> usize;

// ---------------------------------------------------------------------------
// Device acquisition + the dispatch.
// ---------------------------------------------------------------------------

/// Create a D3D12 device: the default adapter if one works, else WARP via
/// `IDXGIFactory4::EnumWarpAdapter` — the path that always exists on a
/// headless runner.
unsafe fn create_device(f: &DxFns) -> Result<Com, String> {
    let mut device: *mut c_void = std::ptr::null_mut();
    let hr = (f.create_device)(
        std::ptr::null_mut(),
        D3D_FEATURE_LEVEL_11_0,
        &IID_ID3D12_DEVICE,
        &mut device,
    );
    if hr == S_OK && !device.is_null() {
        return Ok(Com(device));
    }
    let mut factory: *mut c_void = std::ptr::null_mut();
    let hr = (f.create_dxgi_factory1)(&IID_IDXGI_FACTORY4, &mut factory);
    if hr != S_OK || factory.is_null() {
        return Err(format!("gpu.run: CreateDXGIFactory1 failed (HRESULT 0x{hr:08x})"));
    }
    let factory = Com(factory);
    let mut adapter: *mut c_void = std::ptr::null_mut();
    let hr = com_call!(
        factory.ptr(),
        VT_FACTORY4_ENUM_WARP_ADAPTER,
        FnEnumWarpAdapter,
        &IID_IDXGI_ADAPTER,
        &mut adapter
    );
    if hr != S_OK || adapter.is_null() {
        return Err(format!("gpu.run: EnumWarpAdapter failed (HRESULT 0x{hr:08x})"));
    }
    let adapter = Com(adapter);
    let mut device: *mut c_void = std::ptr::null_mut();
    let hr = (f.create_device)(
        adapter.ptr(),
        D3D_FEATURE_LEVEL_11_0,
        &IID_ID3D12_DEVICE,
        &mut device,
    );
    if hr != S_OK || device.is_null() {
        return Err(format!(
            "gpu.run: D3D12CreateDevice(WARP) failed (HRESULT 0x{hr:08x})"
        ));
    }
    Ok(Com(device))
}

/// Is a D3D12 device creatable? (WARP makes this effectively always true
/// on Windows 10+.)
pub(crate) fn available() -> bool {
    let Ok(f) = fns() else {
        return false;
    };
    unsafe { create_device(f).is_ok() }
}

/// `"d3d12 (default or WARP adapter)"` — adapter naming needs
/// `IDXGIAdapter::GetDesc`'s by-value struct return, deliberately avoided;
/// the backend name is the load-bearing part of the shape.
pub(crate) fn adapter_info() -> String {
    if available() {
        "Direct3D 12 device (d3d12)".to_string()
    } else {
        "no adapter".to_string()
    }
}

/// Read an `ID3DBlob`'s bytes.
unsafe fn blob_bytes(blob: *mut c_void) -> Vec<u8> {
    let p = com_call!(blob, VT_BLOB_GET_BUFFER_POINTER, FnBlobGetPointer,) as *const u8;
    let n = com_call!(blob, VT_BLOB_GET_BUFFER_SIZE, FnBlobGetSize,);
    std::slice::from_raw_parts(p, n).to_vec()
}

/// Create one committed buffer resource.
unsafe fn create_buffer(
    device: *mut c_void,
    heap_type: u32,
    size: u64,
    flags: u32,
    initial_state: u32,
    what: &str,
) -> Result<Com, String> {
    let heap = HeapProperties {
        ty: heap_type,
        cpu_page_property: 0,
        memory_pool_preference: 0,
        creation_node_mask: 1,
        visible_node_mask: 1,
    };
    let desc = ResourceDesc {
        dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        alignment: 0,
        width: size,
        height: 1,
        depth_or_array_size: 1,
        mip_levels: 1,
        format: 0,
        sample_desc: SampleDesc { count: 1, quality: 0 },
        layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        flags,
    };
    let mut res: *mut c_void = std::ptr::null_mut();
    let hr = com_call!(
        device,
        VT_DEVICE_CREATE_COMMITTED_RESOURCE,
        FnCreateCommittedResource,
        &heap,
        0,
        &desc,
        initial_state,
        std::ptr::null(),
        &IID_ID3D12_RESOURCE,
        &mut res
    );
    if hr != S_OK || res.is_null() {
        return Err(format!(
            "gpu.run: CreateCommittedResource({what}) failed (HRESULT 0x{hr:08x})"
        ));
    }
    Ok(Com(res))
}

/// Record a transition barrier.
unsafe fn barrier(list: *mut c_void, resource: *mut c_void, before: u32, after: u32) {
    let b = ResourceBarrier {
        ty: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        flags: 0,
        u: ResourceBarrierU {
            transition: ResourceTransitionBarrier {
                p_resource: resource,
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: before,
                state_after: after,
            },
        },
    };
    com_call!(list, VT_LIST_RESOURCE_BARRIER, FnListResourceBarrier, 1, &b);
}

/// One dispatch of an HLSL compute kernel — see `gpu.rs`'s module docs and
/// SPEC § 7.2 for the ABI (entry `main`, `RWByteAddressBuffer`s at `u0`
/// input / `u1` output, `(wx, wy, wz)` thread groups; the caller has
/// already validated sizes/counts via `gpu::validate`).
pub(crate) fn run(
    hlsl: &str,
    input: &[u8],
    out_len: usize,
    wx: u32,
    wy: u32,
    wz: u32,
) -> Result<Vec<u8>, String> {
    let f = fns().map_err(|e| format!("gpu.run: {e}"))?;
    unsafe {
        let device = create_device(f)?;
        let dev = device.ptr();

        // HLSL -> DXBC via the OS's own compiler; its error blob is the
        // real diagnostic (the OpenCL build-log analog).
        let mut code: *mut c_void = std::ptr::null_mut();
        let mut errors: *mut c_void = std::ptr::null_mut();
        let hr = (f.compile)(
            hlsl.as_ptr() as *const c_void,
            hlsl.len(),
            c"gpu.run".as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            c"main".as_ptr(),
            c"cs_5_0".as_ptr(),
            D3DCOMPILE_OPT3,
            0,
            &mut code,
            &mut errors,
        );
        let error_text = if errors.is_null() {
            String::new()
        } else {
            let errors = Com(errors);
            String::from_utf8_lossy(&blob_bytes(errors.ptr()))
                .trim_end_matches(['\0', '\n'])
                .to_string()
        };
        if hr != S_OK || code.is_null() {
            return Err(if error_text.is_empty() {
                format!("gpu.run: D3DCompile failed (HRESULT 0x{hr:08x})")
            } else {
                format!("gpu.run: D3DCompile failed (HRESULT 0x{hr:08x}): {error_text}")
            });
        }
        let code = Com(code);
        let dxbc = blob_bytes(code.ptr());

        // Root signature: two root UAVs (u0 input, u1 output) — no
        // descriptor heaps anywhere.
        let params = [
            RootParameter {
                parameter_type: D3D12_ROOT_PARAMETER_TYPE_UAV,
                u: RootParameterU {
                    descriptor: RootDescriptor { shader_register: 0, register_space: 0 },
                },
                shader_visibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            RootParameter {
                parameter_type: D3D12_ROOT_PARAMETER_TYPE_UAV,
                u: RootParameterU {
                    descriptor: RootDescriptor { shader_register: 1, register_space: 0 },
                },
                shader_visibility: D3D12_SHADER_VISIBILITY_ALL,
            },
        ];
        let rs_desc = RootSignatureDesc {
            num_parameters: 2,
            p_parameters: params.as_ptr(),
            num_static_samplers: 0,
            p_static_samplers: std::ptr::null(),
            flags: 0,
        };
        let mut rs_blob: *mut c_void = std::ptr::null_mut();
        let mut rs_err: *mut c_void = std::ptr::null_mut();
        let hr =
            (f.serialize_root_signature)(&rs_desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut rs_blob, &mut rs_err);
        if !rs_err.is_null() {
            drop(Com(rs_err));
        }
        if hr != S_OK || rs_blob.is_null() {
            return Err(format!(
                "gpu.run: D3D12SerializeRootSignature failed (HRESULT 0x{hr:08x})"
            ));
        }
        let rs_blob = Com(rs_blob);
        let rs_bytes = blob_bytes(rs_blob.ptr());
        let mut root_sig: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_ROOT_SIGNATURE,
            FnCreateRootSignature,
            0,
            rs_bytes.as_ptr() as *const c_void,
            rs_bytes.len(),
            &IID_ID3D12_ROOT_SIGNATURE,
            &mut root_sig
        );
        if hr != S_OK || root_sig.is_null() {
            return Err(format!(
                "gpu.run: CreateRootSignature failed (HRESULT 0x{hr:08x})"
            ));
        }
        let root_sig = Com(root_sig);

        let pso_desc = ComputePipelineStateDesc {
            p_root_signature: root_sig.ptr(),
            cs: ShaderBytecode { p_bytecode: dxbc.as_ptr() as *const c_void, length: dxbc.len() },
            node_mask: 0,
            cached_pso: CachedPipelineState { p_blob: std::ptr::null(), length: 0 },
            flags: 0,
        };
        let mut pso: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_COMPUTE_PIPELINE_STATE,
            FnCreateWithDesc,
            &pso_desc as *const ComputePipelineStateDesc as *const c_void,
            &IID_ID3D12_PIPELINE_STATE,
            &mut pso
        );
        if hr != S_OK || pso.is_null() {
            return Err(format!(
                "gpu.run: CreateComputePipelineState failed (HRESULT 0x{hr:08x}) — does the \
                 HLSL declare `void main` with RWByteAddressBuffers at u0/u1 (the ABI \
                 gpu.run fixes on d3d12, see SPEC § 7.2)?"
            ));
        }
        let pso = Com(pso);

        // Queue / allocator / list.
        let queue_desc = CommandQueueDesc {
            ty: D3D12_COMMAND_LIST_TYPE_DIRECT,
            priority: 0,
            flags: 0,
            node_mask: 0,
        };
        let mut queue: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_COMMAND_QUEUE,
            FnCreateWithDesc,
            &queue_desc as *const CommandQueueDesc as *const c_void,
            &IID_ID3D12_COMMAND_QUEUE,
            &mut queue
        );
        if hr != S_OK || queue.is_null() {
            return Err(format!("gpu.run: CreateCommandQueue failed (HRESULT 0x{hr:08x})"));
        }
        let queue = Com(queue);
        let mut alloc: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_COMMAND_ALLOCATOR,
            FnCreateCommandAllocator,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            &IID_ID3D12_COMMAND_ALLOCATOR,
            &mut alloc
        );
        if hr != S_OK || alloc.is_null() {
            return Err(format!(
                "gpu.run: CreateCommandAllocator failed (HRESULT 0x{hr:08x})"
            ));
        }
        let alloc = Com(alloc);
        let mut list: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_COMMAND_LIST,
            FnCreateCommandList,
            0,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            alloc.ptr(),
            pso.ptr(),
            &IID_ID3D12_GRAPHICS_COMMAND_LIST,
            &mut list
        );
        if hr != S_OK || list.is_null() {
            return Err(format!("gpu.run: CreateCommandList failed (HRESULT 0x{hr:08x})"));
        }
        let list = Com(list);

        // Buffers: one upload staging buffer holding [input | zeros],
        // two DEFAULT-heap UAV buffers, one readback buffer.
        let staging = create_buffer(
            dev,
            D3D12_HEAP_TYPE_UPLOAD,
            (input.len() + out_len) as u64,
            0,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            "staging",
        )?;
        let mut mapped: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            staging.ptr(),
            VT_RESOURCE_MAP,
            FnResourceMap,
            0,
            std::ptr::null(),
            &mut mapped
        );
        if hr != S_OK || mapped.is_null() {
            return Err(format!("gpu.run: Map(staging) failed (HRESULT 0x{hr:08x})"));
        }
        std::ptr::copy_nonoverlapping(input.as_ptr(), mapped as *mut u8, input.len());
        std::ptr::write_bytes((mapped as *mut u8).add(input.len()), 0, out_len);
        com_call!(staging.ptr(), VT_RESOURCE_UNMAP, FnResourceUnmap, 0, std::ptr::null());

        let inbuf = create_buffer(
            dev,
            D3D12_HEAP_TYPE_DEFAULT,
            input.len() as u64,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
            "input",
        )?;
        let outbuf = create_buffer(
            dev,
            D3D12_HEAP_TYPE_DEFAULT,
            out_len as u64,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_DEST,
            "output",
        )?;
        let readback = create_buffer(
            dev,
            D3D12_HEAP_TYPE_READBACK,
            out_len as u64,
            0,
            D3D12_RESOURCE_STATE_COPY_DEST,
            "readback",
        )?;

        // Record: fill both UAVs from staging, dispatch, copy out.
        com_call!(
            list.ptr(),
            VT_LIST_COPY_BUFFER_REGION,
            FnListCopyBufferRegion,
            inbuf.ptr(),
            0,
            staging.ptr(),
            0,
            input.len() as u64
        );
        com_call!(
            list.ptr(),
            VT_LIST_COPY_BUFFER_REGION,
            FnListCopyBufferRegion,
            outbuf.ptr(),
            0,
            staging.ptr(),
            input.len() as u64,
            out_len as u64
        );
        barrier(
            list.ptr(),
            inbuf.ptr(),
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        barrier(
            list.ptr(),
            outbuf.ptr(),
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );
        com_call!(list.ptr(), VT_LIST_SET_PIPELINE_STATE, FnListSetPtr, pso.ptr());
        com_call!(
            list.ptr(),
            VT_LIST_SET_COMPUTE_ROOT_SIGNATURE,
            FnListSetPtr,
            root_sig.ptr()
        );
        let in_va = com_call!(inbuf.ptr(), VT_RESOURCE_GET_GPU_VIRTUAL_ADDRESS, FnResourceGetGpuVa,);
        let out_va =
            com_call!(outbuf.ptr(), VT_RESOURCE_GET_GPU_VIRTUAL_ADDRESS, FnResourceGetGpuVa,);
        com_call!(list.ptr(), VT_LIST_SET_COMPUTE_ROOT_UAV, FnListSetComputeRootUav, 0, in_va);
        com_call!(list.ptr(), VT_LIST_SET_COMPUTE_ROOT_UAV, FnListSetComputeRootUav, 1, out_va);
        com_call!(list.ptr(), VT_LIST_DISPATCH, FnListDispatch, wx, wy, wz);
        barrier(
            list.ptr(),
            outbuf.ptr(),
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        com_call!(
            list.ptr(),
            VT_LIST_COPY_BUFFER_REGION,
            FnListCopyBufferRegion,
            readback.ptr(),
            0,
            outbuf.ptr(),
            0,
            out_len as u64
        );
        let hr = com_call!(list.ptr(), VT_LIST_CLOSE, FnListClose,);
        if hr != S_OK {
            return Err(format!("gpu.run: command-list Close failed (HRESULT 0x{hr:08x})"));
        }

        // Execute + fence-wait (synchronous, like every backend here).
        let lists = [list.ptr()];
        com_call!(
            queue.ptr(),
            VT_QUEUE_EXECUTE_COMMAND_LISTS,
            FnExecuteCommandLists,
            1,
            lists.as_ptr()
        );
        let mut fence: *mut c_void = std::ptr::null_mut();
        let hr = com_call!(
            dev,
            VT_DEVICE_CREATE_FENCE,
            FnCreateFence,
            0,
            0,
            &IID_ID3D12_FENCE,
            &mut fence
        );
        if hr != S_OK || fence.is_null() {
            return Err(format!("gpu.run: CreateFence failed (HRESULT 0x{hr:08x})"));
        }
        let fence = Com(fence);
        let event = CreateEventA(std::ptr::null_mut(), 0, 0, std::ptr::null());
        if event.is_null() {
            return Err("gpu.run: CreateEvent failed".to_string());
        }
        let event = Event(event);
        let hr = com_call!(
            fence.ptr(),
            VT_FENCE_SET_EVENT_ON_COMPLETION,
            FnFenceSetEventOnCompletion,
            1,
            event.0
        );
        if hr != S_OK {
            return Err(format!(
                "gpu.run: SetEventOnCompletion failed (HRESULT 0x{hr:08x})"
            ));
        }
        let hr = com_call!(queue.ptr(), VT_QUEUE_SIGNAL, FnQueueSignal, fence.ptr(), 1);
        if hr != S_OK {
            return Err(format!("gpu.run: queue Signal failed (HRESULT 0x{hr:08x})"));
        }
        WaitForSingleObject(event.0, INFINITE);

        // Read the result back.
        let mut out_ptr: *mut c_void = std::ptr::null_mut();
        let read_range = Range { begin: 0, end: out_len };
        let hr = com_call!(
            readback.ptr(),
            VT_RESOURCE_MAP,
            FnResourceMap,
            0,
            &read_range,
            &mut out_ptr
        );
        if hr != S_OK || out_ptr.is_null() {
            return Err(format!("gpu.run: Map(readback) failed (HRESULT 0x{hr:08x})"));
        }
        let mut out = vec![0u8; out_len];
        std::ptr::copy_nonoverlapping(out_ptr as *const u8, out.as_mut_ptr(), out_len);
        let empty = Range { begin: 0, end: 0 };
        com_call!(readback.ptr(), VT_RESOURCE_UNMAP, FnResourceUnmap, 0, &empty);
        Ok(out)
    }
}
