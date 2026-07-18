//! Raw-FFI Vulkan compute for the `gpu` namespace — the roadmap's second
//! native compute backend (after Metal), and the first consumer of the
//! SPIR-V lingua-franca decision: `gpu.run_spirv` takes a SPIR-V *binary*
//! (`Bytes`), which Vulkan ingests directly — no shader compiler, no
//! translator, no dependency. The same blob will later feed the OpenCL
//! 2.1+ and GL 4.6 backends (CLAUDE.md's roadmap), which is why the entry
//! point is named for the format, not the API.
//!
//! **Zero dependencies**: the Vulkan loader is resolved at runtime with
//! `dlopen("libvulkan.so.1")` on Linux / `LoadLibraryA("vulkan-1.dll")` on
//! Windows — the same dynamic-resolution strategy `window/x11/gl.rs` uses for
//! `libGL.so.1`, and for the same reason: the loader ships with GPU
//! drivers (and with Mesa's software rasterizer), not with the OS's
//! link-time SDK. Every entry point is then resolved through
//! `vkGetInstanceProcAddr`/`vkGetDeviceProcAddr`, exactly as the Vulkan
//! spec prescribes for loader-independent code. macOS is deliberately
//! absent: Apple ships no system Vulkan (MoltenVK would be a dependency),
//! and the native Metal backend already covers that platform.
//!
//! **Per-call lifecycle, deliberately**: each `run_spirv` builds and tears
//! down the whole instance→device→pipeline chain. That is slower than
//! caching (lavapipe instance creation is milliseconds) but leak-free by
//! construction and thread-safe without any shared-handle reasoning —
//! worker isolates can call it concurrently, each getting its own
//! instance. Per CLAUDE.md's efficiency-pass rule, a cached-device idiom
//! can later become the primitive underneath this exact surface once
//! measured, without changing observable behavior.
//!
//! **Struct transcription discipline**: every `#[repr(C)]` struct below is
//! transcribed from the Vulkan 1.0 core spec with exact field widths
//! (dispatchable handles are pointers; non-dispatchable handles are
//! `u64` even on 64-bit; `VkDeviceSize` is `u64`; enums are `i32`/`u32`).
//! `VkPhysicalDeviceProperties` is read through an over-sized tail pad —
//! only the leading fields through `deviceName` are interpreted, and the
//! pad guarantees the driver's write (824 bytes in 1.0) stays in bounds.
//!
//! **This module doubles as the crate's shared Vulkan primitive layer**
//! (the Vulkan analog of `crate::objc`, promoted the same way — when its
//! second consumer arrived): the loader ([`loader_gipa`], one
//! `dlopen` per process), the handle/scalar typedefs, the shared 1.0-core
//! constants/structs/function-pointer types are `pub(crate)` and consumed
//! by both this file's compute path and the Linux window backend
//! (`window/x11/vulkan.rs`). API-surface-specific shapes stay with their
//! sole consumers — WSI/swapchain/image machinery lives in the window
//! backend, descriptor/pipeline machinery here — mirroring how AppKit
//! messages stayed in `window/macos/shared.rs` when `objc.rs` graduated.

#![cfg_attr(
    not(all(feature = "vulkan", any(target_os = "linux", target_os = "windows"))),
    allow(dead_code)
)]

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Loader resolution (dlopen / LoadLibrary), mirroring window/x11/gl.rs's GL strategy.
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
        dlopen(c"libvulkan.so.1".as_ptr(), RTLD_NOW)
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
        LoadLibraryA(c"vulkan-1.dll".as_ptr())
    }
    pub(super) unsafe fn sym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
        GetProcAddress(handle, name)
    }
}

// ---------------------------------------------------------------------------
// Handles and scalar types (Vulkan 1.0).
// ---------------------------------------------------------------------------

// Dispatchable handles: pointers.
pub(crate) type VkInstance = *mut c_void;
pub(crate) type VkPhysicalDevice = *mut c_void;
pub(crate) type VkDevice = *mut c_void;
pub(crate) type VkQueue = *mut c_void;
pub(crate) type VkCommandBuffer = *mut c_void;
// Non-dispatchable handles: 64-bit integers on every platform.
pub(crate) type VkBuffer = u64;
pub(crate) type VkDeviceMemory = u64;
pub(crate) type VkShaderModule = u64;
pub(crate) type VkDescriptorSetLayout = u64;
pub(crate) type VkPipelineLayout = u64;
pub(crate) type VkPipeline = u64;
type VkPipelineCache = u64;
pub(crate) type VkDescriptorPool = u64;
pub(crate) type VkDescriptorSet = u64;
pub(crate) type VkCommandPool = u64;
pub(crate) type VkFence = u64;

pub(crate) type VkResult = i32;
pub(crate) const VK_SUCCESS: VkResult = 0;
pub(crate) const VK_TRUE: u32 = 1;

pub(crate) const VK_API_VERSION_1_0: u32 = 1 << 22;
const VK_QUEUE_COMPUTE_BIT: u32 = 0x2;
pub(crate) const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 0x2;
pub(crate) const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 0x4;
const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: u32 = 0x20;
pub(crate) const VK_SHARING_MODE_EXCLUSIVE: u32 = 0;
const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: u32 = 7;
const VK_SHADER_STAGE_COMPUTE_BIT: u32 = 0x20;
const VK_PIPELINE_BIND_POINT_COMPUTE: u32 = 1;
pub(crate) const VK_COMMAND_BUFFER_LEVEL_PRIMARY: u32 = 0;
pub(crate) const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 0x1;

