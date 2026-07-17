//! Vulkan backend for the `window` namespace on Windows, additive
//! alongside `gl.rs` (OpenGL/WGL) — never a replacement. A single compiled
//! binary can hold either kind of window, or both at once (`--features
//! gl,vulkan`); see `super::Inner`'s two-variant enum. The Windows twin of
//! `x11/vulkan.rs`, ported from its proven Phase-1 machinery with exactly
//! one platform-specific substitution: `VK_KHR_win32_surface` +
//! `vkCreateWin32SurfaceKHR` (hinstance + hwnd) instead of
//! `VK_KHR_xlib_surface` (display + window). Rides the same `vulkan` cargo
//! feature as the compute backend (`crate::vk`, `gpu.run_spirv`) — the
//! loader comes from [`crate::vk::loader_gipa`]
//! (`LoadLibraryA("vulkan-1.dll")` here), zero Cargo dependencies.
//! windows-latest CI runners were probed before this was built: the
//! loader DLL ships with the image (whether a device enumerates behind it
//! is machine-dependent, so every failure path is a clean prefixed `Err`).
//!
//! **This phase: device/swapchain plumbing; clear + present.** The `gfx.*`
//! draw-call surface is a later phase (see `super::Inner`'s
//! `vulkan_gfx_todo` arms), mirroring how the x11 backend grew.
//!
//! # The offscreen stable back buffer (mirrors `x11/vulkan.rs` exactly)
//!
//! Rendering never targets a swapchain image directly. All work lands in
//! an app-owned offscreen `VkImage` (same format as the swapchain,
//! `TRANSFER_SRC|TRANSFER_DST`); `swap_buffers` acquires the frame's
//! swapchain image, image-copies the offscreen target into it, transitions
//! it to `PRESENT_SRC`, and presents. Swapchain images are transient, so
//! the offscreen target *is* the stable back buffer, letting `clear` and
//! future draws happen at any point before `swap_buffers`, exactly like
//! the GL call pattern demos already use.
//!
//! **Format choice is load-bearing** (learned on lavapipe, kept here):
//! drivers may offer an sRGB format first, which would sRGB-encode every
//! clear value (0.5 becomes 188/255) and break GL parity — so the backend
//! prefers a UNORM format explicitly (`B8G8R8A8_UNORM`, then
//! `R8G8B8A8_UNORM`, then whatever is offered).
//!
//! Every submission is synchronous (submit → fence wait) and a fresh
//! command buffer is allocated per operation and freed after — the same
//! choices as the x11 and Metal backends, for the same reasons. Swapchain
//! recreation (window resize, `OUT_OF_DATE`/`SUBOPTIMAL`) rebuilds both
//! the swapchain and the offscreen target from the surface's current
//! extent.
//!
//! The WSI FFI below is deliberately self-contained (the same
//! hand-transcribed shapes as `x11/vulkan.rs`'s own WSI half, which stayed
//! per-consumer when `crate::vk` graduated) — now that a SECOND WSI
//! consumer exists, extracting the genuinely shared swapchain machinery is
//! a candidate follow-up refactor per the extract-at-real-duplication
//! rule; it is not guessed at up front here.

use std::ffi::{c_char, c_void, CString};
use std::ptr;

use super::shared::{GetModuleHandleW, Win32WindowState};
use crate::vk::loader_gipa;

// ---------------------------------------------------------------------------
// Vulkan ABI (x86_64/aarch64 Linux: dispatchable handles are pointers,
// non-dispatchable are u64) — hand-transcribed 1.0 core + VK_KHR_surface /
// VK_KHR_xlib_surface / VK_KHR_swapchain, same sourcing discipline as
// `crate::vk` (exact field widths from the registry's stable 1.0 layouts).
// ---------------------------------------------------------------------------

type VkInstance = *mut c_void;
type VkPhysicalDevice = *mut c_void;
type VkDevice = *mut c_void;
type VkQueue = *mut c_void;
type VkCommandBuffer = *mut c_void;

type VkSurfaceKhr = u64;
type VkSwapchainKhr = u64;
type VkImage = u64;
type VkDeviceMemory = u64;
type VkCommandPool = u64;
type VkFence = u64;

type VkResult = i32;
const VK_SUCCESS: VkResult = 0;
const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;

const VK_API_VERSION_1_0: u32 = 1 << 22;
const VK_TRUE: u32 = 1;
const VK_QUEUE_GRAPHICS_BIT: u32 = 0x1;
const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;
const VK_COMMAND_BUFFER_LEVEL_PRIMARY: i32 = 0;
const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 0x1;
const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 0x1;

// Formats (the UNORM-preference set — see the module docs).
const VK_FORMAT_R8G8B8A8_UNORM: i32 = 37;
const VK_FORMAT_B8G8R8A8_UNORM: i32 = 44;

// Image machinery.
const VK_IMAGE_TYPE_2D: i32 = 1;
const VK_SAMPLE_COUNT_1_BIT: u32 = 0x1;
const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 0x1;
const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 0x2;
const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 0x10;
const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 0x1;

// Layouts.
const VK_IMAGE_LAYOUT_UNDEFINED: i32 = 0;
const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: i32 = 6;
const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: i32 = 7;
const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: i32 = 1000001002;

// Pipeline stages / access (the transfer-only subset Phase 1 needs).
const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 0x1;
const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 0x1000;
const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: u32 = 0x2000;
const VK_ACCESS_TRANSFER_READ_BIT: u32 = 0x800;
const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 0x1000;

// Presentation.
const VK_PRESENT_MODE_FIFO_KHR: i32 = 2;
const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: u32 = 0x1;
const VK_QUEUE_FAMILY_IGNORED: u32 = !0;

// VkStructureType values.
const ST_APPLICATION_INFO: i32 = 0;
const ST_INSTANCE_CREATE_INFO: i32 = 1;
const ST_DEVICE_QUEUE_CREATE_INFO: i32 = 2;
const ST_DEVICE_CREATE_INFO: i32 = 3;
const ST_SUBMIT_INFO: i32 = 4;
const ST_MEMORY_ALLOCATE_INFO: i32 = 5;
const ST_FENCE_CREATE_INFO: i32 = 8;
const ST_IMAGE_CREATE_INFO: i32 = 14;
const ST_COMMAND_POOL_CREATE_INFO: i32 = 39;
const ST_COMMAND_BUFFER_ALLOCATE_INFO: i32 = 40;
const ST_COMMAND_BUFFER_BEGIN_INFO: i32 = 42;
const ST_IMAGE_MEMORY_BARRIER: i32 = 45;
const ST_SWAPCHAIN_CREATE_INFO_KHR: i32 = 1000001000;
const ST_PRESENT_INFO_KHR: i32 = 1000001001;
const ST_WIN32_SURFACE_CREATE_INFO_KHR: i32 = 1000009000;

#[repr(C)]
struct VkApplicationInfo {
    s_type: i32,
    p_next: *const c_void,
    app_name: *const c_char,
    app_version: u32,
    engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}