// VkStructureType values (core 1.0).
pub(crate) const ST_APPLICATION_INFO: i32 = 0;
pub(crate) const ST_INSTANCE_CREATE_INFO: i32 = 1;
pub(crate) const ST_DEVICE_QUEUE_CREATE_INFO: i32 = 2;
pub(crate) const ST_DEVICE_CREATE_INFO: i32 = 3;
pub(crate) const ST_SUBMIT_INFO: i32 = 4;
pub(crate) const ST_MEMORY_ALLOCATE_INFO: i32 = 5;
pub(crate) const ST_FENCE_CREATE_INFO: i32 = 8;
pub(crate) const ST_BUFFER_CREATE_INFO: i32 = 12;
pub(crate) const ST_SHADER_MODULE_CREATE_INFO: i32 = 16;
pub(crate) const ST_PIPELINE_SHADER_STAGE_CREATE_INFO: i32 = 18;
const ST_COMPUTE_PIPELINE_CREATE_INFO: i32 = 29;
pub(crate) const ST_PIPELINE_LAYOUT_CREATE_INFO: i32 = 30;
pub(crate) const ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: i32 = 32;
pub(crate) const ST_DESCRIPTOR_POOL_CREATE_INFO: i32 = 33;
pub(crate) const ST_DESCRIPTOR_SET_ALLOCATE_INFO: i32 = 34;
pub(crate) const ST_WRITE_DESCRIPTOR_SET: i32 = 35;
pub(crate) const ST_COMMAND_POOL_CREATE_INFO: i32 = 39;
pub(crate) const ST_COMMAND_BUFFER_ALLOCATE_INFO: i32 = 40;
pub(crate) const ST_COMMAND_BUFFER_BEGIN_INFO: i32 = 42;

// ---------------------------------------------------------------------------
// Structs (Vulkan 1.0 core, exact field widths).
// ---------------------------------------------------------------------------

#[repr(C)]
pub(crate) struct VkApplicationInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) p_application_name: *const c_char,
    pub(crate) application_version: u32,
    pub(crate) p_engine_name: *const c_char,
    pub(crate) engine_version: u32,
    pub(crate) api_version: u32,
}
#[repr(C)]
pub(crate) struct VkInstanceCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) p_application_info: *const VkApplicationInfo,
    pub(crate) enabled_layer_count: u32,
    pub(crate) pp_enabled_layer_names: *const *const c_char,
    pub(crate) enabled_extension_count: u32,
    pub(crate) pp_enabled_extension_names: *const *const c_char,
}
#[repr(C)]
pub(crate) struct VkDeviceQueueCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) queue_family_index: u32,
    pub(crate) queue_count: u32,
    pub(crate) p_queue_priorities: *const f32,
}
#[repr(C)]
pub(crate) struct VkDeviceCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) queue_create_info_count: u32,
    pub(crate) p_queue_create_infos: *const VkDeviceQueueCreateInfo,
    pub(crate) enabled_layer_count: u32,
    pub(crate) pp_enabled_layer_names: *const *const c_char,
    pub(crate) enabled_extension_count: u32,
    pub(crate) pp_enabled_extension_names: *const *const c_char,
    pub(crate) p_enabled_features: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VkQueueFamilyProperties {
    pub(crate) queue_flags: u32,
    pub(crate) queue_count: u32,
    pub(crate) timestamp_valid_bits: u32,
    pub(crate) min_image_transfer_granularity: [u32; 3],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VkMemoryType {
    pub(crate) property_flags: u32,
    pub(crate) heap_index: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VkMemoryHeap {
    pub(crate) size: u64,
    pub(crate) flags: u32,
}
#[repr(C)]
pub(crate) struct VkPhysicalDeviceMemoryProperties {
    pub(crate) memory_type_count: u32,
    pub(crate) memory_types: [VkMemoryType; 32],
    pub(crate) memory_heap_count: u32,
    pub(crate) memory_heaps: [VkMemoryHeap; 16],
}
/// Only the fields through `device_name` are interpreted; the tail pad
/// covers the rest of the real 1.0 struct (limits + sparse properties,
/// 824 bytes total) with generous margin so the driver's write stays in
/// bounds.
#[repr(C)]
struct VkPhysicalDevicePropertiesPadded {
    api_version: u32,
    driver_version: u32,
    vendor_id: u32,
    device_id: u32,
    device_type: u32,
    device_name: [c_char; 256],
    tail: [u8; 1024],
}
#[repr(C)]
pub(crate) struct VkBufferCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) size: u64,
    pub(crate) usage: u32,
    pub(crate) sharing_mode: u32,
    pub(crate) queue_family_index_count: u32,
    pub(crate) p_queue_family_indices: *const u32,
}
#[repr(C)]
pub(crate) struct VkMemoryRequirements {
    pub(crate) size: u64,
    pub(crate) alignment: u64,
    pub(crate) memory_type_bits: u32,
}
#[repr(C)]
pub(crate) struct VkMemoryAllocateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) allocation_size: u64,
    pub(crate) memory_type_index: u32,
}
#[repr(C)]
pub(crate) struct VkShaderModuleCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) code_size: usize,
    pub(crate) p_code: *const u32,
}
#[repr(C)]
pub(crate) struct VkDescriptorSetLayoutBinding {
    pub(crate) binding: u32,
    pub(crate) descriptor_type: u32,
    pub(crate) descriptor_count: u32,
    pub(crate) stage_flags: u32,
    pub(crate) p_immutable_samplers: *const c_void,
}
#[repr(C)]
pub(crate) struct VkDescriptorSetLayoutCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) binding_count: u32,
    pub(crate) p_bindings: *const VkDescriptorSetLayoutBinding,
}
#[repr(C)]
pub(crate) struct VkPipelineLayoutCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) set_layout_count: u32,
    pub(crate) p_set_layouts: *const VkDescriptorSetLayout,
    pub(crate) push_constant_range_count: u32,
    pub(crate) p_push_constant_ranges: *const c_void,
}
#[repr(C)]
pub(crate) struct VkPipelineShaderStageCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) stage: u32,
    pub(crate) module: VkShaderModule,
    pub(crate) p_name: *const c_char,
    pub(crate) p_specialization_info: *const c_void,
}
#[repr(C)]
struct VkComputePipelineCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage: VkPipelineShaderStageCreateInfo,
    layout: VkPipelineLayout,
    base_pipeline_handle: VkPipeline,
    base_pipeline_index: i32,
}
#[repr(C)]
pub(crate) struct VkDescriptorPoolSize {
    pub(crate) ty: u32,
    pub(crate) descriptor_count: u32,
}
#[repr(C)]
pub(crate) struct VkDescriptorPoolCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) max_sets: u32,
    pub(crate) pool_size_count: u32,
    pub(crate) p_pool_sizes: *const VkDescriptorPoolSize,
}
#[repr(C)]
pub(crate) struct VkDescriptorSetAllocateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) descriptor_pool: VkDescriptorPool,
    pub(crate) descriptor_set_count: u32,
    pub(crate) p_set_layouts: *const VkDescriptorSetLayout,
}
#[repr(C)]
pub(crate) struct VkDescriptorBufferInfo {
    pub(crate) buffer: VkBuffer,
    pub(crate) offset: u64,
    pub(crate) range: u64,
}
#[repr(C)]
pub(crate) struct VkWriteDescriptorSet {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) dst_set: VkDescriptorSet,
    pub(crate) dst_binding: u32,
    pub(crate) dst_array_element: u32,
    pub(crate) descriptor_count: u32,
    pub(crate) descriptor_type: u32,
    pub(crate) p_image_info: *const c_void,
    pub(crate) p_buffer_info: *const VkDescriptorBufferInfo,
    pub(crate) p_texel_buffer_view: *const c_void,
}
#[repr(C)]
pub(crate) struct VkCommandPoolCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) queue_family_index: u32,
}
#[repr(C)]
pub(crate) struct VkCommandBufferAllocateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) command_pool: VkCommandPool,
    pub(crate) level: u32,
    pub(crate) command_buffer_count: u32,
}
#[repr(C)]
pub(crate) struct VkCommandBufferBeginInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
    pub(crate) p_inheritance_info: *const c_void,
}
#[repr(C)]
pub(crate) struct VkSubmitInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) wait_semaphore_count: u32,
    pub(crate) p_wait_semaphores: *const c_void,
    pub(crate) p_wait_dst_stage_mask: *const u32,
    pub(crate) command_buffer_count: u32,
    pub(crate) p_command_buffers: *const VkCommandBuffer,
    pub(crate) signal_semaphore_count: u32,
    pub(crate) p_signal_semaphores: *const c_void,
}
#[repr(C)]
pub(crate) struct VkFenceCreateInfo {
    pub(crate) s_type: i32,
    pub(crate) p_next: *const c_void,
    pub(crate) flags: u32,
}

// ---------------------------------------------------------------------------
// Function-pointer types and the resolved table.
// ---------------------------------------------------------------------------

pub(crate) type PfnVoidFunction = *mut c_void;
pub(crate) type FnGetInstanceProcAddr =
    unsafe extern "system" fn(VkInstance, *const c_char) -> PfnVoidFunction;
pub(crate) type FnCreateInstance = unsafe extern "system" fn(
    *const VkInstanceCreateInfo,
    *const c_void,
    *mut VkInstance,
) -> VkResult;
pub(crate) type FnDestroyInstance = unsafe extern "system" fn(VkInstance, *const c_void);
pub(crate) type FnEnumeratePhysicalDevices =
    unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> VkResult;
type FnGetPhysicalDeviceProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDevicePropertiesPadded);
pub(crate) type FnGetPhysicalDeviceQueueFamilyProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
pub(crate) type FnGetPhysicalDeviceMemoryProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
pub(crate) type FnCreateDevice = unsafe extern "system" fn(
    VkPhysicalDevice,
    *const VkDeviceCreateInfo,
    *const c_void,
    *mut VkDevice,
) -> VkResult;
type FnGetDeviceProcAddr = unsafe extern "system" fn(VkDevice, *const c_char) -> PfnVoidFunction;
pub(crate) type FnDestroyDevice = unsafe extern "system" fn(VkDevice, *const c_void);
pub(crate) type FnGetDeviceQueue = unsafe extern "system" fn(VkDevice, u32, u32, *mut VkQueue);
pub(crate) type FnCreateShaderModule = unsafe extern "system" fn(
    VkDevice,
    *const VkShaderModuleCreateInfo,
    *const c_void,
    *mut VkShaderModule,
) -> VkResult;
pub(crate) type FnDestroyShaderModule = unsafe extern "system" fn(VkDevice, VkShaderModule, *const c_void);
pub(crate) type FnCreateDescriptorSetLayout = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorSetLayoutCreateInfo,
    *const c_void,
    *mut VkDescriptorSetLayout,
) -> VkResult;
pub(crate) type FnDestroyDescriptorSetLayout =
    unsafe extern "system" fn(VkDevice, VkDescriptorSetLayout, *const c_void);
pub(crate) type FnCreatePipelineLayout = unsafe extern "system" fn(
    VkDevice,
    *const VkPipelineLayoutCreateInfo,
    *const c_void,
    *mut VkPipelineLayout,
) -> VkResult;
pub(crate) type FnDestroyPipelineLayout =
    unsafe extern "system" fn(VkDevice, VkPipelineLayout, *const c_void);
type FnCreateComputePipelines = unsafe extern "system" fn(
    VkDevice,
    VkPipelineCache,
    u32,
    *const VkComputePipelineCreateInfo,
    *const c_void,
    *mut VkPipeline,
) -> VkResult;
pub(crate) type FnDestroyPipeline = unsafe extern "system" fn(VkDevice, VkPipeline, *const c_void);
pub(crate) type FnCreateDescriptorPool = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorPoolCreateInfo,
    *const c_void,
    *mut VkDescriptorPool,
) -> VkResult;
pub(crate) type FnDestroyDescriptorPool =
    unsafe extern "system" fn(VkDevice, VkDescriptorPool, *const c_void);
pub(crate) type FnAllocateDescriptorSets = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorSetAllocateInfo,
    *mut VkDescriptorSet,
) -> VkResult;
pub(crate) type FnUpdateDescriptorSets =
    unsafe extern "system" fn(VkDevice, u32, *const VkWriteDescriptorSet, u32, *const c_void);
pub(crate) type FnCreateBuffer = unsafe extern "system" fn(
    VkDevice,
    *const VkBufferCreateInfo,
    *const c_void,
    *mut VkBuffer,
) -> VkResult;
pub(crate) type FnDestroyBuffer = unsafe extern "system" fn(VkDevice, VkBuffer, *const c_void);
pub(crate) type FnGetBufferMemoryRequirements =
    unsafe extern "system" fn(VkDevice, VkBuffer, *mut VkMemoryRequirements);