#[repr(C)]
struct VkInstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    app_info: *const VkApplicationInfo,
    enabled_layer_count: u32,
    enabled_layers: *const *const c_char,
    enabled_extension_count: u32,
    enabled_extensions: *const *const c_char,
}
#[repr(C)]
struct VkWin32SurfaceCreateInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    hinstance: *mut c_void,
    hwnd: *mut c_void,
}
#[repr(C)]
struct VkQueueFamilyProperties {
    queue_flags: u32,
    queue_count: u32,
    timestamp_valid_bits: u32,
    min_image_transfer_granularity: [u32; 3],
}
#[repr(C)]
struct VkExtensionProperties {
    extension_name: [c_char; 256],
    spec_version: u32,
}
#[repr(C)]
struct VkDeviceQueueCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    queue_family_index: u32,
    queue_count: u32,
    queue_priorities: *const f32,
}
#[repr(C)]
struct VkDeviceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    queue_create_info_count: u32,
    queue_create_infos: *const VkDeviceQueueCreateInfo,
    enabled_layer_count: u32,
    enabled_layers: *const *const c_char,
    enabled_extension_count: u32,
    enabled_extensions: *const *const c_char,
    enabled_features: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkExtent2D {
    width: u32,
    height: u32,
}
#[repr(C)]
struct VkSurfaceCapabilitiesKhr {
    min_image_count: u32,
    max_image_count: u32,
    current_extent: VkExtent2D,
    min_image_extent: VkExtent2D,
    max_image_extent: VkExtent2D,
    max_image_array_layers: u32,
    supported_transforms: u32,
    current_transform: u32,
    supported_composite_alpha: u32,
    supported_usage_flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkSurfaceFormatKhr {
    format: i32,
    color_space: i32,
}
#[repr(C)]
struct VkSwapchainCreateInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    surface: VkSurfaceKhr,
    min_image_count: u32,
    image_format: i32,
    image_color_space: i32,
    image_extent: VkExtent2D,
    image_array_layers: u32,
    image_usage: u32,
    image_sharing_mode: i32,
    queue_family_index_count: u32,
    queue_family_indices: *const u32,
    pre_transform: u32,
    composite_alpha: u32,
    present_mode: i32,
    clipped: u32,
    old_swapchain: VkSwapchainKhr,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkExtent3D {
    width: u32,
    height: u32,
    depth: u32,
}
#[repr(C)]
struct VkImageCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    image_type: i32,
    format: i32,
    extent: VkExtent3D,
    mip_levels: u32,
    array_layers: u32,
    samples: u32,
    tiling: i32,
    usage: u32,
    sharing_mode: i32,
    queue_family_index_count: u32,
    queue_family_indices: *const u32,
    initial_layout: i32,
}
#[repr(C)]
struct VkMemoryRequirements {
    size: u64,
    alignment: u64,
    memory_type_bits: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkMemoryType {
    property_flags: u32,
    heap_index: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkMemoryHeap {
    size: u64,
    flags: u32,
}
#[repr(C)]
struct VkPhysicalDeviceMemoryProperties {
    memory_type_count: u32,
    memory_types: [VkMemoryType; 32],
    memory_heap_count: u32,
    memory_heaps: [VkMemoryHeap; 16],
}
#[repr(C)]
struct VkMemoryAllocateInfo {
    s_type: i32,
    p_next: *const c_void,
    allocation_size: u64,
    memory_type_index: u32,
}
#[repr(C)]
struct VkCommandPoolCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    queue_family_index: u32,
}
#[repr(C)]
struct VkCommandBufferAllocateInfo {
    s_type: i32,
    p_next: *const c_void,
    command_pool: VkCommandPool,
    level: i32,
    command_buffer_count: u32,
}
#[repr(C)]
struct VkCommandBufferBeginInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    inheritance_info: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkImageSubresourceRange {
    aspect_mask: u32,
    base_mip_level: u32,
    level_count: u32,
    base_array_layer: u32,
    layer_count: u32,
}
#[repr(C)]
struct VkImageMemoryBarrier {
    s_type: i32,
    p_next: *const c_void,
    src_access_mask: u32,
    dst_access_mask: u32,
    old_layout: i32,
    new_layout: i32,
    src_queue_family_index: u32,
    dst_queue_family_index: u32,
    image: VkImage,
    subresource_range: VkImageSubresourceRange,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkImageSubresourceLayers {
    aspect_mask: u32,
    mip_level: u32,
    base_array_layer: u32,
    layer_count: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkOffset3D {
    x: i32,
    y: i32,
    z: i32,
}
#[repr(C)]
struct VkImageCopy {
    src_subresource: VkImageSubresourceLayers,
    src_offset: VkOffset3D,
    dst_subresource: VkImageSubresourceLayers,
    dst_offset: VkOffset3D,
    extent: VkExtent3D,
}
#[repr(C)]
struct VkFenceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
}
#[repr(C)]
struct VkSubmitInfo {
    s_type: i32,
    p_next: *const c_void,
    wait_semaphore_count: u32,
    wait_semaphores: *const u64,
    wait_dst_stage_mask: *const u32,
    command_buffer_count: u32,
    command_buffers: *const VkCommandBuffer,
    signal_semaphore_count: u32,
    signal_semaphores: *const u64,
}
#[repr(C)]
struct VkPresentInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    wait_semaphore_count: u32,
    wait_semaphores: *const u64,
    swapchain_count: u32,
    swapchains: *const VkSwapchainKhr,
    image_indices: *const u32,
    results: *mut VkResult,
}

// Function-pointer types (extern "system" == extern "C" on these targets,
// matching crate::vk's convention).
type PfnVoidFunction = *mut c_void;
type FnGetInstanceProcAddr =
    unsafe extern "system" fn(VkInstance, *const c_char) -> PfnVoidFunction;
type FnCreateInstance = unsafe extern "system" fn(
    *const VkInstanceCreateInfo,
    *const c_void,
    *mut VkInstance,
) -> VkResult;
type FnDestroyInstance = unsafe extern "system" fn(VkInstance, *const c_void);
type FnEnumeratePhysicalDevices =
    unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> VkResult;
type FnGetPhysicalDeviceQueueFamilyProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
type FnEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    VkPhysicalDevice,
    *const c_char,
    *mut u32,
    *mut VkExtensionProperties,
) -> VkResult;
type FnGetPhysicalDeviceMemoryProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
type FnCreateWin32SurfaceKhr = unsafe extern "system" fn(
    VkInstance,
    *const VkWin32SurfaceCreateInfoKhr,
    *const c_void,
    *mut VkSurfaceKhr,
) -> VkResult;
type FnDestroySurfaceKhr = unsafe extern "system" fn(VkInstance, VkSurfaceKhr, *const c_void);
type FnGetPhysicalDeviceSurfaceSupportKhr =
    unsafe extern "system" fn(VkPhysicalDevice, u32, VkSurfaceKhr, *mut u32) -> VkResult;
type FnGetPhysicalDeviceSurfaceCapabilitiesKhr = unsafe extern "system" fn(
    VkPhysicalDevice,
    VkSurfaceKhr,
    *mut VkSurfaceCapabilitiesKhr,
) -> VkResult;
type FnGetPhysicalDeviceSurfaceFormatsKhr = unsafe extern "system" fn(
    VkPhysicalDevice,
    VkSurfaceKhr,
    *mut u32,
    *mut VkSurfaceFormatKhr,
) -> VkResult;
type FnGetPhysicalDeviceSurfacePresentModesKhr =
    unsafe extern "system" fn(VkPhysicalDevice, VkSurfaceKhr, *mut u32, *mut i32) -> VkResult;
type FnCreateDevice = unsafe extern "system" fn(
    VkPhysicalDevice,
    *const VkDeviceCreateInfo,
    *const c_void,
    *mut VkDevice,
) -> VkResult;
type FnDestroyDevice = unsafe extern "system" fn(VkDevice, *const c_void);
type FnDeviceWaitIdle = unsafe extern "system" fn(VkDevice) -> VkResult;
type FnGetDeviceQueue = unsafe extern "system" fn(VkDevice, u32, u32, *mut VkQueue);
type FnCreateSwapchainKhr = unsafe extern "system" fn(
    VkDevice,
    *const VkSwapchainCreateInfoKhr,
    *const c_void,
    *mut VkSwapchainKhr,
) -> VkResult;
type FnDestroySwapchainKhr = unsafe extern "system" fn(VkDevice, VkSwapchainKhr, *const c_void);
type FnGetSwapchainImagesKhr =
    unsafe extern "system" fn(VkDevice, VkSwapchainKhr, *mut u32, *mut VkImage) -> VkResult;
type FnAcquireNextImageKhr =
    unsafe extern "system" fn(VkDevice, VkSwapchainKhr, u64, u64, VkFence, *mut u32) -> VkResult;
type FnQueuePresentKhr =
    unsafe extern "system" fn(VkQueue, *const VkPresentInfoKhr) -> VkResult;
type FnCreateImage = unsafe extern "system" fn(
    VkDevice,
    *const VkImageCreateInfo,
    *const c_void,
    *mut VkImage,
) -> VkResult;
type FnDestroyImage = unsafe extern "system" fn(VkDevice, VkImage, *const c_void);
type FnGetImageMemoryRequirements =
    unsafe extern "system" fn(VkDevice, VkImage, *mut VkMemoryRequirements);
type FnAllocateMemory = unsafe extern "system" fn(
    VkDevice,
    *const VkMemoryAllocateInfo,
    *const c_void,
    *mut VkDeviceMemory,
) -> VkResult;
type FnFreeMemory = unsafe extern "system" fn(VkDevice, VkDeviceMemory, *const c_void);
type FnBindImageMemory =
    unsafe extern "system" fn(VkDevice, VkImage, VkDeviceMemory, u64) -> VkResult;
type FnCreateCommandPool = unsafe extern "system" fn(
    VkDevice,
    *const VkCommandPoolCreateInfo,
    *const c_void,
    *mut VkCommandPool,
) -> VkResult;
type FnDestroyCommandPool = unsafe extern "system" fn(VkDevice, VkCommandPool, *const c_void);
type FnAllocateCommandBuffers = unsafe extern "system" fn(
    VkDevice,
    *const VkCommandBufferAllocateInfo,
    *mut VkCommandBuffer,
) -> VkResult;
type FnFreeCommandBuffers =
    unsafe extern "system" fn(VkDevice, VkCommandPool, u32, *const VkCommandBuffer);
type FnBeginCommandBuffer =
    unsafe extern "system" fn(VkCommandBuffer, *const VkCommandBufferBeginInfo) -> VkResult;
type FnEndCommandBuffer = unsafe extern "system" fn(VkCommandBuffer) -> VkResult;
type FnCmdPipelineBarrier = unsafe extern "system" fn(
    VkCommandBuffer,
    u32,
    u32,
    u32,
    u32,
    *const c_void,
    u32,
    *const c_void,
    u32,
    *const VkImageMemoryBarrier,
);
type FnCmdClearColorImage = unsafe extern "system" fn(
    VkCommandBuffer,
    VkImage,
    i32,
    *const [f32; 4],
    u32,
    *const VkImageSubresourceRange,
);
type FnCmdCopyImage = unsafe extern "system" fn(
    VkCommandBuffer,
    VkImage,
    i32,
    VkImage,
    i32,
    u32,
    *const VkImageCopy,
);
type FnCreateFence = unsafe extern "system" fn(
    VkDevice,
    *const VkFenceCreateInfo,
    *const c_void,
    *mut VkFence,
) -> VkResult;
type FnDestroyFence = unsafe extern "system" fn(VkDevice, VkFence, *const c_void);
type FnResetFences = unsafe extern "system" fn(VkDevice, u32, *const VkFence) -> VkResult;
type FnWaitForFences =
    unsafe extern "system" fn(VkDevice, u32, *const VkFence, u32, u64) -> VkResult;
type FnQueueSubmit =
    unsafe extern "system" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;


/// Resolve one entry point through `vkGetInstanceProcAddr` or `Err` — used
/// for everything past `vkCreateInstance` (instance-level trampolines
/// dispatch device-level calls correctly; the direct-`vkGetDeviceProcAddr`
/// optimization is a later efficiency-pass item, not a Phase 1 need).
macro_rules! vkload {
    ($gipa:expr, $inst:expr, $name:literal, $ty:ty) => {{
        let cname = CString::new($name).unwrap();
        let p = $gipa($inst, cname.as_ptr());
        if p.is_null() {
            return Err(format!(
                "window.create_vulkan: loader is missing `{}`",
                $name
            ));
        }
        std::mem::transmute::<PfnVoidFunction, $ty>(p)
    }};
}