pub(crate) type FnAllocateMemory = unsafe extern "system" fn(
    VkDevice,
    *const VkMemoryAllocateInfo,
    *const c_void,
    *mut VkDeviceMemory,
) -> VkResult;
pub(crate) type FnFreeMemory = unsafe extern "system" fn(VkDevice, VkDeviceMemory, *const c_void);
pub(crate) type FnBindBufferMemory =
    unsafe extern "system" fn(VkDevice, VkBuffer, VkDeviceMemory, u64) -> VkResult;
pub(crate) type FnMapMemory = unsafe extern "system" fn(
    VkDevice,
    VkDeviceMemory,
    u64,
    u64,
    u32,
    *mut *mut c_void,
) -> VkResult;
pub(crate) type FnCreateCommandPool = unsafe extern "system" fn(
    VkDevice,
    *const VkCommandPoolCreateInfo,
    *const c_void,
    *mut VkCommandPool,
) -> VkResult;
pub(crate) type FnDestroyCommandPool = unsafe extern "system" fn(VkDevice, VkCommandPool, *const c_void);
pub(crate) type FnAllocateCommandBuffers = unsafe extern "system" fn(
    VkDevice,
    *const VkCommandBufferAllocateInfo,
    *mut VkCommandBuffer,
) -> VkResult;
pub(crate) type FnBeginCommandBuffer =
    unsafe extern "system" fn(VkCommandBuffer, *const VkCommandBufferBeginInfo) -> VkResult;
pub(crate) type FnCmdBindPipeline = unsafe extern "system" fn(VkCommandBuffer, u32, VkPipeline);
pub(crate) type FnCmdBindDescriptorSets = unsafe extern "system" fn(
    VkCommandBuffer,
    u32,
    VkPipelineLayout,
    u32,
    u32,
    *const VkDescriptorSet,
    u32,
    *const u32,
);
type FnCmdDispatch = unsafe extern "system" fn(VkCommandBuffer, u32, u32, u32);
pub(crate) type FnEndCommandBuffer = unsafe extern "system" fn(VkCommandBuffer) -> VkResult;
pub(crate) type FnQueueSubmit =
    unsafe extern "system" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;
pub(crate) type FnCreateFence = unsafe extern "system" fn(
    VkDevice,
    *const VkFenceCreateInfo,
    *const c_void,
    *mut VkFence,
) -> VkResult;
pub(crate) type FnDestroyFence = unsafe extern "system" fn(VkDevice, VkFence, *const c_void);
pub(crate) type FnWaitForFences =
    unsafe extern "system" fn(VkDevice, u32, *const VkFence, u32, u64) -> VkResult;

/// `vkGetInstanceProcAddr`, resolved once per process from the dynamically
/// opened loader (nul = loader missing — every entry point reports "no
/// adapter"-shaped errors from that).
pub(crate) fn loader_gipa() -> Option<FnGetInstanceProcAddr> {
    static CELL: OnceLock<usize> = OnceLock::new();
    let addr = *CELL.get_or_init(|| unsafe {
        let lib = libloading::open();
        if lib.is_null() {
            return 0;
        }
        libloading::sym(lib, c"vkGetInstanceProcAddr".as_ptr()) as usize
    });
    if addr == 0 {
        None
    } else {
        Some(unsafe { std::mem::transmute::<usize, FnGetInstanceProcAddr>(addr) })
    }
}

/// Resolve one entry point via `vkGetInstanceProcAddr`, `Err`ing with its
/// name on failure — a missing core-1.0 symbol means a broken loader, and
/// the name makes that diagnosable from the message alone.
macro_rules! load {
    ($gipa:expr, $instance:expr, $name:literal, $ty:ty) => {{
        let p = $gipa($instance, concat!($name, "\0").as_ptr() as *const c_char);
        if p.is_null() {
            return Err(format!("gpu.run_spirv: loader has no {}", $name));
        }
        std::mem::transmute::<*mut c_void, $ty>(p)
    }};
}

// ---------------------------------------------------------------------------
// Device-level function table + RAII cleanup.
// ---------------------------------------------------------------------------

struct DeviceFns {
    destroy_device: FnDestroyDevice,
    get_device_queue: FnGetDeviceQueue,
    create_shader_module: FnCreateShaderModule,
    destroy_shader_module: FnDestroyShaderModule,
    create_descriptor_set_layout: FnCreateDescriptorSetLayout,
    destroy_descriptor_set_layout: FnDestroyDescriptorSetLayout,
    create_pipeline_layout: FnCreatePipelineLayout,
    destroy_pipeline_layout: FnDestroyPipelineLayout,
    create_compute_pipelines: FnCreateComputePipelines,
    destroy_pipeline: FnDestroyPipeline,
    create_descriptor_pool: FnCreateDescriptorPool,
    destroy_descriptor_pool: FnDestroyDescriptorPool,
    allocate_descriptor_sets: FnAllocateDescriptorSets,
    update_descriptor_sets: FnUpdateDescriptorSets,
    create_buffer: FnCreateBuffer,
    destroy_buffer: FnDestroyBuffer,
    get_buffer_memory_requirements: FnGetBufferMemoryRequirements,
    allocate_memory: FnAllocateMemory,
    free_memory: FnFreeMemory,
    bind_buffer_memory: FnBindBufferMemory,
    map_memory: FnMapMemory,
    create_command_pool: FnCreateCommandPool,
    destroy_command_pool: FnDestroyCommandPool,
    allocate_command_buffers: FnAllocateCommandBuffers,
    begin_command_buffer: FnBeginCommandBuffer,
    cmd_bind_pipeline: FnCmdBindPipeline,
    cmd_bind_descriptor_sets: FnCmdBindDescriptorSets,
    cmd_dispatch: FnCmdDispatch,
    end_command_buffer: FnEndCommandBuffer,
    queue_submit: FnQueueSubmit,
    create_fence: FnCreateFence,
    destroy_fence: FnDestroyFence,
    wait_for_fences: FnWaitForFences,
}

/// Everything `run_spirv` creates, torn down in reverse creation order by
/// `Drop` — so every early `return Err(...)` cleans up whatever exists so
/// far without per-path release chains (the Rust-native spelling of the
/// no-partial-leaks discipline the Metal backends hand-roll).
struct Run {
    destroy_instance: Option<FnDestroyInstance>,
    instance: VkInstance,
    fns: Option<DeviceFns>,
    device: VkDevice,
    shader: VkShaderModule,
    dsl: VkDescriptorSetLayout,
    playout: VkPipelineLayout,
    pipeline: VkPipeline,
    dpool: VkDescriptorPool,
    cpool: VkCommandPool,
    fence: VkFence,
    inbuf: VkBuffer,
    inmem: VkDeviceMemory,
    outbuf: VkBuffer,
    outmem: VkDeviceMemory,
}

impl Run {
    fn new() -> Run {
        Run {
            destroy_instance: None,
            instance: std::ptr::null_mut(),
            fns: None,
            device: std::ptr::null_mut(),
            shader: 0,
            dsl: 0,
            playout: 0,
            pipeline: 0,
            dpool: 0,
            cpool: 0,
            fence: 0,
            inbuf: 0,
            inmem: 0,
            outbuf: 0,
            outmem: 0,
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        unsafe {
            if let Some(f) = &self.fns {
                if !self.device.is_null() {
                    let d = self.device;
                    let nul = std::ptr::null();
                    if self.fence != 0 {
                        (f.destroy_fence)(d, self.fence, nul);
                    }
                    // Destroying the pool frees its command buffers.
                    if self.cpool != 0 {
                        (f.destroy_command_pool)(d, self.cpool, nul);
                    }
                    // Destroying the pool frees its descriptor sets.
                    if self.dpool != 0 {
                        (f.destroy_descriptor_pool)(d, self.dpool, nul);
                    }
                    if self.pipeline != 0 {
                        (f.destroy_pipeline)(d, self.pipeline, nul);
                    }
                    if self.playout != 0 {
                        (f.destroy_pipeline_layout)(d, self.playout, nul);
                    }
                    if self.dsl != 0 {
                        (f.destroy_descriptor_set_layout)(d, self.dsl, nul);
                    }
                    if self.shader != 0 {
                        (f.destroy_shader_module)(d, self.shader, nul);
                    }
                    if self.inbuf != 0 {
                        (f.destroy_buffer)(d, self.inbuf, nul);
                    }
                    // Freeing mapped memory implicitly unmaps it (spec).
                    if self.inmem != 0 {
                        (f.free_memory)(d, self.inmem, nul);
                    }
                    if self.outbuf != 0 {
                        (f.destroy_buffer)(d, self.outbuf, nul);
                    }
                    if self.outmem != 0 {
                        (f.free_memory)(d, self.outmem, nul);
                    }
                    (f.destroy_device)(d, nul);
                }
            }
            if !self.instance.is_null() {
                if let Some(di) = self.destroy_instance {
                    di(self.instance, std::ptr::null());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Instance / device selection, shared by available/adapter_info/run_spirv.
// ---------------------------------------------------------------------------

unsafe fn create_instance(
    gipa: FnGetInstanceProcAddr,
) -> Result<(VkInstance, FnDestroyInstance), String> {
    let create: FnCreateInstance = load!(gipa, std::ptr::null_mut(), "vkCreateInstance", _);
    let app = VkApplicationInfo {
        s_type: ST_APPLICATION_INFO,
        p_next: std::ptr::null(),
        p_application_name: c"socrates".as_ptr(),
        application_version: 0,
        p_engine_name: c"socrates".as_ptr(),
        engine_version: 0,
        api_version: VK_API_VERSION_1_0,
    };
    let info = VkInstanceCreateInfo {
        s_type: ST_INSTANCE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        p_application_info: &app,
        enabled_layer_count: 0,
        pp_enabled_layer_names: std::ptr::null(),
        enabled_extension_count: 0,
        pp_enabled_extension_names: std::ptr::null(),
    };
    let mut instance: VkInstance = std::ptr::null_mut();
    let r = create(&info, std::ptr::null(), &mut instance);
    if r != VK_SUCCESS || instance.is_null() {
        return Err(format!("gpu.run_spirv: vkCreateInstance failed (VkResult {r})"));
    }
    let destroy: FnDestroyInstance = load!(gipa, instance, "vkDestroyInstance", _);
    Ok((instance, destroy))
}

/// First physical device exposing a compute-capable queue family — the
/// default-adapter idiom every gpu backend uses. Returns the device and
/// the queue family index.
unsafe fn pick_device(
    gipa: FnGetInstanceProcAddr,
    instance: VkInstance,
) -> Result<(VkPhysicalDevice, u32), String> {
    let enumerate: FnEnumeratePhysicalDevices =
        load!(gipa, instance, "vkEnumeratePhysicalDevices", _);
    let get_qfp: FnGetPhysicalDeviceQueueFamilyProperties =
        load!(gipa, instance, "vkGetPhysicalDeviceQueueFamilyProperties", _);
    let mut count: u32 = 0;
    let r = enumerate(instance, &mut count, std::ptr::null_mut());
    if r != VK_SUCCESS || count == 0 {
        return Err("gpu.run_spirv: no Vulkan physical devices".to_string());
    }
    let mut devices = vec![std::ptr::null_mut(); count as usize];
    let r = enumerate(instance, &mut count, devices.as_mut_ptr());
    if r != VK_SUCCESS {
        return Err(format!(
            "gpu.run_spirv: vkEnumeratePhysicalDevices failed (VkResult {r})"
        ));
    }
    for &pd in &devices {
        let mut qcount: u32 = 0;
        get_qfp(pd, &mut qcount, std::ptr::null_mut());
        let mut families = vec![
            VkQueueFamilyProperties {
                queue_flags: 0,
                queue_count: 0,
                timestamp_valid_bits: 0,
                min_image_transfer_granularity: [0; 3],
            };
            qcount as usize
        ];
        get_qfp(pd, &mut qcount, families.as_mut_ptr());
        for (i, fam) in families.iter().enumerate() {
            if fam.queue_flags & VK_QUEUE_COMPUTE_BIT != 0 && fam.queue_count > 0 {
                return Ok((pd, i as u32));
            }
        }
    }
    Err("gpu.run_spirv: no Vulkan device has a compute queue".to_string())
}

/// Is a compute-capable Vulkan device reachable? (Loader present, instance
/// creates, a device has a compute queue family.)
pub(crate) fn available() -> bool {
    let Some(gipa) = loader_gipa() else {
        return false;
    };
    unsafe {
        let Ok((instance, destroy)) = create_instance(gipa) else {
            return false;
        };
        let ok = pick_device(gipa, instance).is_ok();
        destroy(instance, std::ptr::null());
        ok
    }
}

/// `"<deviceName> (vulkan)"` — the same `"<name> (<backend>)"` shape the
/// other gpu backends report, or `"no adapter"`.
pub(crate) fn adapter_info() -> String {
    let Some(gipa) = loader_gipa() else {
        return "no adapter".to_string();
    };
    unsafe {
        let Ok((instance, destroy)) = create_instance(gipa) else {
            return "no adapter".to_string();
        };
        let info = (|| -> Result<String, String> {
            let (pd, _) = pick_device(gipa, instance)?;
            let get_props: FnGetPhysicalDeviceProperties =
                load!(gipa, instance, "vkGetPhysicalDeviceProperties", _);
            let mut props = std::mem::MaybeUninit::<VkPhysicalDevicePropertiesPadded>::zeroed();
            get_props(pd, props.as_mut_ptr());
            let props = props.assume_init();
            let name = CStr::from_ptr(props.device_name.as_ptr())
                .to_string_lossy()
                .into_owned();
            Ok(format!("{name} (vulkan)"))
        })();
        destroy(instance, std::ptr::null());
        info.unwrap_or_else(|_| "no adapter".to_string())
    }
}

/// Create a storage buffer backed by HOST_VISIBLE|HOST_COHERENT memory and
/// return `(buffer, memory, mapped pointer)`. The mapping stays live for
/// the buffer's lifetime (freeing mapped memory implicitly unmaps).
unsafe fn create_host_buffer(
    f: &DeviceFns,
    device: VkDevice,
    memprops: &VkPhysicalDeviceMemoryProperties,
    size: u64,
) -> Result<(VkBuffer, VkDeviceMemory, *mut u8), String> {
    let info = VkBufferCreateInfo {
        s_type: ST_BUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        size,
        usage: VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut buf: VkBuffer = 0;
    let r = (f.create_buffer)(device, &info, std::ptr::null(), &mut buf);
    if r != VK_SUCCESS {
        return Err(format!("gpu.run_spirv: vkCreateBuffer failed (VkResult {r})"));
    }
    let mut req = VkMemoryRequirements {
        size: 0,
        alignment: 0,
        memory_type_bits: 0,
    };
    (f.get_buffer_memory_requirements)(device, buf, &mut req);
    let wanted = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    let mut type_index = None;
    for i in 0..memprops.memory_type_count.min(32) {
        let supported = req.memory_type_bits & (1 << i) != 0;
        let flags = memprops.memory_types[i as usize].property_flags;
        if supported && flags & wanted == wanted {
            type_index = Some(i);
            break;
        }
    }
    let Some(type_index) = type_index else {
        (f.destroy_buffer)(device, buf, std::ptr::null());
        return Err("gpu.run_spirv: no host-visible coherent memory type".to_string());
    };
    let alloc = VkMemoryAllocateInfo {
        s_type: ST_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: req.size,
        memory_type_index: type_index,
    };
    let mut mem: VkDeviceMemory = 0;
    let r = (f.allocate_memory)(device, &alloc, std::ptr::null(), &mut mem);
    if r != VK_SUCCESS {
        (f.destroy_buffer)(device, buf, std::ptr::null());
        return Err(format!("gpu.run_spirv: vkAllocateMemory failed (VkResult {r})"));
    }
    let r = (f.bind_buffer_memory)(device, buf, mem, 0);
    if r != VK_SUCCESS {
        (f.destroy_buffer)(device, buf, std::ptr::null());
        (f.free_memory)(device, mem, std::ptr::null());
        return Err(format!("gpu.run_spirv: vkBindBufferMemory failed (VkResult {r})"));
    }
    let mut mapped: *mut c_void = std::ptr::null_mut();
    let r = (f.map_memory)(device, mem, 0, u64::MAX /* VK_WHOLE_SIZE */, 0, &mut mapped);
    if r != VK_SUCCESS || mapped.is_null() {
        (f.destroy_buffer)(device, buf, std::ptr::null());
        (f.free_memory)(device, mem, std::ptr::null());
        return Err(format!("gpu.run_spirv: vkMapMemory failed (VkResult {r})"));
    }
    Ok((buf, mem, mapped as *mut u8))
}

/// One compute dispatch of a SPIR-V binary — see `gpu.rs`'s module docs for
/// the ABI (entry point `main`, storage buffers at set 0 bindings 0/1, the
/// caller has already validated sizes/counts via `gpu::validate`).
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

    let Some(gipa) = loader_gipa() else {
        return Err("gpu.run_spirv: no Vulkan loader (libvulkan) on this system".to_string());
    };

    let mut run = Run::new();
    unsafe {
        let (instance, destroy_instance) = create_instance(gipa)?;
        run.instance = instance;
        run.destroy_instance = Some(destroy_instance);

        let (pd, qfam) = pick_device(gipa, instance)?;
        let get_memprops: FnGetPhysicalDeviceMemoryProperties =
            load!(gipa, instance, "vkGetPhysicalDeviceMemoryProperties", _);
        let mut memprops = std::mem::MaybeUninit::<VkPhysicalDeviceMemoryProperties>::zeroed();
        get_memprops(pd, memprops.as_mut_ptr());
        let memprops = memprops.assume_init();

        // Device with one compute queue.
        let create_device: FnCreateDevice = load!(gipa, instance, "vkCreateDevice", _);
        let priority = 1.0f32;
        let qinfo = VkDeviceQueueCreateInfo {
            s_type: ST_DEVICE_QUEUE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            queue_family_index: qfam,
            queue_count: 1,
            p_queue_priorities: &priority,
        };
        let dinfo = VkDeviceCreateInfo {
            s_type: ST_DEVICE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            queue_create_info_count: 1,
            p_queue_create_infos: &qinfo,
            enabled_layer_count: 0,
            pp_enabled_layer_names: std::ptr::null(),
            enabled_extension_count: 0,
            pp_enabled_extension_names: std::ptr::null(),
            p_enabled_features: std::ptr::null(),
        };
        let mut device: VkDevice = std::ptr::null_mut();
        let r = create_device(pd, &dinfo, std::ptr::null(), &mut device);
        if r != VK_SUCCESS || device.is_null() {
            return Err(format!("gpu.run_spirv: vkCreateDevice failed (VkResult {r})"));
        }

        // Resolve the device-level table (via vkGetDeviceProcAddr, the
        // spec's zero-overhead dispatch path).
        let gdpa: FnGetDeviceProcAddr = load!(gipa, instance, "vkGetDeviceProcAddr", _);
        macro_rules! dload {
            ($name:literal, $ty:ty) => {{
                let p = gdpa(device, concat!($name, "\0").as_ptr() as *const c_char);
                if p.is_null() {
                    // The device exists but a core-1.0 symbol is missing —
                    // destroy what the Run guard doesn't know about yet.
                    let di: FnDestroyDevice =
                        load!(gipa, instance, "vkDestroyDevice", FnDestroyDevice);
                    di(device, std::ptr::null());
                    return Err(format!("gpu.run_spirv: device has no {}", $name));
                }
                std::mem::transmute::<*mut c_void, $ty>(p)
            }};
        }
        let fns = DeviceFns {
            destroy_device: dload!("vkDestroyDevice", FnDestroyDevice),
            get_device_queue: dload!("vkGetDeviceQueue", FnGetDeviceQueue),
            create_shader_module: dload!("vkCreateShaderModule", FnCreateShaderModule),
            destroy_shader_module: dload!("vkDestroyShaderModule", FnDestroyShaderModule),
            create_descriptor_set_layout: dload!(
                "vkCreateDescriptorSetLayout",
                FnCreateDescriptorSetLayout
            ),
            destroy_descriptor_set_layout: dload!(
                "vkDestroyDescriptorSetLayout",
                FnDestroyDescriptorSetLayout
            ),
            create_pipeline_layout: dload!("vkCreatePipelineLayout", FnCreatePipelineLayout),
            destroy_pipeline_layout: dload!("vkDestroyPipelineLayout", FnDestroyPipelineLayout),
            create_compute_pipelines: dload!("vkCreateComputePipelines", FnCreateComputePipelines),
            destroy_pipeline: dload!("vkDestroyPipeline", FnDestroyPipeline),
            create_descriptor_pool: dload!("vkCreateDescriptorPool", FnCreateDescriptorPool),
            destroy_descriptor_pool: dload!("vkDestroyDescriptorPool", FnDestroyDescriptorPool),
            allocate_descriptor_sets: dload!("vkAllocateDescriptorSets", FnAllocateDescriptorSets),
            update_descriptor_sets: dload!("vkUpdateDescriptorSets", FnUpdateDescriptorSets),
            create_buffer: dload!("vkCreateBuffer", FnCreateBuffer),
            destroy_buffer: dload!("vkDestroyBuffer", FnDestroyBuffer),
            get_buffer_memory_requirements: dload!(
                "vkGetBufferMemoryRequirements",
                FnGetBufferMemoryRequirements
            ),
            allocate_memory: dload!("vkAllocateMemory", FnAllocateMemory),
            free_memory: dload!("vkFreeMemory", FnFreeMemory),
            bind_buffer_memory: dload!("vkBindBufferMemory", FnBindBufferMemory),
            map_memory: dload!("vkMapMemory", FnMapMemory),
            create_command_pool: dload!("vkCreateCommandPool", FnCreateCommandPool),
            destroy_command_pool: dload!("vkDestroyCommandPool", FnDestroyCommandPool),
            allocate_command_buffers: dload!("vkAllocateCommandBuffers", FnAllocateCommandBuffers),
            begin_command_buffer: dload!("vkBeginCommandBuffer", FnBeginCommandBuffer),
            cmd_bind_pipeline: dload!("vkCmdBindPipeline", FnCmdBindPipeline),
            cmd_bind_descriptor_sets: dload!("vkCmdBindDescriptorSets", FnCmdBindDescriptorSets),
            cmd_dispatch: dload!("vkCmdDispatch", FnCmdDispatch),
            end_command_buffer: dload!("vkEndCommandBuffer", FnEndCommandBuffer),
            queue_submit: dload!("vkQueueSubmit", FnQueueSubmit),
            create_fence: dload!("vkCreateFence", FnCreateFence),
            destroy_fence: dload!("vkDestroyFence", FnDestroyFence),
            wait_for_fences: dload!("vkWaitForFences", FnWaitForFences),
        };
        run.device = device;
        run.fns = Some(fns);
        let f = run.fns.as_ref().unwrap();

        let mut queue: VkQueue = std::ptr::null_mut();
        (f.get_device_queue)(device, qfam, 0, &mut queue);
        if queue.is_null() {
            return Err("gpu.run_spirv: vkGetDeviceQueue returned null".to_string());
        }

        // Shader module straight from the caller's words — the whole point
        // of the SPIR-V lingua franca: no compiler in the loop.
        let sinfo = VkShaderModuleCreateInfo {
            s_type: ST_SHADER_MODULE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            code_size: words.len() * 4,
            p_code: words.as_ptr(),
        };
        let r = (f.create_shader_module)(device, &sinfo, std::ptr::null(), &mut run.shader);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkCreateShaderModule rejected the binary (VkResult {r})"
            ));
        }

        // Set layout: two storage buffers (bindings 0/1), compute stage.
        let bindings = [
            VkDescriptorSetLayoutBinding {
                binding: 0,
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                descriptor_count: 1,
                stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                p_immutable_samplers: std::ptr::null(),
            },
            VkDescriptorSetLayoutBinding {
                binding: 1,
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                descriptor_count: 1,
                stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                p_immutable_samplers: std::ptr::null(),
            },
        ];
        let dsl_info = VkDescriptorSetLayoutCreateInfo {
            s_type: ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            binding_count: 2,
            p_bindings: bindings.as_ptr(),
        };
        let r = (f.create_descriptor_set_layout)(device, &dsl_info, std::ptr::null(), &mut run.dsl);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkCreateDescriptorSetLayout failed (VkResult {r})"
            ));
        }
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: ST_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            set_layout_count: 1,
            p_set_layouts: &run.dsl,
            push_constant_range_count: 0,
            p_push_constant_ranges: std::ptr::null(),
        };
        let r = (f.create_pipeline_layout)(device, &pl_info, std::ptr::null(), &mut run.playout);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkCreatePipelineLayout failed (VkResult {r})"
            ));
        }
        let pinfo = VkComputePipelineCreateInfo {
            s_type: ST_COMPUTE_PIPELINE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            stage: VkPipelineShaderStageCreateInfo {
                s_type: ST_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_COMPUTE_BIT,
                module: run.shader,
                p_name: c"main".as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
            layout: run.playout,
            base_pipeline_handle: 0,
            base_pipeline_index: -1,
        };
        let r = (f.create_compute_pipelines)(
            device,
            0,
            1,
            &pinfo,
            std::ptr::null(),
            &mut run.pipeline,
        );
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: pipeline creation failed — does the binary declare a compute \
                 entry point named `main` with storage buffers at set 0, bindings 0/1? \
                 (VkResult {r})"
            ));
        }

        // Buffers: input (copied in) and output (zeroed — the shared
        // contract: all backends must agree on bytes the kernel never
        // wrote). Both stay mapped until teardown.
        let (inbuf, inmem, inptr) = create_host_buffer(f, device, &memprops, input.len() as u64)?;
        run.inbuf = inbuf;
        run.inmem = inmem;
        std::ptr::copy_nonoverlapping(input.as_ptr(), inptr, input.len());
        let (outbuf, outmem, outptr) = create_host_buffer(f, device, &memprops, out_len as u64)?;
        run.outbuf = outbuf;
        run.outmem = outmem;
        std::ptr::write_bytes(outptr, 0, out_len);

        // Descriptor set: pool → set → the two buffer writes.
        let pool_size = VkDescriptorPoolSize {
            ty: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 2,
        };
        let pool_info = VkDescriptorPoolCreateInfo {
            s_type: ST_DESCRIPTOR_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            max_sets: 1,
            pool_size_count: 1,
            p_pool_sizes: &pool_size,
        };
        let r = (f.create_descriptor_pool)(device, &pool_info, std::ptr::null(), &mut run.dpool);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkCreateDescriptorPool failed (VkResult {r})"
            ));
        }
        let set_info = VkDescriptorSetAllocateInfo {
            s_type: ST_DESCRIPTOR_SET_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            descriptor_pool: run.dpool,
            descriptor_set_count: 1,
            p_set_layouts: &run.dsl,
        };
        let mut dset: VkDescriptorSet = 0;
        let r = (f.allocate_descriptor_sets)(device, &set_info, &mut dset);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkAllocateDescriptorSets failed (VkResult {r})"
            ));
        }
        let buf_infos = [
            VkDescriptorBufferInfo {
                buffer: run.inbuf,
                offset: 0,
                range: u64::MAX, // VK_WHOLE_SIZE
            },
            VkDescriptorBufferInfo {
                buffer: run.outbuf,
                offset: 0,
                range: u64::MAX,
            },
        ];
        let writes = [
            VkWriteDescriptorSet {
                s_type: ST_WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset,
                dst_binding: 0,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: &buf_infos[0],
                p_texel_buffer_view: std::ptr::null(),
            },
            VkWriteDescriptorSet {
                s_type: ST_WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset,
                dst_binding: 1,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: &buf_infos[1],
                p_texel_buffer_view: std::ptr::null(),
            },
        ];
        (f.update_descriptor_sets)(device, 2, writes.as_ptr(), 0, std::ptr::null());

        // Record + submit + wait.
        let cp_info = VkCommandPoolCreateInfo {
            s_type: ST_COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            queue_family_index: qfam,
        };
        let r = (f.create_command_pool)(device, &cp_info, std::ptr::null(), &mut run.cpool);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkCreateCommandPool failed (VkResult {r})"
            ));
        }
        let cb_info = VkCommandBufferAllocateInfo {
            s_type: ST_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: run.cpool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut cmd: VkCommandBuffer = std::ptr::null_mut();
        let r = (f.allocate_command_buffers)(device, &cb_info, &mut cmd);
        if r != VK_SUCCESS || cmd.is_null() {
            return Err(format!(
                "gpu.run_spirv: vkAllocateCommandBuffers failed (VkResult {r})"
            ));
        }
        let begin = VkCommandBufferBeginInfo {
            s_type: ST_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        let r = (f.begin_command_buffer)(cmd, &begin);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkBeginCommandBuffer failed (VkResult {r})"
            ));
        }
        (f.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, run.pipeline);
        (f.cmd_bind_descriptor_sets)(
            cmd,
            VK_PIPELINE_BIND_POINT_COMPUTE,
            run.playout,
            0,
            1,
            &dset,
            0,
            std::ptr::null(),
        );
        (f.cmd_dispatch)(cmd, wx, wy, wz);
        let r = (f.end_command_buffer)(cmd);
        if r != VK_SUCCESS {
            return Err(format!(
                "gpu.run_spirv: vkEndCommandBuffer failed (VkResult {r})"
            ));
        }
        let fence_info = VkFenceCreateInfo {
            s_type: ST_FENCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
        };
        let r = (f.create_fence)(device, &fence_info, std::ptr::null(), &mut run.fence);
        if r != VK_SUCCESS {
            return Err(format!("gpu.run_spirv: vkCreateFence failed (VkResult {r})"));
        }
        let submit = VkSubmitInfo {
            s_type: ST_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        let r = (f.queue_submit)(queue, 1, &submit, run.fence);
        if r != VK_SUCCESS {
            return Err(format!("gpu.run_spirv: vkQueueSubmit failed (VkResult {r})"));
        }
        let r = (f.wait_for_fences)(device, 1, &run.fence, VK_TRUE, u64::MAX);
        if r != VK_SUCCESS {
            return Err(format!("gpu.run_spirv: vkWaitForFences failed (VkResult {r})"));
        }

        // Fence signal makes the device writes visible to the host domain
        // (coherent memory, no explicit invalidate needed).
        let mut out = vec![0u8; out_len];
        std::ptr::copy_nonoverlapping(outptr, out.as_mut_ptr(), out_len);
        Ok(out)
    }
}