/// Every entry point the window path uses, resolved once per window right
/// after instance creation. Plain function pointers, `Copy` (the loader is
/// process-permanent, so nothing here needs a `Drop`).
#[derive(Clone, Copy)]
struct Fns {
    destroy_instance: FnDestroyInstance,
    destroy_surface: FnDestroySurfaceKhr,
    get_surface_capabilities: FnGetPhysicalDeviceSurfaceCapabilitiesKhr,
    create_device: FnCreateDevice,
    destroy_device: FnDestroyDevice,
    device_wait_idle: FnDeviceWaitIdle,
    get_device_queue: FnGetDeviceQueue,
    create_swapchain: FnCreateSwapchainKhr,
    destroy_swapchain: FnDestroySwapchainKhr,
    get_swapchain_images: FnGetSwapchainImagesKhr,
    acquire_next_image: FnAcquireNextImageKhr,
    queue_present: FnQueuePresentKhr,
    create_image: FnCreateImage,
    destroy_image: FnDestroyImage,
    get_image_memory_requirements: FnGetImageMemoryRequirements,
    allocate_memory: FnAllocateMemory,
    free_memory: FnFreeMemory,
    bind_image_memory: FnBindImageMemory,
    create_command_pool: FnCreateCommandPool,
    destroy_command_pool: FnDestroyCommandPool,
    allocate_command_buffers: FnAllocateCommandBuffers,
    free_command_buffers: FnFreeCommandBuffers,
    begin_command_buffer: FnBeginCommandBuffer,
    end_command_buffer: FnEndCommandBuffer,
    cmd_pipeline_barrier: FnCmdPipelineBarrier,
    cmd_clear_color_image: FnCmdClearColorImage,
    cmd_copy_image: FnCmdCopyImage,
    create_fence: FnCreateFence,
    destroy_fence: FnDestroyFence,
    reset_fences: FnResetFences,
    wait_for_fences: FnWaitForFences,
    queue_submit: FnQueueSubmit,
}

const SUBRESOURCE_COLOR: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
const SUBRESOURCE_COLOR_LAYERS: VkImageSubresourceLayers = VkImageSubresourceLayers {
    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
    mip_level: 0,
    base_array_layer: 0,
    layer_count: 1,
};

/// The Vulkan half of a `WindowHandle` on Windows — a
/// [`Win32WindowState`] (the window + message pump, composed from
/// `shared.rs`, exactly like `gl.rs`) plus the WSI chain and the offscreen
/// stable back buffer.
pub struct Inner {
    win32: Win32WindowState,
    fns: Fns,
    instance: VkInstance,
    surface: VkSurfaceKhr,
    phys: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    cmd_pool: VkCommandPool,
    fence: VkFence,
    swapchain: VkSwapchainKhr,
    swap_images: Vec<VkImage>,
    format: i32,
    color_space: i32,
    extent: VkExtent2D,
    off_image: VkImage,
    off_memory: VkDeviceMemory,
    /// The offscreen image's current layout, tracked so each operation's
    /// entry barrier states the true `oldLayout` (starts `UNDEFINED`).
    off_layout: i32,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        let Some(gipa) = loader_gipa() else {
            return Err(
                "window.create_vulkan: no Vulkan loader (vulkan-1.dll) on this system"
                    .to_string(),
            );
        };

        // Safety: the Win32 half mirrors gl.rs's create exactly (shared
        // machinery); the Vulkan half is checked call by call with full
        // unwind of everything created before a failure.
        unsafe {
            let win32 = Win32WindowState::create_window("window.create_vulkan", title, w, h)?;
            Self::create_vulkan_chain(gipa, win32)
        }
    }

    /// The instance→surface→device→swapchain→offscreen chain, consuming
    /// `win32` — on any failure everything created so far (the Win32 state
    /// included) is destroyed before returning `Err`.
    unsafe fn create_vulkan_chain(
        gipa: FnGetInstanceProcAddr,
        win32: Win32WindowState,
    ) -> Result<Inner, String> {
        // Instance with the two WSI instance extensions.
        let create_instance = vkload!(gipa, ptr::null_mut(), "vkCreateInstance", FnCreateInstance);
        let app = VkApplicationInfo {
            s_type: ST_APPLICATION_INFO,
            p_next: ptr::null(),
            app_name: ptr::null(),
            app_version: 0,
            engine_name: ptr::null(),
            engine_version: 0,
            api_version: VK_API_VERSION_1_0,
        };
        let ext_surface = CString::new("VK_KHR_surface").unwrap();
        let ext_win32 = CString::new("VK_KHR_win32_surface").unwrap();
        let inst_exts = [ext_surface.as_ptr(), ext_win32.as_ptr()];
        let ici = VkInstanceCreateInfo {
            s_type: ST_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            app_info: &app,
            enabled_layer_count: 0,
            enabled_layers: ptr::null(),
            enabled_extension_count: 2,
            enabled_extensions: inst_exts.as_ptr(),
        };
        let mut instance: VkInstance = ptr::null_mut();
        let r = create_instance(&ici, ptr::null(), &mut instance);
        if r != VK_SUCCESS {
            win32.teardown();
            return Err(format!(
                "window.create_vulkan: vkCreateInstance failed ({r}) — driver lacks \
                 VK_KHR_win32_surface?"
            ));
        }

        // Resolve the full table now that an instance exists. A resolution
        // failure inside `vkload!` returns early — clean up first via this
        // little scope trick: resolve into a closure result so the `?`
        // paths below can unwind uniformly.
        let fns = match Self::resolve_fns(gipa, instance) {
            Ok(f) => f,
            Err(e) => {
                let destroy: FnDestroyInstance = {
                    let cname = CString::new("vkDestroyInstance").unwrap();
                    std::mem::transmute::<PfnVoidFunction, FnDestroyInstance>(gipa(
                        instance,
                        cname.as_ptr(),
                    ))
                };
                destroy(instance, ptr::null());
                win32.teardown();
                return Err(e);
            }
        };

        // From here on, one macro-free manual unwind: track what exists.
        macro_rules! fail {
            ($surface:expr, $device:expr, $msg:expr) => {{
                if $device != ptr::null_mut() as VkDevice {
                    (fns.device_wait_idle)($device);
                    (fns.destroy_device)($device, ptr::null());
                }
                if $surface != 0 {
                    (fns.destroy_surface)(instance, $surface, ptr::null());
                }
                (fns.destroy_instance)(instance, ptr::null());
                win32.teardown();
                return Err($msg);
            }};
        }

        // Surface over the Win32 window.
        let create_win32_surface =
            vkload!(gipa, instance, "vkCreateWin32SurfaceKHR", FnCreateWin32SurfaceKhr);
        let wci = VkWin32SurfaceCreateInfoKhr {
            s_type: ST_WIN32_SURFACE_CREATE_INFO_KHR,
            p_next: ptr::null(),
            flags: 0,
            hinstance: GetModuleHandleW(ptr::null()),
            hwnd: win32.hwnd,
        };
        let mut surface: VkSurfaceKhr = 0;
        let r = create_win32_surface(instance, &wci, ptr::null(), &mut surface);
        if r != VK_SUCCESS {
            fail!(
                0u64,
                ptr::null_mut() as VkDevice,
                format!("window.create_vulkan: vkCreateWin32SurfaceKHR failed ({r})")
            );
        }

        // Physical device: first one with a queue family that is both
        // GRAPHICS-capable and able to present to this surface.
        let enum_phys =
            vkload!(gipa, instance, "vkEnumeratePhysicalDevices", FnEnumeratePhysicalDevices);
        let get_qf_props = vkload!(
            gipa,
            instance,
            "vkGetPhysicalDeviceQueueFamilyProperties",
            FnGetPhysicalDeviceQueueFamilyProperties
        );
        let surface_support = vkload!(
            gipa,
            instance,
            "vkGetPhysicalDeviceSurfaceSupportKHR",
            FnGetPhysicalDeviceSurfaceSupportKhr
        );
        let enum_dev_ext = vkload!(
            gipa,
            instance,
            "vkEnumerateDeviceExtensionProperties",
            FnEnumerateDeviceExtensionProperties
        );
        let mut n: u32 = 0;
        enum_phys(instance, &mut n, ptr::null_mut());
        let mut phys_devices: Vec<VkPhysicalDevice> = vec![ptr::null_mut(); n as usize];
        if n > 0 {
            enum_phys(instance, &mut n, phys_devices.as_mut_ptr());
        }
        let mut picked: Option<(VkPhysicalDevice, u32)> = None;
        'outer: for &pd in &phys_devices {
            let mut qn: u32 = 0;
            get_qf_props(pd, &mut qn, ptr::null_mut());
            let mut qfs: Vec<VkQueueFamilyProperties> = Vec::with_capacity(qn as usize);
            get_qf_props(pd, &mut qn, qfs.as_mut_ptr());
            qfs.set_len(qn as usize);
            for (i, qf) in qfs.iter().enumerate() {
                if qf.queue_flags & VK_QUEUE_GRAPHICS_BIT == 0 {
                    continue;
                }
                let mut supported: u32 = 0;
                if surface_support(pd, i as u32, surface, &mut supported) == VK_SUCCESS
                    && supported == VK_TRUE
                {
                    // Also require VK_KHR_swapchain on the device.
                    let mut en: u32 = 0;
                    enum_dev_ext(pd, ptr::null(), &mut en, ptr::null_mut());
                    let mut eps: Vec<VkExtensionProperties> = Vec::with_capacity(en as usize);
                    enum_dev_ext(pd, ptr::null(), &mut en, eps.as_mut_ptr());
                    eps.set_len(en as usize);
                    let has_swapchain = eps.iter().any(|e| {
                        std::ffi::CStr::from_ptr(e.extension_name.as_ptr())
                            .to_bytes()
                            .eq(b"VK_KHR_swapchain")
                    });
                    if has_swapchain {
                        picked = Some((pd, i as u32));
                        break 'outer;
                    }
                }
            }
        }
        let Some((phys, qfi)) = picked else {
            fail!(
                surface,
                ptr::null_mut() as VkDevice,
                "window.create_vulkan: no Vulkan device can present to a Win32 surface \
                 (need a graphics queue with surface support and VK_KHR_swapchain)"
                    .to_string()
            );
        };

        // Logical device (one graphics+present queue) + VK_KHR_swapchain.
        let prio = 1.0f32;
        let qci = VkDeviceQueueCreateInfo {
            s_type: ST_DEVICE_QUEUE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            queue_family_index: qfi,
            queue_count: 1,
            queue_priorities: &prio,
        };
        let ext_swapchain = CString::new("VK_KHR_swapchain").unwrap();
        let dev_exts = [ext_swapchain.as_ptr()];
        let dci = VkDeviceCreateInfo {
            s_type: ST_DEVICE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            queue_create_info_count: 1,
            queue_create_infos: &qci,
            enabled_layer_count: 0,
            enabled_layers: ptr::null(),
            enabled_extension_count: 1,
            enabled_extensions: dev_exts.as_ptr(),
            enabled_features: ptr::null(),
        };
        let mut device: VkDevice = ptr::null_mut();
        let r = (fns.create_device)(phys, &dci, ptr::null(), &mut device);
        if r != VK_SUCCESS {
            fail!(
                surface,
                ptr::null_mut() as VkDevice,
                format!("window.create_vulkan: vkCreateDevice failed ({r})")
            );
        }
        let mut queue: VkQueue = ptr::null_mut();
        (fns.get_device_queue)(device, qfi, 0, &mut queue);

        // Command pool + submission fence (reused for every synchronous
        // submit; command buffers themselves are allocated per operation).
        let pci = VkCommandPoolCreateInfo {
            s_type: ST_COMMAND_POOL_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            queue_family_index: qfi,
        };
        let mut cmd_pool: VkCommandPool = 0;
        let r = (fns.create_command_pool)(device, &pci, ptr::null(), &mut cmd_pool);
        if r != VK_SUCCESS {
            fail!(
                surface,
                device,
                format!("window.create_vulkan: vkCreateCommandPool failed ({r})")
            );
        }
        let fci = VkFenceCreateInfo {
            s_type: ST_FENCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
        };
        let mut fence: VkFence = 0;
        let r = (fns.create_fence)(device, &fci, ptr::null(), &mut fence);
        if r != VK_SUCCESS {
            (fns.destroy_command_pool)(device, cmd_pool, ptr::null());
            fail!(
                surface,
                device,
                format!("window.create_vulkan: vkCreateFence failed ({r})")
            );
        }

        let mut inner = Inner {
            win32,
            fns,
            instance,
            surface,
            phys,
            device,
            queue,
            cmd_pool,
            fence,
            swapchain: 0,
            swap_images: Vec::new(),
            format: 0,
            color_space: 0,
            extent: VkExtent2D {
                width: 0,
                height: 0,
            },
            off_image: 0,
            off_memory: 0,
            off_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };

        // Pick the surface format once (UNORM preference — see module
        // docs), then build the swapchain + offscreen pair.
        if let Err(e) = inner.pick_surface_format(gipa) {
            inner.destroy_vulkan();
            return Err(e);
        }
        if let Err(e) = inner.rebuild_swapchain_and_offscreen() {
            inner.destroy_vulkan();
            return Err(e);
        }
        inner.win32.show();
        Ok(inner)
    }

    unsafe fn resolve_fns(gipa: FnGetInstanceProcAddr, instance: VkInstance) -> Result<Fns, String> {
        Ok(Fns {
            destroy_instance: vkload!(gipa, instance, "vkDestroyInstance", FnDestroyInstance),
            destroy_surface: vkload!(gipa, instance, "vkDestroySurfaceKHR", FnDestroySurfaceKhr),
            get_surface_capabilities: vkload!(
                gipa,
                instance,
                "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
                FnGetPhysicalDeviceSurfaceCapabilitiesKhr
            ),
            create_device: vkload!(gipa, instance, "vkCreateDevice", FnCreateDevice),
            destroy_device: vkload!(gipa, instance, "vkDestroyDevice", FnDestroyDevice),
            device_wait_idle: vkload!(gipa, instance, "vkDeviceWaitIdle", FnDeviceWaitIdle),
            get_device_queue: vkload!(gipa, instance, "vkGetDeviceQueue", FnGetDeviceQueue),
            create_swapchain: vkload!(gipa, instance, "vkCreateSwapchainKHR", FnCreateSwapchainKhr),
            destroy_swapchain: vkload!(
                gipa,
                instance,
                "vkDestroySwapchainKHR",
                FnDestroySwapchainKhr
            ),
            get_swapchain_images: vkload!(
                gipa,
                instance,
                "vkGetSwapchainImagesKHR",
                FnGetSwapchainImagesKhr
            ),
            acquire_next_image: vkload!(
                gipa,
                instance,
                "vkAcquireNextImageKHR",
                FnAcquireNextImageKhr
            ),
            queue_present: vkload!(gipa, instance, "vkQueuePresentKHR", FnQueuePresentKhr),
            create_image: vkload!(gipa, instance, "vkCreateImage", FnCreateImage),
            destroy_image: vkload!(gipa, instance, "vkDestroyImage", FnDestroyImage),
            get_image_memory_requirements: vkload!(
                gipa,
                instance,
                "vkGetImageMemoryRequirements",
                FnGetImageMemoryRequirements
            ),
            allocate_memory: vkload!(gipa, instance, "vkAllocateMemory", FnAllocateMemory),
            free_memory: vkload!(gipa, instance, "vkFreeMemory", FnFreeMemory),
            bind_image_memory: vkload!(gipa, instance, "vkBindImageMemory", FnBindImageMemory),
            create_command_pool: vkload!(
                gipa,
                instance,
                "vkCreateCommandPool",
                FnCreateCommandPool
            ),
            destroy_command_pool: vkload!(
                gipa,
                instance,
                "vkDestroyCommandPool",
                FnDestroyCommandPool
            ),
            allocate_command_buffers: vkload!(
                gipa,
                instance,
                "vkAllocateCommandBuffers",
                FnAllocateCommandBuffers
            ),
            free_command_buffers: vkload!(
                gipa,
                instance,
                "vkFreeCommandBuffers",
                FnFreeCommandBuffers
            ),
            begin_command_buffer: vkload!(
                gipa,
                instance,
                "vkBeginCommandBuffer",
                FnBeginCommandBuffer
            ),
            end_command_buffer: vkload!(gipa, instance, "vkEndCommandBuffer", FnEndCommandBuffer),
            cmd_pipeline_barrier: vkload!(
                gipa,
                instance,
                "vkCmdPipelineBarrier",
                FnCmdPipelineBarrier
            ),
            cmd_clear_color_image: vkload!(
                gipa,
                instance,
                "vkCmdClearColorImage",
                FnCmdClearColorImage
            ),
            cmd_copy_image: vkload!(gipa, instance, "vkCmdCopyImage", FnCmdCopyImage),
            create_fence: vkload!(gipa, instance, "vkCreateFence", FnCreateFence),
            destroy_fence: vkload!(gipa, instance, "vkDestroyFence", FnDestroyFence),
            reset_fences: vkload!(gipa, instance, "vkResetFences", FnResetFences),
            wait_for_fences: vkload!(gipa, instance, "vkWaitForFences", FnWaitForFences),
            queue_submit: vkload!(gipa, instance, "vkQueueSubmit", FnQueueSubmit),
        })
    }

    /// Choose the swapchain format once at create time: prefer a UNORM
    /// format so clear values stay linear (see the module docs for the
    /// empirically-verified sRGB trap), falling back to whatever the
    /// surface offers first.
    unsafe fn pick_surface_format(&mut self, gipa: FnGetInstanceProcAddr) -> Result<(), String> {
        let get_formats = vkload!(
            gipa,
            self.instance,
            "vkGetPhysicalDeviceSurfaceFormatsKHR",
            FnGetPhysicalDeviceSurfaceFormatsKhr
        );
        let get_modes = vkload!(
            gipa,
            self.instance,
            "vkGetPhysicalDeviceSurfacePresentModesKHR",
            FnGetPhysicalDeviceSurfacePresentModesKhr
        );
        let mut fc: u32 = 0;
        get_formats(self.phys, self.surface, &mut fc, ptr::null_mut());
        if fc == 0 {
            return Err("window.create_vulkan: surface offers no formats".to_string());
        }
        let mut formats: Vec<VkSurfaceFormatKhr> = Vec::with_capacity(fc as usize);
        get_formats(self.phys, self.surface, &mut fc, formats.as_mut_ptr());
        formats.set_len(fc as usize);
        let chosen = formats
            .iter()
            .find(|f| f.format == VK_FORMAT_B8G8R8A8_UNORM)
            .or_else(|| formats.iter().find(|f| f.format == VK_FORMAT_R8G8B8A8_UNORM))
            .copied()
            .unwrap_or(formats[0]);
        self.format = chosen.format;
        self.color_space = chosen.color_space;

        // FIFO is the one present mode the spec requires every surface to
        // support — confirm rather than assume, since this backend pins it.
        let mut mc: u32 = 0;
        get_modes(self.phys, self.surface, &mut mc, ptr::null_mut());
        let mut modes = vec![0i32; mc as usize];
        get_modes(self.phys, self.surface, &mut mc, modes.as_mut_ptr());
        if !modes.contains(&VK_PRESENT_MODE_FIFO_KHR) {
            return Err("window.create_vulkan: surface does not offer FIFO presentation".to_string());
        }
        Ok(())
    }

    /// (Re)build the swapchain from the surface's *current* extent, plus
    /// the offscreen back buffer at the same size — called at create time
    /// and again whenever presentation reports `OUT_OF_DATE`/`SUBOPTIMAL`
    /// (window resize). Destroys the previous pair first, after a
    /// device-idle wait.
    unsafe fn rebuild_swapchain_and_offscreen(&mut self) -> Result<(), String> {
        (self.fns.device_wait_idle)(self.device);
        if self.swapchain != 0 {
            (self.fns.destroy_swapchain)(self.device, self.swapchain, ptr::null());
            self.swapchain = 0;
            self.swap_images.clear();
        }
        if self.off_image != 0 {
            (self.fns.destroy_image)(self.device, self.off_image, ptr::null());
            self.off_image = 0;
        }
        if self.off_memory != 0 {
            (self.fns.free_memory)(self.device, self.off_memory, ptr::null());
            self.off_memory = 0;
        }
        self.off_layout = VK_IMAGE_LAYOUT_UNDEFINED;

        let mut caps: VkSurfaceCapabilitiesKhr = std::mem::zeroed();
        let r = (self.fns.get_surface_capabilities)(self.phys, self.surface, &mut caps);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkGetPhysicalDeviceSurfaceCapabilitiesKHR failed ({r})"
            ));
        }
        // 0xFFFFFFFF means "the surface takes the swapchain's size" — use
        // the live X window dimensions then; otherwise the surface dictates.
        let extent = if caps.current_extent.width == u32::MAX {
            VkExtent2D {
                width: self.win32.width.max(1) as u32,
                height: self.win32.height.max(1) as u32,
            }
        } else {
            caps.current_extent
        };
        let sci = VkSwapchainCreateInfoKhr {
            s_type: ST_SWAPCHAIN_CREATE_INFO_KHR,
            p_next: ptr::null(),
            flags: 0,
            surface: self.surface,
            min_image_count: caps.min_image_count.max(2),
            image_format: self.format,
            image_color_space: self.color_space,
            image_extent: extent,
            image_array_layers: 1,
            image_usage: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
            image_sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            queue_family_indices: ptr::null(),
            pre_transform: caps.current_transform,
            composite_alpha: VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
            present_mode: VK_PRESENT_MODE_FIFO_KHR,
            clipped: VK_TRUE,
            old_swapchain: 0,
        };
        let mut swapchain: VkSwapchainKhr = 0;
        let r = (self.fns.create_swapchain)(self.device, &sci, ptr::null(), &mut swapchain);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkCreateSwapchainKHR failed ({r})"));
        }
        self.swapchain = swapchain;
        let mut ic: u32 = 0;
        (self.fns.get_swapchain_images)(self.device, swapchain, &mut ic, ptr::null_mut());
        self.swap_images = vec![0; ic as usize];
        (self.fns.get_swapchain_images)(
            self.device,
            swapchain,
            &mut ic,
            self.swap_images.as_mut_ptr(),
        );
        self.extent = extent;

        // Offscreen stable back buffer, same format/extent as the
        // swapchain so `swap_buffers`'s vkCmdCopyImage is a raw 1:1 copy.
        let ici = VkImageCreateInfo {
            s_type: ST_IMAGE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            image_type: VK_IMAGE_TYPE_2D,
            format: self.format,
            extent: VkExtent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT
                | VK_IMAGE_USAGE_TRANSFER_DST_BIT
                | VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            queue_family_indices: ptr::null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image: VkImage = 0;
        let r = (self.fns.create_image)(self.device, &ici, ptr::null(), &mut image);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkCreateImage (offscreen) failed ({r})"));
        }
        self.off_image = image;
        let mut req: VkMemoryRequirements = std::mem::zeroed();
        (self.fns.get_image_memory_requirements)(self.device, image, &mut req);

        // Memory type: any allowed type, preferring DEVICE_LOCAL (on
        // lavapipe everything is host memory anyway; on a discrete GPU
        // this keeps the render target where rendering wants it).
        let gipa = loader_gipa().unwrap();
        let get_mem_props = vkload!(
            gipa,
            self.instance,
            "vkGetPhysicalDeviceMemoryProperties",
            FnGetPhysicalDeviceMemoryProperties
        );
        let mut props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
        get_mem_props(self.phys, &mut props);
        let pick = |want: u32| {
            (0..props.memory_type_count as usize).find(|&i| {
                req.memory_type_bits & (1 << i) != 0
                    && props.memory_types[i].property_flags & want == want
            })
        };
        let Some(type_index) = pick(VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT).or_else(|| pick(0)) else {
            return Err("window.create_vulkan: no memory type for the offscreen image".to_string());
        };
        let mai = VkMemoryAllocateInfo {
            s_type: ST_MEMORY_ALLOCATE_INFO,
            p_next: ptr::null(),
            allocation_size: req.size,
            memory_type_index: type_index as u32,
        };
        let mut memory: VkDeviceMemory = 0;
        let r = (self.fns.allocate_memory)(self.device, &mai, ptr::null(), &mut memory);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkAllocateMemory (offscreen) failed ({r})"));
        }
        self.off_memory = memory;
        let r = (self.fns.bind_image_memory)(self.device, image, memory, 0);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkBindImageMemory failed ({r})"));
        }
        Ok(())
    }

    /// Allocate a one-shot primary command buffer, record into it via
    /// `record`, submit it, wait on the shared fence, and free it — the
    /// synchronous per-operation shape every Phase 1 operation uses
    /// (mirroring Metal's per-operation command buffer + wait).
    unsafe fn one_shot(&mut self, record: impl FnOnce(&Fns, VkCommandBuffer)) -> bool {
        let cai = VkCommandBufferAllocateInfo {
            s_type: ST_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: ptr::null(),
            command_pool: self.cmd_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut cmd: VkCommandBuffer = ptr::null_mut();
        if (self.fns.allocate_command_buffers)(self.device, &cai, &mut cmd) != VK_SUCCESS {
            return false;
        }
        let cbi = VkCommandBufferBeginInfo {
            s_type: ST_COMMAND_BUFFER_BEGIN_INFO,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            inheritance_info: ptr::null(),
        };
        (self.fns.begin_command_buffer)(cmd, &cbi);
        record(&self.fns, cmd);
        (self.fns.end_command_buffer)(cmd);
        let si = VkSubmitInfo {
            s_type: ST_SUBMIT_INFO,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            wait_semaphores: ptr::null(),
            wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            command_buffers: &cmd,
            signal_semaphore_count: 0,
            signal_semaphores: ptr::null(),
        };
        let ok = (self.fns.queue_submit)(self.queue, 1, &si, self.fence) == VK_SUCCESS;
        if ok {
            (self.fns.wait_for_fences)(self.device, 1, &self.fence, VK_TRUE, u64::MAX);
            (self.fns.reset_fences)(self.device, 1, &self.fence);
        }
        (self.fns.free_command_buffers)(self.device, self.cmd_pool, 1, &cmd);
        ok
    }

    /// One image-layout transition, recorded into `cmd`. Nine parameters
    /// because a Vulkan barrier genuinely has nine degrees of freedom —
    /// this mirrors `vkCmdPipelineBarrier`'s own shape rather than
    /// inventing a config struct for four call sites.
    #[allow(clippy::too_many_arguments)]
    unsafe fn barrier(
        fns: &Fns,
        cmd: VkCommandBuffer,
        image: VkImage,
        old_layout: i32,
        new_layout: i32,
        src_access: u32,
        dst_access: u32,
        src_stage: u32,
        dst_stage: u32,
    ) {
        let b = VkImageMemoryBarrier {
            s_type: ST_IMAGE_MEMORY_BARRIER,
            p_next: ptr::null(),
            src_access_mask: src_access,
            dst_access_mask: dst_access,
            old_layout,
            new_layout,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: SUBRESOURCE_COLOR,
        };
        (fns.cmd_pipeline_barrier)(
            cmd,
            src_stage,
            dst_stage,
            0,
            0,
            ptr::null(),
            0,
            ptr::null(),
            1,
            &b,
        );
    }

    pub fn poll(&mut self) {
        self.win32.poll();
    }

    pub fn key_down(&self, name: &str) -> bool {
        self.win32.key_down(name)
    }

    pub fn mouse(&self) -> (f64, f64) {
        self.win32.mouse
    }
    pub fn width(&self) -> i32 {
        self.win32.width
    }
    pub fn height(&self) -> i32 {
        self.win32.height
    }
    pub fn should_close(&self) -> bool {
        self.win32.should_close
    }

    /// This half is always Vulkan — `super::Inner`'s GL variant reports
    /// its own name.
    pub fn backend_name(&self) -> &'static str {
        "vulkan"
    }

    /// No-op on Vulkan (there is no thread-bound "current context" to
    /// assert the way GLX/CGL need) — exists so `win.make_current()` keeps
    /// its cross-backend meaning: "make this the window `gfx.*` targets"
    /// (the VM-level current-window registration happens in `natives.rs`,
    /// backend-independently).
    pub fn make_current(&mut self) {}

    /// `vkCmdClearColorImage` into the offscreen back buffer — visible
    /// after the next `swap_buffers`, exactly like GL's clear-then-swap.
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // The clear value is linear RGBA regardless of the image's channel
        // order (B8G8R8A8 vs R8G8B8A8) — Vulkan swizzles per-format, so no
        // component shuffle happens here.
        let color = [r, g, b, a];
        let off_image = self.off_image;
        let old_layout = self.off_layout;
        // Safety: images/layouts tracked by this struct; the one-shot
        // helper owns submission and synchronization.
        unsafe {
            self.one_shot(|fns, cmd| {
                Self::barrier(
                    fns,
                    cmd,
                    off_image,
                    old_layout,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    0,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                );
                (fns.cmd_clear_color_image)(
                    cmd,
                    off_image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    &color,
                    1,
                    &SUBRESOURCE_COLOR,
                );
            });
        }
        self.off_layout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    }

    pub fn swap_buffers(&mut self) {
        // Never presented into a zero-sized surface (minimized window).
        if self.extent.width == 0 || self.extent.height == 0 {
            return;
        }
        // Safety: the WSI dance verified against lavapipe: fence-synced
        // acquire, offscreen→swapchain copy under explicit layout
        // transitions, PRESENT_SRC handoff, synchronous submit, present.
        unsafe {
            let mut idx: u32 = 0;
            let r = (self.fns.acquire_next_image)(
                self.device,
                self.swapchain,
                u64::MAX,
                0,
                self.fence,
                &mut idx,
            );
            if r == VK_ERROR_OUT_OF_DATE_KHR {
                // Window resized under us — rebuild and skip this frame
                // (the next swap presents at the new size).
                let _ = self.rebuild_swapchain_and_offscreen();
                return;
            }
            if r != VK_SUCCESS && r != VK_SUBOPTIMAL_KHR {
                return;
            }
            (self.fns.wait_for_fences)(self.device, 1, &self.fence, VK_TRUE, u64::MAX);
            (self.fns.reset_fences)(self.device, 1, &self.fence);

            let swap_image = self.swap_images[idx as usize];
            let off_image = self.off_image;
            let off_layout = self.off_layout;
            let extent = self.extent;
            let copy = VkImageCopy {
                src_subresource: SUBRESOURCE_COLOR_LAYERS,
                src_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                dst_subresource: SUBRESOURCE_COLOR_LAYERS,
                dst_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                extent: VkExtent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                },
            };
            let ok = self.one_shot(|fns, cmd| {
                Self::barrier(
                    fns,
                    cmd,
                    off_image,
                    off_layout,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                );
                Self::barrier(
                    fns,
                    cmd,
                    swap_image,
                    VK_IMAGE_LAYOUT_UNDEFINED,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    0,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                );
                (fns.cmd_copy_image)(
                    cmd,
                    off_image,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    swap_image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    1,
                    &copy,
                );
                Self::barrier(
                    fns,
                    cmd,
                    swap_image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    0,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                );
            });
            self.off_layout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
            if !ok {
                return;
            }

            let pi = VkPresentInfoKhr {
                s_type: ST_PRESENT_INFO_KHR,
                p_next: ptr::null(),
                wait_semaphore_count: 0,
                wait_semaphores: ptr::null(),
                swapchain_count: 1,
                swapchains: &self.swapchain,
                image_indices: &idx,
                results: ptr::null_mut(),
            };
            let r = (self.fns.queue_present)(self.queue, &pi);
            if r == VK_ERROR_OUT_OF_DATE_KHR || r == VK_SUBOPTIMAL_KHR {
                let _ = self.rebuild_swapchain_and_offscreen();
            }
        }
    }

    /// The Vulkan half of teardown, reverse creation order after a
    /// device-idle wait; callable on a partially-built `Inner` (every
    /// handle checked). Consumes nothing so `create_vulkan_chain`'s error
    /// paths can call it before also tearing down the X state.
    unsafe fn destroy_vulkan(&mut self) {
        if !self.device.is_null() {
            (self.fns.device_wait_idle)(self.device);
            if self.off_image != 0 {
                (self.fns.destroy_image)(self.device, self.off_image, ptr::null());
            }
            if self.off_memory != 0 {
                (self.fns.free_memory)(self.device, self.off_memory, ptr::null());
            }
            if self.swapchain != 0 {
                (self.fns.destroy_swapchain)(self.device, self.swapchain, ptr::null());
            }
            if self.fence != 0 {
                (self.fns.destroy_fence)(self.device, self.fence, ptr::null());
            }
            if self.cmd_pool != 0 {
                (self.fns.destroy_command_pool)(self.device, self.cmd_pool, ptr::null());
            }
            (self.fns.destroy_device)(self.device, ptr::null());
            self.device = ptr::null_mut();
        }
        if self.surface != 0 {
            (self.fns.destroy_surface)(self.instance, self.surface, ptr::null());
            self.surface = 0;
        }
        if !self.instance.is_null() {
            (self.fns.destroy_instance)(self.instance, ptr::null());
            self.instance = ptr::null_mut();
        }
    }

    /// Idempotent-by-construction teardown (consumes `self`): Vulkan chain
    /// in reverse creation order, then the Win32 half — the same split
    /// `gl.rs`'s teardown has (context first, then
    /// [`Win32WindowState::teardown`]).
    pub fn teardown(mut self) {
        // Safety: every handle was produced by the matching create call in
        // `create_vulkan_chain` and destroyed exactly once; the surface
        // must outlive the swapchain, the instance the surface, and the X
        // window all of them — the orders below respect that.
        unsafe {
            self.destroy_vulkan();
        }
        self.win32.teardown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke: create a Vulkan window (loader → instance →
    /// surface → device → swapchain → offscreen), clear, present, pump
    /// events, tear down. Skips gracefully when anything in that chain is
    /// unavailable (no loader, no device — headless CI machines vary), so
    /// every build shape is deterministic; on a Vulkan-capable Windows
    /// machine this exercises the whole pipe for real. Pixel ground truth
    /// (the x11 backend's XGetImage roundtrip) has no portable Win32
    /// equivalent worth hand-rolling here — the gfx phase's `read_pixels`
    /// becomes the byte-exact gate on this platform, as it did on macOS.
    #[test]
    fn create_clear_present_smoke() {
        let mut inner = match Inner::create("fable vulkan window test", 320, 240) {
            Ok(inner) => inner,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        assert_eq!(inner.width(), 320);
        assert_eq!(inner.height(), 240);
        inner.clear(1.0, 0.5, 0.0, 1.0);
        inner.swap_buffers();
        inner.poll();
        assert!(!inner.should_close());
        inner.teardown();
    }
}
