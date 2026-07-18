//! The shared Vulkan windowing core. Every platform's Vulkan window
//! backend (`x11/vulkan.rs` on Linux/X11, `win32/vulkan.rs` on Windows) is
//! a thin shim over the [`Chain`] here: the platform owns its native
//! window plus the `VkSurfaceKHR` over it, and everything downstream of
//! the surface — device pick, swapchain, the offscreen stable back
//! buffer, clear/present, and the whole `gfx.*` draw-call surface — lives
//! here, compiled from one source for every platform. The lavapipe CI
//! pixel asserts that gate the X11 backend therefore prove the exact code
//! the Windows backend runs; only each shim's few dozen lines (native
//! window creation, `vkCreate*SurfaceKHR`) are platform-specific. The
//! parameterization is deliberately minimal — the surface instance-
//! extension name, a surface-create closure, and the live window size for
//! the extent fallback — because that is the entire platform-specific
//! residue Vulkan WSI actually has.
//!
//! # The offscreen stable back buffer (mirrors `macos/metal.rs` exactly)
//!
//! Rendering never targets a swapchain image directly. All work lands in
//! an app-owned offscreen `VkImage` (same format as the swapchain,
//! `TRANSFER_SRC|TRANSFER_DST`); `swap_buffers` acquires the frame's
//! swapchain image, image-copies the offscreen target into it, transitions
//! it to `PRESENT_SRC`, and presents. Same load-bearing reasons as Metal's
//! offscreen indirection:
//! - Swapchain images are transient (`vkAcquireNextImageKHR` hands back a
//!   rotating member of a small pool — 3 on lavapipe), so "the back
//!   buffer" has no stable identity across frames the way a GL default
//!   framebuffer does. The offscreen target *is* the stable back buffer,
//!   letting `clear`/draws happen at any point before `swap_buffers`,
//!   exactly like the GL call pattern demos already use.
//! - `read_pixels` needs a CPU-readable source it owns; a presented
//!   swapchain image isn't reliably that.
//!
//! **Format choice is load-bearing**: lavapipe offers `B8G8R8A8_SRGB`
//! *first* and `B8G8R8A8_UNORM` second — picking the first format would
//! sRGB-encode every clear value (0.5 becomes 188/255, not 128/255) and
//! break GL parity, so the backend prefers a UNORM format explicitly
//! (`B8G8R8A8_UNORM`, then `R8G8B8A8_UNORM`, then whatever is offered).
//! Verified empirically: with UNORM chosen, the presented pixel read back
//! out of the X window is exactly the linear clear value.
//!
//! Every submission is synchronous (submit → fence wait), matching both
//! the Metal backend's `waitUntilCompleted` and GL's effective semantics
//! for this namespace's simple frame loop; a fresh command buffer is
//! allocated per operation and freed after, mirroring Metal's
//! per-operation command buffers. Swapchain recreation (window resize,
//! `OUT_OF_DATE`/`SUBOPTIMAL`) rebuilds both the swapchain and the
//! offscreen target from the surface's current extent.
//!
//! The 1.0-core primitives (loader, handle types, shared constants/
//! structs/function-pointer types) come from [`crate::vk`], the crate's
//! shared Vulkan layer — one `dlopen` per process across the compute and
//! window paths. What is WSI/image/draw-specific (surface, swapchain,
//! present, image transitions and copies, pipelines and descriptors) is
//! transcribed here, shared by the platform shims.


use std::ffi::{c_char, c_void, CString};
use std::ptr;

use crate::vk::{
    loader_gipa, FnAllocateCommandBuffers, FnAllocateDescriptorSets, FnAllocateMemory,
    FnBeginCommandBuffer, FnBindBufferMemory, FnCmdBindDescriptorSets, FnCmdBindPipeline,
    FnCreateBuffer, FnCreateCommandPool, FnCreateDescriptorPool, FnCreateDescriptorSetLayout,
    FnCreateDevice, FnCreateFence, FnCreateInstance, FnCreatePipelineLayout, FnCreateShaderModule,
    FnDestroyBuffer, FnDestroyCommandPool, FnDestroyDescriptorPool, FnDestroyDescriptorSetLayout,
    FnDestroyDevice, FnDestroyFence, FnDestroyInstance, FnDestroyPipeline, FnDestroyPipelineLayout,
    FnDestroyShaderModule, FnEndCommandBuffer, FnEnumeratePhysicalDevices, FnFreeMemory,
    FnGetBufferMemoryRequirements, FnGetDeviceQueue, FnGetInstanceProcAddr,
    FnGetPhysicalDeviceMemoryProperties, FnGetPhysicalDeviceQueueFamilyProperties, FnMapMemory,
    FnQueueSubmit, FnUpdateDescriptorSets, FnWaitForFences, PfnVoidFunction, VkApplicationInfo,
    VkBuffer, VkBufferCreateInfo, VkCommandBuffer, VkCommandBufferAllocateInfo,
    VkCommandBufferBeginInfo, VkCommandPool, VkCommandPoolCreateInfo, VkDescriptorBufferInfo,
    VkDescriptorPool, VkDescriptorPoolCreateInfo, VkDescriptorPoolSize, VkDescriptorSet,
    VkDescriptorSetAllocateInfo, VkDescriptorSetLayout, VkDescriptorSetLayoutBinding,
    VkDescriptorSetLayoutCreateInfo, VkDevice, VkDeviceCreateInfo, VkDeviceMemory,
    VkDeviceQueueCreateInfo, VkFence, VkFenceCreateInfo, VkInstance, VkInstanceCreateInfo,
    VkMemoryAllocateInfo, VkMemoryRequirements, VkPhysicalDevice, VkPhysicalDeviceMemoryProperties,
    VkPipeline, VkPipelineLayout, VkPipelineLayoutCreateInfo, VkPipelineShaderStageCreateInfo,
    VkQueue, VkQueueFamilyProperties, VkResult, VkShaderModule, VkShaderModuleCreateInfo,
    VkSubmitInfo, VkWriteDescriptorSet, ST_APPLICATION_INFO, ST_BUFFER_CREATE_INFO,
    ST_COMMAND_BUFFER_ALLOCATE_INFO, ST_COMMAND_BUFFER_BEGIN_INFO, ST_COMMAND_POOL_CREATE_INFO,
    ST_DESCRIPTOR_POOL_CREATE_INFO, ST_DESCRIPTOR_SET_ALLOCATE_INFO,
    ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, ST_DEVICE_CREATE_INFO, ST_DEVICE_QUEUE_CREATE_INFO,
    ST_FENCE_CREATE_INFO, ST_INSTANCE_CREATE_INFO, ST_MEMORY_ALLOCATE_INFO,
    ST_PIPELINE_LAYOUT_CREATE_INFO, ST_PIPELINE_SHADER_STAGE_CREATE_INFO,
    ST_SHADER_MODULE_CREATE_INFO, ST_SUBMIT_INFO, ST_WRITE_DESCRIPTOR_SET, VK_API_VERSION_1_0,
    VK_COMMAND_BUFFER_LEVEL_PRIMARY, VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
    VK_MEMORY_PROPERTY_HOST_COHERENT_BIT, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT,
    VK_SHARING_MODE_EXCLUSIVE, VK_SUCCESS, VK_TRUE,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// WSI + image machinery (VK_KHR_surface / VK_KHR_swapchain, plus the
// 1.0-core image/barrier/copy subset only the window path uses) — same
// transcription discipline as `crate::vk`. Everything 1.0-core that the
// compute path shares is imported from `crate::vk` above instead; each
// platform's surface extension (create-info struct + entry point) lives
// with its shim.
// ---------------------------------------------------------------------------

pub(crate) type VkSurfaceKhr = u64;
type VkSwapchainKhr = u64;
type VkImage = u64;

const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;

const VK_QUEUE_GRAPHICS_BIT: u32 = 0x1;
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
const ST_IMAGE_CREATE_INFO: i32 = 14;
const ST_IMAGE_MEMORY_BARRIER: i32 = 45;
const ST_SWAPCHAIN_CREATE_INFO_KHR: i32 = 1000001000;
const ST_PRESENT_INFO_KHR: i32 = 1000001001;

#[repr(C)]
struct VkExtensionProperties {
    extension_name: [c_char; 256],
    spec_version: u32,
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

type FnEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    VkPhysicalDevice,
    *const c_char,
    *mut u32,
    *mut VkExtensionProperties,
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
type FnDeviceWaitIdle = unsafe extern "system" fn(VkDevice) -> VkResult;
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
type FnQueuePresentKhr = unsafe extern "system" fn(VkQueue, *const VkPresentInfoKhr) -> VkResult;
type FnCreateImage = unsafe extern "system" fn(
    VkDevice,
    *const VkImageCreateInfo,
    *const c_void,
    *mut VkImage,
) -> VkResult;
type FnDestroyImage = unsafe extern "system" fn(VkDevice, VkImage, *const c_void);
type FnGetImageMemoryRequirements =
    unsafe extern "system" fn(VkDevice, VkImage, *mut VkMemoryRequirements);
type FnBindImageMemory =
    unsafe extern "system" fn(VkDevice, VkImage, VkDeviceMemory, u64) -> VkResult;
type FnFreeCommandBuffers =
    unsafe extern "system" fn(VkDevice, VkCommandPool, u32, *const VkCommandBuffer);
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
type FnCmdCopyImage =
    unsafe extern "system" fn(VkCommandBuffer, VkImage, i32, VkImage, i32, u32, *const VkImageCopy);
type FnResetFences = unsafe extern "system" fn(VkDevice, u32, *const VkFence) -> VkResult;

// ---------------------------------------------------------------------------
// Graphics machinery (render pass / framebuffer / pipeline / sampler /
// vertex input), used only by the gfx.* surface below — the same
// transcription discipline as everything above.
// ---------------------------------------------------------------------------

const VK_FORMAT_R32_SFLOAT: i32 = 100;
const VK_FORMAT_R32G32_SFLOAT: i32 = 103;
const VK_FORMAT_R32G32B32_SFLOAT: i32 = 106;
const VK_FORMAT_R32G32B32A32_SFLOAT: i32 = 109;
const VK_FORMAT_D32_SFLOAT: i32 = 126;

const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 0x4;
const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x20;
const VK_IMAGE_ASPECT_DEPTH_BIT: u32 = 0x2;

const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: i32 = 2;
const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: i32 = 3;
const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: i32 = 5;

const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 0x400;
const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: u32 = 0x100;
const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: u32 = 0x200;
const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: u32 = 0x80;
const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: u32 = 0x80;
const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 0x100;
const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: u32 = 0x200;
const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: u32 = 0x400;
const VK_ACCESS_SHADER_READ_BIT: u32 = 0x20;

const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: u32 = 1;
const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: u32 = 6;
const VK_SHADER_STAGE_VERTEX_BIT: u32 = 0x1;
const VK_SHADER_STAGE_FRAGMENT_BIT: u32 = 0x10;

const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 0x2;
const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: u32 = 0x10;
const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: u32 = 0x40;
const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 0x80;

const VK_PIPELINE_BIND_POINT_GRAPHICS: u32 = 0;
const VK_ATTACHMENT_LOAD_OP_LOAD: i32 = 0;
const VK_ATTACHMENT_STORE_OP_STORE: i32 = 0;
const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: i32 = 3;
const VK_POLYGON_MODE_FILL: i32 = 0;
const VK_CULL_MODE_NONE: u32 = 0;
const VK_FRONT_FACE_COUNTER_CLOCKWISE: i32 = 0;
const VK_COMPARE_OP_LESS: i32 = 1;
const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
const VK_DYNAMIC_STATE_SCISSOR: i32 = 1;
const VK_VERTEX_INPUT_RATE_VERTEX: i32 = 0;
const VK_INDEX_TYPE_UINT32: i32 = 1;
const VK_FILTER_LINEAR: i32 = 1;
const VK_SAMPLER_MIPMAP_MODE_NEAREST: i32 = 0;
const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: i32 = 2;
const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;
const VK_COMPARE_OP_NEVER: i32 = 0;
const VK_BORDER_COLOR_OPAQUE_BLACK: i32 = 3;

const ST_IMAGE_VIEW_CREATE_INFO: i32 = 15;
const ST_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: i32 = 19;
const ST_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: i32 = 20;
const ST_PIPELINE_VIEWPORT_STATE_CREATE_INFO: i32 = 22;
const ST_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: i32 = 23;
const ST_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: i32 = 24;
const ST_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: i32 = 25;
const ST_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: i32 = 26;
const ST_PIPELINE_DYNAMIC_STATE_CREATE_INFO: i32 = 27;
const ST_GRAPHICS_PIPELINE_CREATE_INFO: i32 = 28;
const ST_SAMPLER_CREATE_INFO: i32 = 31;
const ST_FRAMEBUFFER_CREATE_INFO: i32 = 37;
const ST_RENDER_PASS_CREATE_INFO: i32 = 38;
const ST_RENDER_PASS_BEGIN_INFO: i32 = 43;

#[repr(C)]
struct VkComponentMapping {
    r: i32,
    g: i32,
    b: i32,
    a: i32,
}
#[repr(C)]
struct VkImageViewCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    image: VkImage,
    view_type: i32,
    format: i32,
    components: VkComponentMapping,
    subresource_range: VkImageSubresourceRange,
}
#[repr(C)]
struct VkAttachmentDescription {
    flags: u32,
    format: i32,
    samples: u32,
    load_op: i32,
    store_op: i32,
    stencil_load_op: i32,
    stencil_store_op: i32,
    initial_layout: i32,
    final_layout: i32,
}
#[repr(C)]
struct VkAttachmentReference {
    attachment: u32,
    layout: i32,
}
#[repr(C)]
struct VkSubpassDescription {
    flags: u32,
    pipeline_bind_point: u32,
    input_attachment_count: u32,
    p_input_attachments: *const VkAttachmentReference,
    color_attachment_count: u32,
    p_color_attachments: *const VkAttachmentReference,
    p_resolve_attachments: *const VkAttachmentReference,
    p_depth_stencil_attachment: *const VkAttachmentReference,
    preserve_attachment_count: u32,
    p_preserve_attachments: *const u32,
}
#[repr(C)]
struct VkRenderPassCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    attachment_count: u32,
    p_attachments: *const VkAttachmentDescription,
    subpass_count: u32,
    p_subpasses: *const VkSubpassDescription,
    dependency_count: u32,
    p_dependencies: *const c_void,
}
#[repr(C)]
struct VkFramebufferCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    render_pass: u64,
    attachment_count: u32,
    p_attachments: *const u64,
    width: u32,
    height: u32,
    layers: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkOffset2D {
    x: i32,
    y: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkRect2D {
    offset: VkOffset2D,
    extent: VkExtent2D,
}
#[repr(C)]
struct VkRenderPassBeginInfo {
    s_type: i32,
    p_next: *const c_void,
    render_pass: u64,
    framebuffer: u64,
    render_area: VkRect2D,
    clear_value_count: u32,
    p_clear_values: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkVertexInputBindingDescription {
    binding: u32,
    stride: u32,
    input_rate: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkVertexInputAttributeDescription {
    location: u32,
    binding: u32,
    format: i32,
    offset: u32,
}
#[repr(C)]
struct VkPipelineVertexInputStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    vertex_binding_description_count: u32,
    p_vertex_binding_descriptions: *const VkVertexInputBindingDescription,
    vertex_attribute_description_count: u32,
    p_vertex_attribute_descriptions: *const VkVertexInputAttributeDescription,
}
#[repr(C)]
struct VkPipelineInputAssemblyStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    topology: i32,
    primitive_restart_enable: u32,
}
#[repr(C)]
struct VkViewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}
#[repr(C)]
struct VkPipelineViewportStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    viewport_count: u32,
    p_viewports: *const VkViewport,
    scissor_count: u32,
    p_scissors: *const VkRect2D,
}
#[repr(C)]
struct VkPipelineRasterizationStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    depth_clamp_enable: u32,
    rasterizer_discard_enable: u32,
    polygon_mode: i32,
    cull_mode: u32,
    front_face: i32,
    depth_bias_enable: u32,
    depth_bias_constant_factor: f32,
    depth_bias_clamp: f32,
    depth_bias_slope_factor: f32,
    line_width: f32,
}
#[repr(C)]
struct VkPipelineMultisampleStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    rasterization_samples: u32,
    sample_shading_enable: u32,
    min_sample_shading: f32,
    p_sample_mask: *const u32,
    alpha_to_coverage_enable: u32,
    alpha_to_one_enable: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct VkStencilOpState {
    fail_op: i32,
    pass_op: i32,
    depth_fail_op: i32,
    compare_op: i32,
    compare_mask: u32,
    write_mask: u32,
    reference: u32,
}
#[repr(C)]
struct VkPipelineDepthStencilStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    depth_test_enable: u32,
    depth_write_enable: u32,
    depth_compare_op: i32,
    depth_bounds_test_enable: u32,
    stencil_test_enable: u32,
    front: VkStencilOpState,
    back: VkStencilOpState,
    min_depth_bounds: f32,
    max_depth_bounds: f32,
}
#[repr(C)]
struct VkPipelineColorBlendAttachmentState {
    blend_enable: u32,
    src_color_blend_factor: i32,
    dst_color_blend_factor: i32,
    color_blend_op: i32,
    src_alpha_blend_factor: i32,
    dst_alpha_blend_factor: i32,
    alpha_blend_op: i32,
    color_write_mask: u32,
}
#[repr(C)]
struct VkPipelineColorBlendStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    logic_op_enable: u32,
    logic_op: i32,
    attachment_count: u32,
    p_attachments: *const VkPipelineColorBlendAttachmentState,
    blend_constants: [f32; 4],
}
#[repr(C)]
struct VkPipelineDynamicStateCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    dynamic_state_count: u32,
    p_dynamic_states: *const i32,
}
#[repr(C)]
struct VkGraphicsPipelineCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage_count: u32,
    p_stages: *const VkPipelineShaderStageCreateInfo,
    p_vertex_input_state: *const VkPipelineVertexInputStateCreateInfo,
    p_input_assembly_state: *const VkPipelineInputAssemblyStateCreateInfo,
    p_tessellation_state: *const c_void,
    p_viewport_state: *const VkPipelineViewportStateCreateInfo,
    p_rasterization_state: *const VkPipelineRasterizationStateCreateInfo,
    p_multisample_state: *const VkPipelineMultisampleStateCreateInfo,
    p_depth_stencil_state: *const VkPipelineDepthStencilStateCreateInfo,
    p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo,
    p_dynamic_state: *const VkPipelineDynamicStateCreateInfo,
    layout: VkPipelineLayout,
    render_pass: u64,
    subpass: u32,
    base_pipeline_handle: VkPipeline,
    base_pipeline_index: i32,
}
#[repr(C)]
struct VkBufferImageCopy {
    buffer_offset: u64,
    buffer_row_length: u32,
    buffer_image_height: u32,
    image_subresource: VkImageSubresourceLayers,
    image_offset: VkOffset3D,
    image_extent: VkExtent3D,
}
#[repr(C)]
struct VkSamplerCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    mag_filter: i32,
    min_filter: i32,
    mipmap_mode: i32,
    address_mode_u: i32,
    address_mode_v: i32,
    address_mode_w: i32,
    mip_lod_bias: f32,
    anisotropy_enable: u32,
    max_anisotropy: f32,
    compare_enable: u32,
    compare_op: i32,
    min_lod: f32,
    max_lod: f32,
    border_color: i32,
    unnormalized_coordinates: u32,
}
#[repr(C)]
struct VkDescriptorImageInfo {
    sampler: u64,
    image_view: u64,
    image_layout: i32,
}

type FnCreateImageView = unsafe extern "system" fn(
    VkDevice,
    *const VkImageViewCreateInfo,
    *const c_void,
    *mut u64,
) -> VkResult;
type FnDestroyImageView = unsafe extern "system" fn(VkDevice, u64, *const c_void);
type FnCreateRenderPass = unsafe extern "system" fn(
    VkDevice,
    *const VkRenderPassCreateInfo,
    *const c_void,
    *mut u64,
) -> VkResult;
type FnDestroyRenderPass = unsafe extern "system" fn(VkDevice, u64, *const c_void);
type FnCreateFramebuffer = unsafe extern "system" fn(
    VkDevice,
    *const VkFramebufferCreateInfo,
    *const c_void,
    *mut u64,
) -> VkResult;
type FnDestroyFramebuffer = unsafe extern "system" fn(VkDevice, u64, *const c_void);
type FnCreateGraphicsPipelines = unsafe extern "system" fn(
    VkDevice,
    u64,
    u32,
    *const VkGraphicsPipelineCreateInfo,
    *const c_void,
    *mut VkPipeline,
) -> VkResult;
type FnCmdBeginRenderPass =
    unsafe extern "system" fn(VkCommandBuffer, *const VkRenderPassBeginInfo, i32);
type FnCmdEndRenderPass = unsafe extern "system" fn(VkCommandBuffer);
type FnCmdSetViewport = unsafe extern "system" fn(VkCommandBuffer, u32, u32, *const VkViewport);
type FnCmdSetScissor = unsafe extern "system" fn(VkCommandBuffer, u32, u32, *const VkRect2D);
type FnCmdBindVertexBuffers =
    unsafe extern "system" fn(VkCommandBuffer, u32, u32, *const VkBuffer, *const u64);
type FnCmdBindIndexBuffer = unsafe extern "system" fn(VkCommandBuffer, VkBuffer, u64, i32);
type FnCmdDraw = unsafe extern "system" fn(VkCommandBuffer, u32, u32, u32, u32);
type FnCmdDrawIndexed = unsafe extern "system" fn(VkCommandBuffer, u32, u32, u32, i32, u32);
type FnCmdCopyImageToBuffer = unsafe extern "system" fn(
    VkCommandBuffer,
    VkImage,
    i32,
    VkBuffer,
    u32,
    *const VkBufferImageCopy,
);
type FnCmdCopyBufferToImage = unsafe extern "system" fn(
    VkCommandBuffer,
    VkBuffer,
    VkImage,
    i32,
    u32,
    *const VkBufferImageCopy,
);
type FnCreateSampler = unsafe extern "system" fn(
    VkDevice,
    *const VkSamplerCreateInfo,
    *const c_void,
    *mut u64,
) -> VkResult;
type FnDestroySampler = unsafe extern "system" fn(VkDevice, u64, *const c_void);
type FnCmdClearDepthStencilImage = unsafe extern "system" fn(
    VkCommandBuffer,
    VkImage,
    i32,
    *const [f32; 2],
    u32,
    *const VkImageSubresourceRange,
);

/// Resolve one entry point through `vkGetInstanceProcAddr` or `Err` — used
/// for everything past `vkCreateInstance` (instance-level trampolines
/// dispatch device-level calls correctly; the direct-`vkGetDeviceProcAddr`
/// optimization is a later efficiency-pass item, not a Phase 1 need).
macro_rules! vkload {
    ($gipa:expr, $inst:expr, $name:literal, $ty:ty) => {{
        let cname = std::ffi::CString::new($name).unwrap();
        let p = $gipa($inst, cname.as_ptr());
        if p.is_null() {
            return Err(format!(
                "window.create_vulkan: loader is missing `{}`",
                $name
            ));
        }
        std::mem::transmute::<crate::vk::PfnVoidFunction, $ty>(p)
    }};
}
pub(crate) use vkload;

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

    // gfx.* machinery (the draw-call surface).
    create_shader_module: FnCreateShaderModule,
    destroy_shader_module: FnDestroyShaderModule,
    create_descriptor_set_layout: FnCreateDescriptorSetLayout,
    destroy_descriptor_set_layout: FnDestroyDescriptorSetLayout,
    create_pipeline_layout: FnCreatePipelineLayout,
    destroy_pipeline_layout: FnDestroyPipelineLayout,
    create_graphics_pipelines: FnCreateGraphicsPipelines,
    destroy_pipeline: FnDestroyPipeline,
    create_descriptor_pool: FnCreateDescriptorPool,
    destroy_descriptor_pool: FnDestroyDescriptorPool,
    allocate_descriptor_sets: FnAllocateDescriptorSets,
    update_descriptor_sets: FnUpdateDescriptorSets,
    create_buffer: FnCreateBuffer,
    destroy_buffer: FnDestroyBuffer,
    get_buffer_memory_requirements: FnGetBufferMemoryRequirements,
    bind_buffer_memory: FnBindBufferMemory,
    map_memory: FnMapMemory,
    create_image_view: FnCreateImageView,
    destroy_image_view: FnDestroyImageView,
    create_render_pass: FnCreateRenderPass,
    destroy_render_pass: FnDestroyRenderPass,
    create_framebuffer: FnCreateFramebuffer,
    destroy_framebuffer: FnDestroyFramebuffer,
    cmd_begin_render_pass: FnCmdBeginRenderPass,
    cmd_end_render_pass: FnCmdEndRenderPass,
    cmd_set_viewport: FnCmdSetViewport,
    cmd_set_scissor: FnCmdSetScissor,
    cmd_bind_pipeline: FnCmdBindPipeline,
    cmd_bind_descriptor_sets: FnCmdBindDescriptorSets,
    cmd_bind_vertex_buffers: FnCmdBindVertexBuffers,
    cmd_bind_index_buffer: FnCmdBindIndexBuffer,
    cmd_draw: FnCmdDraw,
    cmd_draw_indexed: FnCmdDrawIndexed,
    cmd_copy_image_to_buffer: FnCmdCopyImageToBuffer,
    cmd_copy_buffer_to_image: FnCmdCopyBufferToImage,
    cmd_clear_depth_stencil_image: FnCmdClearDepthStencilImage,
    create_sampler: FnCreateSampler,
    destroy_sampler: FnDestroySampler,
    get_memory_properties: FnGetPhysicalDeviceMemoryProperties,
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
const SUBRESOURCE_DEPTH: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_DEPTH_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// The platform-neutral Vulkan window: the WSI chain (instance → surface
/// → device → swapchain), the offscreen stable back buffer, and the full
/// `gfx.*` draw-call state. Each platform shim composes one of these next
/// to its native window state — nothing here knows which platform it is
/// presenting on.
pub(crate) struct Chain {
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

    // gfx.* state (the draw-call surface). Handle tables map the Int
    // handles Socrates sees onto Vulkan objects, exactly the Metal backend's
    // pattern (Vulkan handles are 64-bit and driver-owned, not the small
    // driver-issued integers GL hands out).
    off_view: u64,
    depth_image: VkImage,
    depth_memory: VkDeviceMemory,
    depth_view: u64,
    depth_layout: i32,
    render_pass: u64,
    framebuffer: u64,
    sampler: u64,
    programs: HashMap<u32, Program>,
    next_program: u32,
    buffers: HashMap<u32, GfxBuffer>,
    next_buffer: u32,
    textures: HashMap<u32, GfxTexture>,
    next_texture: u32,
    vaos: HashMap<u32, VaoState>,
    next_vao: u32,
    bound_vao: u32,
    bound_array_buffer: u32,
    current_program: u32,
    depth_test: bool,
    viewport: Option<(i32, i32, i32, i32)>,
    texture_units: [u32; 8],
    active_unit: usize,
}

/// One linked SPIR-V vertex+fragment program: modules, layouts, the
/// per-stage reflected uniform blocks with their persistently-mapped UBO
/// buffers, texture bindings, and the pipeline cache (Vulkan fuses
/// program + vertex layout + depth state into one immutable object; GL
/// binds them independently — caching per combination bridges the two,
/// the same bridge the Metal backend builds for its PSOs).
struct Program {
    vs: VkShaderModule,
    fs: VkShaderModule,
    dsl: VkDescriptorSetLayout,
    playout: VkPipelineLayout,
    dpool: VkDescriptorPool,
    dset: VkDescriptorSet,
    vs_uniforms: StageUniforms,
    fs_uniforms: StageUniforms,
    vs_ubo: UboBuffer,
    fs_ubo: UboBuffer,
    /// Combined-image-sampler bindings the fragment SPIR-V declares
    /// (binding N samples texture unit N-2 — see the module docs).
    texture_bindings: Vec<u32>,
    /// (vertex-layout fingerprint, depth_test) → baked pipeline.
    pipelines: HashMap<(u64, bool), VkPipeline>,
}

/// A stage's reflected uniform block: member name → (byte offset, size),
/// plus the block's total size and the CPU-side staging bytes
/// `set_uniform_*` writes into (uploaded to the UBO before each draw).
struct StageUniforms {
    binding: Option<u32>,
    size: usize,
    members: Vec<(String, usize, usize)>,
    staging: Vec<u8>,
}

/// A host-visible, persistently-mapped device buffer. Draws are
/// synchronous (submit → fence wait), so rewriting one buffer between
/// draws can never race work in flight.
struct UboBuffer {
    buf: VkBuffer,
    mem: VkDeviceMemory,
    ptr: *mut u8,
}

/// One `gfx.create_buffer` handle: host-visible mapped storage sized to
/// the largest upload seen (grown by destroy+recreate — safe, synchronous
/// draws), usable as vertex and index data.
struct GfxBuffer {
    buf: VkBuffer,
    mem: VkDeviceMemory,
    ptr: *mut u8,
    cap: usize,
}

/// One `gfx.create_texture` handle: an optimal-tiled sampled image (+its
/// view), uploaded through a staging buffer. Its layout is UNDEFINED until
/// the first `upload_texture` and SHADER_READ_ONLY_OPTIMAL forever after —
/// a constant, so it isn't tracked per-object the way the render targets'
/// layouts are.
struct GfxTexture {
    image: VkImage,
    memory: VkDeviceMemory,
    view: u64,
}

/// The VAO shim, identical in spirit to the Metal backend's: GL's
/// `glVertexAttribPointer` captures (size, stride, offset, currently
/// bound array buffer) into VAO state; this records the same tuple per
/// attribute index and replays it as VkVertexInput*Descriptions at
/// pipeline bake + vkCmdBindVertexBuffers at draw. The element-array
/// binding is VAO state on both APIs.
#[derive(Default, Clone)]
struct VaoState {
    attribs: Vec<(u32, i32, i32, i32, u32)>, // (index, size, stride, offset, buffer handle)
    element_buffer: u32,
}

impl Chain {
    /// The instance→surface→device→swapchain→offscreen chain. The platform
    /// shim has already created its native window; on any failure every
    /// Vulkan object created so far is destroyed before returning `Err`
    /// (the shim then tears down its window). `surface_ext` is the
    /// platform's WSI instance extension (`VK_KHR_xlib_surface` /
    /// `VK_KHR_win32_surface`), `create_surface` wraps the platform's
    /// `vkCreate*SurfaceKHR` call, `surface_noun` names the surface kind
    /// in the no-device error, and `fallback` is the live window size,
    /// used when the surface does not dictate an extent.
    pub(crate) unsafe fn create(
        gipa: FnGetInstanceProcAddr,
        surface_ext: &str,
        surface_noun: &str,
        create_surface: impl FnOnce(FnGetInstanceProcAddr, VkInstance) -> Result<VkSurfaceKhr, String>,
        fallback: (i32, i32),
    ) -> Result<Chain, String> {
        // Instance with the two WSI instance extensions.
        let create_instance = vkload!(gipa, ptr::null_mut(), "vkCreateInstance", FnCreateInstance);
        let app = VkApplicationInfo {
            s_type: ST_APPLICATION_INFO,
            p_next: ptr::null(),
            p_application_name: ptr::null(),
            application_version: 0,
            p_engine_name: ptr::null(),
            engine_version: 0,
            api_version: VK_API_VERSION_1_0,
        };
        let ext_surface = CString::new("VK_KHR_surface").unwrap();
        let ext_platform = CString::new(surface_ext).unwrap();
        let inst_exts = [ext_surface.as_ptr(), ext_platform.as_ptr()];
        let ici = VkInstanceCreateInfo {
            s_type: ST_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_application_info: &app,
            enabled_layer_count: 0,
            pp_enabled_layer_names: ptr::null(),
            enabled_extension_count: 2,
            pp_enabled_extension_names: inst_exts.as_ptr(),
        };
        let mut instance: VkInstance = ptr::null_mut();
        let r = create_instance(&ici, ptr::null(), &mut instance);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkCreateInstance failed ({r}) — driver lacks {surface_ext}?"
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
                return Err($msg);
            }};
        }

        // Surface over the platform's native window.
        let surface = match create_surface(gipa, instance) {
            Ok(s) => s,
            Err(e) => {
                fail!(0u64, ptr::null_mut() as VkDevice, e);
            }
        };

        // Physical device: first one with a queue family that is both
        // GRAPHICS-capable and able to present to this surface.
        let enum_phys = vkload!(
            gipa,
            instance,
            "vkEnumeratePhysicalDevices",
            FnEnumeratePhysicalDevices
        );
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
                    let has = |name: &[u8]| {
                        eps.iter().any(|e| {
                            std::ffi::CStr::from_ptr(e.extension_name.as_ptr())
                                .to_bytes()
                                .eq(name)
                        })
                    };
                    if has(b"VK_KHR_swapchain") && has(b"VK_KHR_maintenance1") {
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
                format!(
                    "window.create_vulkan: no Vulkan device can present to {surface_noun} \
                     (need a graphics queue with surface support and VK_KHR_swapchain)"
                )
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
            p_queue_priorities: &prio,
        };
        let ext_swapchain = CString::new("VK_KHR_swapchain").unwrap();
        // maintenance1 legalizes negative viewport heights, which is how
        // the gfx surface makes Vulkan's downward clip-space +Y behave
        // like GL's upward one with no shader changes (SPEC § 7.4's
        // Vulkan notes). Universally present wherever swapchains are
        // (core in 1.1); required rather than probed so a hypothetical
        // driver without it fails loudly at create, not with flipped
        // geometry later.
        let ext_maintenance1 = CString::new("VK_KHR_maintenance1").unwrap();
        let dev_exts = [ext_swapchain.as_ptr(), ext_maintenance1.as_ptr()];
        let dci = VkDeviceCreateInfo {
            s_type: ST_DEVICE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            queue_create_info_count: 1,
            p_queue_create_infos: &qci,
            enabled_layer_count: 0,
            pp_enabled_layer_names: ptr::null(),
            enabled_extension_count: 2,
            pp_enabled_extension_names: dev_exts.as_ptr(),
            p_enabled_features: ptr::null(),
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

        let mut inner = Chain {
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
            off_view: 0,
            depth_image: 0,
            depth_memory: 0,
            depth_view: 0,
            depth_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            render_pass: 0,
            framebuffer: 0,
            sampler: 0,
            programs: HashMap::new(),
            next_program: 1,
            buffers: HashMap::new(),
            next_buffer: 1,
            textures: HashMap::new(),
            next_texture: 1,
            vaos: HashMap::new(),
            next_vao: 1,
            bound_vao: 0,
            bound_array_buffer: 0,
            current_program: 0,
            depth_test: false,
            viewport: None,
            texture_units: [0; 8],
            active_unit: 0,
        };

        // Pick the surface format once (UNORM preference — see module
        // docs), then build the swapchain + offscreen pair.
        if let Err(e) = inner.pick_surface_format(gipa) {
            inner.destroy_vulkan();
            return Err(e);
        }
        if let Err(e) = inner.rebuild_swapchain_and_offscreen(fallback) {
            inner.destroy_vulkan();
            return Err(e);
        }
        Ok(inner)
    }

    unsafe fn resolve_fns(
        gipa: FnGetInstanceProcAddr,
        instance: VkInstance,
    ) -> Result<Fns, String> {
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
            create_shader_module: vkload!(
                gipa,
                instance,
                "vkCreateShaderModule",
                FnCreateShaderModule
            ),
            destroy_shader_module: vkload!(
                gipa,
                instance,
                "vkDestroyShaderModule",
                FnDestroyShaderModule
            ),
            create_descriptor_set_layout: vkload!(
                gipa,
                instance,
                "vkCreateDescriptorSetLayout",
                FnCreateDescriptorSetLayout
            ),
            destroy_descriptor_set_layout: vkload!(
                gipa,
                instance,
                "vkDestroyDescriptorSetLayout",
                FnDestroyDescriptorSetLayout
            ),
            create_pipeline_layout: vkload!(
                gipa,
                instance,
                "vkCreatePipelineLayout",
                FnCreatePipelineLayout
            ),
            destroy_pipeline_layout: vkload!(
                gipa,
                instance,
                "vkDestroyPipelineLayout",
                FnDestroyPipelineLayout
            ),
            create_graphics_pipelines: vkload!(
                gipa,
                instance,
                "vkCreateGraphicsPipelines",
                FnCreateGraphicsPipelines
            ),
            destroy_pipeline: vkload!(gipa, instance, "vkDestroyPipeline", FnDestroyPipeline),
            create_descriptor_pool: vkload!(
                gipa,
                instance,
                "vkCreateDescriptorPool",
                FnCreateDescriptorPool
            ),
            destroy_descriptor_pool: vkload!(
                gipa,
                instance,
                "vkDestroyDescriptorPool",
                FnDestroyDescriptorPool
            ),
            allocate_descriptor_sets: vkload!(
                gipa,
                instance,
                "vkAllocateDescriptorSets",
                FnAllocateDescriptorSets
            ),
            update_descriptor_sets: vkload!(
                gipa,
                instance,
                "vkUpdateDescriptorSets",
                FnUpdateDescriptorSets
            ),
            create_buffer: vkload!(gipa, instance, "vkCreateBuffer", FnCreateBuffer),
            destroy_buffer: vkload!(gipa, instance, "vkDestroyBuffer", FnDestroyBuffer),
            get_buffer_memory_requirements: vkload!(
                gipa,
                instance,
                "vkGetBufferMemoryRequirements",
                FnGetBufferMemoryRequirements
            ),
            bind_buffer_memory: vkload!(gipa, instance, "vkBindBufferMemory", FnBindBufferMemory),
            map_memory: vkload!(gipa, instance, "vkMapMemory", FnMapMemory),
            create_image_view: vkload!(gipa, instance, "vkCreateImageView", FnCreateImageView),
            destroy_image_view: vkload!(gipa, instance, "vkDestroyImageView", FnDestroyImageView),
            create_render_pass: vkload!(gipa, instance, "vkCreateRenderPass", FnCreateRenderPass),
            destroy_render_pass: vkload!(
                gipa,
                instance,
                "vkDestroyRenderPass",
                FnDestroyRenderPass
            ),
            create_framebuffer: vkload!(gipa, instance, "vkCreateFramebuffer", FnCreateFramebuffer),
            destroy_framebuffer: vkload!(
                gipa,
                instance,
                "vkDestroyFramebuffer",
                FnDestroyFramebuffer
            ),
            cmd_begin_render_pass: vkload!(
                gipa,
                instance,
                "vkCmdBeginRenderPass",
                FnCmdBeginRenderPass
            ),
            cmd_end_render_pass: vkload!(gipa, instance, "vkCmdEndRenderPass", FnCmdEndRenderPass),
            cmd_set_viewport: vkload!(gipa, instance, "vkCmdSetViewport", FnCmdSetViewport),
            cmd_set_scissor: vkload!(gipa, instance, "vkCmdSetScissor", FnCmdSetScissor),
            cmd_bind_pipeline: vkload!(gipa, instance, "vkCmdBindPipeline", FnCmdBindPipeline),
            cmd_bind_descriptor_sets: vkload!(
                gipa,
                instance,
                "vkCmdBindDescriptorSets",
                FnCmdBindDescriptorSets
            ),
            cmd_bind_vertex_buffers: vkload!(
                gipa,
                instance,
                "vkCmdBindVertexBuffers",
                FnCmdBindVertexBuffers
            ),
            cmd_bind_index_buffer: vkload!(
                gipa,
                instance,
                "vkCmdBindIndexBuffer",
                FnCmdBindIndexBuffer
            ),
            cmd_draw: vkload!(gipa, instance, "vkCmdDraw", FnCmdDraw),
            cmd_draw_indexed: vkload!(gipa, instance, "vkCmdDrawIndexed", FnCmdDrawIndexed),
            cmd_copy_image_to_buffer: vkload!(
                gipa,
                instance,
                "vkCmdCopyImageToBuffer",
                FnCmdCopyImageToBuffer
            ),
            cmd_copy_buffer_to_image: vkload!(
                gipa,
                instance,
                "vkCmdCopyBufferToImage",
                FnCmdCopyBufferToImage
            ),
            cmd_clear_depth_stencil_image: vkload!(
                gipa,
                instance,
                "vkCmdClearDepthStencilImage",
                FnCmdClearDepthStencilImage
            ),
            create_sampler: vkload!(gipa, instance, "vkCreateSampler", FnCreateSampler),
            destroy_sampler: vkload!(gipa, instance, "vkDestroySampler", FnDestroySampler),
            get_memory_properties: vkload!(
                gipa,
                instance,
                "vkGetPhysicalDeviceMemoryProperties",
                FnGetPhysicalDeviceMemoryProperties
            ),
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
            .or_else(|| {
                formats
                    .iter()
                    .find(|f| f.format == VK_FORMAT_R8G8B8A8_UNORM)
            })
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
            return Err(
                "window.create_vulkan: surface does not offer FIFO presentation".to_string(),
            );
        }
        Ok(())
    }

    /// (Re)build the swapchain from the surface's *current* extent, plus
    /// the offscreen back buffer at the same size — called at create time
    /// and again whenever presentation reports `OUT_OF_DATE`/`SUBOPTIMAL`
    /// (window resize). Destroys the previous pair first, after a
    /// device-idle wait.
    unsafe fn rebuild_swapchain_and_offscreen(
        &mut self,
        fallback: (i32, i32),
    ) -> Result<(), String> {
        (self.fns.device_wait_idle)(self.device);
        if self.swapchain != 0 {
            (self.fns.destroy_swapchain)(self.device, self.swapchain, ptr::null());
            self.swapchain = 0;
            self.swap_images.clear();
        }
        if self.framebuffer != 0 {
            (self.fns.destroy_framebuffer)(self.device, self.framebuffer, ptr::null());
            self.framebuffer = 0;
        }
        if self.off_view != 0 {
            (self.fns.destroy_image_view)(self.device, self.off_view, ptr::null());
            self.off_view = 0;
        }
        if self.depth_view != 0 {
            (self.fns.destroy_image_view)(self.device, self.depth_view, ptr::null());
            self.depth_view = 0;
        }
        if self.off_image != 0 {
            (self.fns.destroy_image)(self.device, self.off_image, ptr::null());
            self.off_image = 0;
        }
        if self.off_memory != 0 {
            (self.fns.free_memory)(self.device, self.off_memory, ptr::null());
            self.off_memory = 0;
        }
        if self.depth_image != 0 {
            (self.fns.destroy_image)(self.device, self.depth_image, ptr::null());
            self.depth_image = 0;
        }
        if self.depth_memory != 0 {
            (self.fns.free_memory)(self.device, self.depth_memory, ptr::null());
            self.depth_memory = 0;
        }
        self.off_layout = VK_IMAGE_LAYOUT_UNDEFINED;
        self.depth_layout = VK_IMAGE_LAYOUT_UNDEFINED;

        let mut caps: VkSurfaceCapabilitiesKhr = std::mem::zeroed();
        let r = (self.fns.get_surface_capabilities)(self.phys, self.surface, &mut caps);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkGetPhysicalDeviceSurfaceCapabilitiesKHR failed ({r})"
            ));
        }
        // 0xFFFFFFFF means "the surface takes the swapchain's size" — use
        // the live window dimensions then; otherwise the surface dictates.
        let extent = if caps.current_extent.width == u32::MAX {
            VkExtent2D {
                width: fallback.0.max(1) as u32,
                height: fallback.1.max(1) as u32,
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
            image_sharing_mode: VK_SHARING_MODE_EXCLUSIVE as i32,
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
            return Err(format!(
                "window.create_vulkan: vkCreateSwapchainKHR failed ({r})"
            ));
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
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE as i32,
            queue_family_index_count: 0,
            queue_family_indices: ptr::null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image: VkImage = 0;
        let r = (self.fns.create_image)(self.device, &ici, ptr::null(), &mut image);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkCreateImage (offscreen) failed ({r})"
            ));
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
            return Err(format!(
                "window.create_vulkan: vkAllocateMemory (offscreen) failed ({r})"
            ));
        }
        self.off_memory = memory;
        let r = (self.fns.bind_image_memory)(self.device, image, memory, 0);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkBindImageMemory failed ({r})"
            ));
        }

        // Depth attachment (D32) at the same extent, for the gfx.* draw
        // surface — Metal's depth target, transliterated.
        let (dimg, dmem) = self.create_image_2d(
            VK_FORMAT_D32_SFLOAT,
            extent,
            VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        )?;
        self.depth_image = dimg;
        self.depth_memory = dmem;

        // Image views + the one render pass (both attachments loadOp=LOAD:
        // one draw = one pass, GL's free interleaving preserved — the
        // Metal model) + the framebuffer over the pair.
        self.off_view = self.create_view(self.off_image, self.format, VK_IMAGE_ASPECT_COLOR_BIT)?;
        self.depth_view = self.create_view(
            self.depth_image,
            VK_FORMAT_D32_SFLOAT,
            VK_IMAGE_ASPECT_DEPTH_BIT,
        )?;
        if self.render_pass == 0 {
            let attachments = [
                VkAttachmentDescription {
                    flags: 0,
                    format: self.format,
                    samples: VK_SAMPLE_COUNT_1_BIT,
                    load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    stencil_load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    stencil_store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    initial_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    final_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                },
                VkAttachmentDescription {
                    flags: 0,
                    format: VK_FORMAT_D32_SFLOAT,
                    samples: VK_SAMPLE_COUNT_1_BIT,
                    load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    stencil_load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    stencil_store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    initial_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                    final_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                },
            ];
            let color_ref = VkAttachmentReference {
                attachment: 0,
                layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            };
            let depth_ref = VkAttachmentReference {
                attachment: 1,
                layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };
            let subpass = VkSubpassDescription {
                flags: 0,
                pipeline_bind_point: VK_PIPELINE_BIND_POINT_GRAPHICS,
                input_attachment_count: 0,
                p_input_attachments: ptr::null(),
                color_attachment_count: 1,
                p_color_attachments: &color_ref,
                p_resolve_attachments: ptr::null(),
                p_depth_stencil_attachment: &depth_ref,
                preserve_attachment_count: 0,
                p_preserve_attachments: ptr::null(),
            };
            let rpci = VkRenderPassCreateInfo {
                s_type: ST_RENDER_PASS_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                attachment_count: 2,
                p_attachments: attachments.as_ptr(),
                subpass_count: 1,
                p_subpasses: &subpass,
                // No explicit dependencies: every submission here is
                // fence-waited before the next records, so the implicit
                // external dependencies are trivially satisfied.
                dependency_count: 0,
                p_dependencies: ptr::null(),
            };
            let mut rp: u64 = 0;
            let r = (self.fns.create_render_pass)(self.device, &rpci, ptr::null(), &mut rp);
            if r != VK_SUCCESS {
                return Err(format!(
                    "window.create_vulkan: vkCreateRenderPass failed ({r})"
                ));
            }
            self.render_pass = rp;
        }
        let fb_views = [self.off_view, self.depth_view];
        let fci = VkFramebufferCreateInfo {
            s_type: ST_FRAMEBUFFER_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            render_pass: self.render_pass,
            attachment_count: 2,
            p_attachments: fb_views.as_ptr(),
            width: extent.width,
            height: extent.height,
            layers: 1,
        };
        let mut fb: u64 = 0;
        let r = (self.fns.create_framebuffer)(self.device, &fci, ptr::null(), &mut fb);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkCreateFramebuffer failed ({r})"
            ));
        }
        self.framebuffer = fb;

        // The one fixed sampler (linear, clamp-to-edge — the mode
        // gfx.upload_texture configures on GL), created once.
        if self.sampler == 0 {
            let sci = VkSamplerCreateInfo {
                s_type: ST_SAMPLER_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                mag_filter: VK_FILTER_LINEAR,
                min_filter: VK_FILTER_LINEAR,
                mipmap_mode: VK_SAMPLER_MIPMAP_MODE_NEAREST,
                address_mode_u: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                address_mode_v: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                address_mode_w: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                mip_lod_bias: 0.0,
                anisotropy_enable: 0,
                max_anisotropy: 1.0,
                compare_enable: 0,
                compare_op: VK_COMPARE_OP_NEVER,
                min_lod: 0.0,
                max_lod: 0.0,
                border_color: VK_BORDER_COLOR_OPAQUE_BLACK,
                unnormalized_coordinates: 0,
            };
            let mut sampler: u64 = 0;
            let r = (self.fns.create_sampler)(self.device, &sci, ptr::null(), &mut sampler);
            if r != VK_SUCCESS {
                return Err(format!(
                    "window.create_vulkan: vkCreateSampler failed ({r})"
                ));
            }
            self.sampler = sampler;
        }
        Ok(())
    }

    /// Create a 2D optimal-tiled image + bound device memory — the shape
    /// the offscreen/depth targets and textures all share.
    unsafe fn create_image_2d(
        &mut self,
        format: i32,
        extent: VkExtent2D,
        usage: u32,
    ) -> Result<(VkImage, VkDeviceMemory), String> {
        let ici = VkImageCreateInfo {
            s_type: ST_IMAGE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            image_type: VK_IMAGE_TYPE_2D,
            format,
            extent: VkExtent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE as i32,
            queue_family_index_count: 0,
            queue_family_indices: ptr::null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image: VkImage = 0;
        let r = (self.fns.create_image)(self.device, &ici, ptr::null(), &mut image);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkCreateImage failed ({r})"));
        }
        let mut req: VkMemoryRequirements = std::mem::zeroed();
        (self.fns.get_image_memory_requirements)(self.device, image, &mut req);
        let memory = match self.alloc_memory(&req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT) {
            Ok(m) => m,
            Err(e) => {
                (self.fns.destroy_image)(self.device, image, ptr::null());
                return Err(e);
            }
        };
        let r = (self.fns.bind_image_memory)(self.device, image, memory, 0);
        if r != VK_SUCCESS {
            (self.fns.destroy_image)(self.device, image, ptr::null());
            (self.fns.free_memory)(self.device, memory, ptr::null());
            return Err(format!(
                "window.create_vulkan: vkBindImageMemory failed ({r})"
            ));
        }
        Ok((image, memory))
    }

    /// Allocate device memory satisfying `req`, preferring `want`
    /// properties, falling back to any allowed type.
    unsafe fn alloc_memory(
        &mut self,
        req: &VkMemoryRequirements,
        want: u32,
    ) -> Result<VkDeviceMemory, String> {
        let mut props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
        (self.fns.get_memory_properties)(self.phys, &mut props);
        let pick = |flags: u32| {
            (0..props.memory_type_count as usize).find(|&i| {
                req.memory_type_bits & (1 << i) != 0
                    && props.memory_types[i].property_flags & flags == flags
            })
        };
        let Some(type_index) = pick(want).or_else(|| pick(0)) else {
            return Err("window.create_vulkan: no suitable memory type".to_string());
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
            return Err(format!(
                "window.create_vulkan: vkAllocateMemory failed ({r})"
            ));
        }
        Ok(memory)
    }

    /// Create an image view over `image` with identity swizzles.
    unsafe fn create_view(
        &mut self,
        image: VkImage,
        format: i32,
        aspect: u32,
    ) -> Result<u64, String> {
        let ivci = VkImageViewCreateInfo {
            s_type: ST_IMAGE_VIEW_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            image,
            view_type: VK_IMAGE_VIEW_TYPE_2D,
            format,
            components: VkComponentMapping {
                r: VK_COMPONENT_SWIZZLE_IDENTITY,
                g: VK_COMPONENT_SWIZZLE_IDENTITY,
                b: VK_COMPONENT_SWIZZLE_IDENTITY,
                a: VK_COMPONENT_SWIZZLE_IDENTITY,
            },
            subresource_range: VkImageSubresourceRange {
                aspect_mask: aspect,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
        };
        let mut view: u64 = 0;
        let r = (self.fns.create_image_view)(self.device, &ivci, ptr::null(), &mut view);
        if r != VK_SUCCESS {
            return Err(format!(
                "window.create_vulkan: vkCreateImageView failed ({r})"
            ));
        }
        Ok(view)
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
            p_inheritance_info: ptr::null(),
        };
        (self.fns.begin_command_buffer)(cmd, &cbi);
        record(&self.fns, cmd);
        (self.fns.end_command_buffer)(cmd);
        let si = VkSubmitInfo {
            s_type: ST_SUBMIT_INFO,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: ptr::null(),
            p_wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: ptr::null(),
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

    /// `vkCmdClearColorImage` into the offscreen back buffer — visible
    /// after the next `swap_buffers`, exactly like GL's clear-then-swap.
    pub(crate) fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
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

    pub(crate) fn swap_buffers(&mut self, fallback: (i32, i32)) {
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
                let _ = self.rebuild_swapchain_and_offscreen(fallback);
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
                let _ = self.rebuild_swapchain_and_offscreen(fallback);
            }
        }
    }

    /// The workhorse of [`Chain::destroy`]: reverse creation order after
    /// a device-idle wait; callable on a partially-built `Chain` (every
    /// handle checked), which is how [`Chain::create`]'s error paths
    /// unwind.
    unsafe fn destroy_vulkan(&mut self) {
        if !self.device.is_null() {
            (self.fns.device_wait_idle)(self.device);
            // gfx objects first (they reference the render targets).
            let program_ids: Vec<u32> = self.programs.keys().copied().collect();
            for id in program_ids {
                self.destroy_program(id);
            }
            let buffer_ids: Vec<u32> = self.buffers.keys().copied().collect();
            for id in buffer_ids {
                self.destroy_gfx_buffer(id);
            }
            let texture_ids: Vec<u32> = self.textures.keys().copied().collect();
            for id in texture_ids {
                self.destroy_gfx_texture(id);
            }
            if self.sampler != 0 {
                (self.fns.destroy_sampler)(self.device, self.sampler, ptr::null());
                self.sampler = 0;
            }
            if self.framebuffer != 0 {
                (self.fns.destroy_framebuffer)(self.device, self.framebuffer, ptr::null());
                self.framebuffer = 0;
            }
            if self.render_pass != 0 {
                (self.fns.destroy_render_pass)(self.device, self.render_pass, ptr::null());
                self.render_pass = 0;
            }
            if self.off_view != 0 {
                (self.fns.destroy_image_view)(self.device, self.off_view, ptr::null());
                self.off_view = 0;
            }
            if self.depth_view != 0 {
                (self.fns.destroy_image_view)(self.device, self.depth_view, ptr::null());
                self.depth_view = 0;
            }
            if self.depth_image != 0 {
                (self.fns.destroy_image)(self.device, self.depth_image, ptr::null());
                self.depth_image = 0;
            }
            if self.depth_memory != 0 {
                (self.fns.free_memory)(self.device, self.depth_memory, ptr::null());
                self.depth_memory = 0;
            }
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

    /// Consume the chain, destroying every Vulkan object in reverse
    /// creation order — the platform shim calls this first, then tears
    /// down its native window (the surface must outlive the swapchain,
    /// the instance the surface, and the native window all of them).
    pub(crate) fn destroy(mut self) {
        // Safety: every handle was produced by the matching create call in
        // [`Chain::create`] and destroyed exactly once, in an order that
        // respects the lifetimes above.
        unsafe {
            self.destroy_vulkan();
        }
    }
}

// ---------------------------------------------------------------------------
// SPIR-V reflection — the in-house parser that keeps `set_uniform_*(name,
// ...)` working on precompiled binaries. A single linear pass over the
// instruction words collects names, decorations, and type shapes; from
// those, each stage's one uniform block (a `Block`-decorated struct
// variable in `Uniform` storage) yields member name → (offset, size), and
// the fragment stage's `UniformConstant` sampled-image variables yield the
// combined-image-sampler bindings. Only the type shapes `gfx` uniforms can
// have (int/float/vec2/3/4/mat4, std140-offset-decorated) are sized —
// anything else simply doesn't resolve by name, matching GL's silent
// unknown-uniform behavior.
// ---------------------------------------------------------------------------

/// What one stage's SPIR-V declares, as far as the gfx surface cares.
struct SpirvReflection {
    ubo_binding: Option<u32>,
    ubo_size: usize,
    members: Vec<(String, usize, usize)>,
    sampler_bindings: Vec<u32>,
}

fn reflect_spirv(words: &[u32]) -> Result<SpirvReflection, String> {
    use std::collections::HashMap as Map;
    const OP_NAME: u32 = 5;
    const OP_MEMBER_NAME: u32 = 6;
    const OP_TYPE_INT: u32 = 21;
    const OP_TYPE_FLOAT: u32 = 22;
    const OP_TYPE_VECTOR: u32 = 23;
    const OP_TYPE_MATRIX: u32 = 24;
    const OP_TYPE_IMAGE: u32 = 25;
    const OP_TYPE_SAMPLED_IMAGE: u32 = 27;
    const OP_TYPE_STRUCT: u32 = 30;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_VARIABLE: u32 = 59;
    const OP_DECORATE: u32 = 71;
    const OP_MEMBER_DECORATE: u32 = 72;
    const DEC_BLOCK: u32 = 2;
    const DEC_BINDING: u32 = 33;
    const DEC_OFFSET: u32 = 35;
    const STORAGE_UNIFORM_CONSTANT: u32 = 0;
    const STORAGE_UNIFORM: u32 = 2;

    if words.len() < 5 || words[0] != 0x0723_0203 {
        return Err("not a SPIR-V module (bad magic)".to_string());
    }

    let mut member_names: Map<(u32, u32), String> = Map::new();
    let mut member_offsets: Map<(u32, u32), u32> = Map::new();
    let mut bindings: Map<u32, u32> = Map::new();
    let mut blocks: Vec<u32> = Vec::new();
    let mut scalars: Map<u32, usize> = Map::new(); // float/int id -> byte size
    let mut vectors: Map<u32, (u32, u32)> = Map::new(); // id -> (component type, count)
    let mut matrices: Map<u32, (u32, u32)> = Map::new(); // id -> (column type, count)
    let mut structs: Map<u32, Vec<u32>> = Map::new(); // id -> member type ids
    let mut pointers: Map<u32, (u32, u32)> = Map::new(); // id -> (storage, pointee)
    let mut sampled_images: Vec<u32> = Vec::new(); // OpTypeSampledImage/Image ids
    let mut variables: Vec<(u32, u32, u32)> = Vec::new(); // (id, pointer type, storage)

    // A packed, nul-terminated literal string starting at `words[i]`.
    let read_string = |ws: &[u32]| -> String {
        let mut bytes = Vec::new();
        'outer: for w in ws {
            for b in w.to_le_bytes() {
                if b == 0 {
                    break 'outer;
                }
                bytes.push(b);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    };

    let mut i = 5;
    while i < words.len() {
        let opcode = words[i] & 0xFFFF;
        let len = (words[i] >> 16) as usize;
        if len == 0 || i + len > words.len() {
            return Err("malformed SPIR-V (bad instruction length)".to_string());
        }
        let ops = &words[i + 1..i + len];
        match opcode {
            OP_MEMBER_NAME if ops.len() >= 3 => {
                member_names.insert((ops[0], ops[1]), read_string(&ops[2..]));
            }
            OP_NAME => {} // ids' own names are not needed, members carry theirs
            OP_DECORATE if ops.len() >= 2 => match ops[1] {
                DEC_BLOCK => blocks.push(ops[0]),
                DEC_BINDING if ops.len() >= 3 => {
                    bindings.insert(ops[0], ops[2]);
                }
                _ => {}
            },
            OP_MEMBER_DECORATE if ops.len() >= 4 && ops[2] == DEC_OFFSET => {
                member_offsets.insert((ops[0], ops[1]), ops[3]);
            }
            OP_TYPE_INT | OP_TYPE_FLOAT if ops.len() >= 2 => {
                scalars.insert(ops[0], (ops[1] / 8) as usize);
            }
            OP_TYPE_VECTOR if ops.len() >= 3 => {
                vectors.insert(ops[0], (ops[1], ops[2]));
            }
            OP_TYPE_MATRIX if ops.len() >= 3 => {
                matrices.insert(ops[0], (ops[1], ops[2]));
            }
            OP_TYPE_IMAGE | OP_TYPE_SAMPLED_IMAGE if !ops.is_empty() => {
                sampled_images.push(ops[0]);
            }
            OP_TYPE_STRUCT if !ops.is_empty() => {
                structs.insert(ops[0], ops[1..].to_vec());
            }
            OP_TYPE_POINTER if ops.len() >= 3 => {
                pointers.insert(ops[0], (ops[1], ops[2]));
            }
            OP_VARIABLE if ops.len() >= 3 => {
                variables.push((ops[1], ops[0], ops[2]));
            }
            _ => {}
        }
        i += len;
    }

    let size_of = |ty: u32| -> usize {
        if let Some(&sz) = scalars.get(&ty) {
            return sz;
        }
        if let Some(&(comp, n)) = vectors.get(&ty) {
            return scalars.get(&comp).copied().unwrap_or(4) * n as usize;
        }
        if let Some(&(col, n)) = matrices.get(&ty) {
            let col_sz = vectors
                .get(&col)
                .map(|&(c, cn)| scalars.get(&c).copied().unwrap_or(4) * cn as usize)
                .unwrap_or(16);
            // std140 pads matrix columns to vec4 strides; gfx's mat4 is
            // exactly 4 vec4 columns, so col stride == col size here.
            return col_sz * n as usize;
        }
        0
    };

    let mut out = SpirvReflection {
        ubo_binding: None,
        ubo_size: 0,
        members: Vec::new(),
        sampler_bindings: Vec::new(),
    };
    for &(var_id, ptr_ty, storage) in &variables {
        let Some(&(_ptr_storage, pointee)) = pointers.get(&ptr_ty) else {
            continue;
        };
        if storage == STORAGE_UNIFORM && blocks.contains(&pointee) {
            let Some(member_types) = structs.get(&pointee) else {
                continue;
            };
            out.ubo_binding = Some(bindings.get(&var_id).copied().unwrap_or(0));
            for (idx, &mty) in member_types.iter().enumerate() {
                let name = member_names
                    .get(&(pointee, idx as u32))
                    .cloned()
                    .unwrap_or_default();
                let offset = member_offsets
                    .get(&(pointee, idx as u32))
                    .copied()
                    .unwrap_or(0) as usize;
                let size = size_of(mty);
                out.ubo_size = out.ubo_size.max(offset + size);
                out.members.push((name, offset, size));
            }
        } else if storage == STORAGE_UNIFORM_CONSTANT && sampled_images.contains(&pointee) {
            if let Some(&b) = bindings.get(&var_id) {
                out.sampler_bindings.push(b);
            }
        }
    }
    out.ubo_size = out.ubo_size.next_multiple_of(16);
    out.sampler_bindings.sort_unstable();
    Ok(out)
}

impl Chain {
    // -----------------------------------------------------------------
    // gfx.* — the Vulkan draw-call surface (SPEC § 7.4's Vulkan notes).
    // Consumed through the platform shims' forwards; observable
    // semantics match the GL and Metal backends, with SPIR-V binaries as
    // the one deliberate per-backend difference in shader input
    // (`gfx.compile_program_spirv`; `win.backend_name()` is the branch
    // point, exactly as GLSL-vs-MSL already is).
    // -----------------------------------------------------------------

    /// `gfx.compile_program_spirv`: validate both blobs, create the
    /// modules, reflect each stage's uniform block + the fragment stage's
    /// samplers, and bake everything draw-independent (set layout,
    /// pipeline layout, descriptor set, UBO buffers). Pipelines themselves
    /// are baked lazily per (vertex layout, depth state) at draw time.
    pub(crate) fn compile_program_spirv(&mut self, vs: &[u8], fs: &[u8]) -> Result<u32, String> {
        let vs_words = spirv_words("vertex", vs)?;
        let fs_words = spirv_words("fragment", fs)?;
        let vs_refl = reflect_spirv(&vs_words).map_err(|e| format!("vertex shader: {e}"))?;
        let fs_refl = reflect_spirv(&fs_words).map_err(|e| format!("fragment shader: {e}"))?;

        // Safety: every create below is checked, and everything created
        // before a failure is destroyed before returning `Err` (manual
        // unwind, same discipline as [`Chain::create`]).
        unsafe {
            let vs_mod = self
                .make_shader_module(&vs_words)
                .map_err(|e| format!("vertex shader: {e}"))?;
            let fs_mod = match self.make_shader_module(&fs_words) {
                Ok(m) => m,
                Err(e) => {
                    (self.fns.destroy_shader_module)(self.device, vs_mod, ptr::null());
                    return Err(format!("fragment shader: {e}"));
                }
            };
            macro_rules! unwind {
                ($($handle:expr => $destroy:ident),* $(,)?) => {{
                    $(if $handle != 0 {
                        (self.fns.$destroy)(self.device, $handle, ptr::null());
                    })*
                    (self.fns.destroy_shader_module)(self.device, vs_mod, ptr::null());
                    (self.fns.destroy_shader_module)(self.device, fs_mod, ptr::null());
                }};
            }

            // Descriptor set layout: the stage UBOs plus the fragment
            // samplers, bindings straight from the reflection.
            let mut layout_bindings: Vec<VkDescriptorSetLayoutBinding> = Vec::new();
            if let Some(b) = vs_refl.ubo_binding {
                layout_bindings.push(VkDescriptorSetLayoutBinding {
                    binding: b,
                    descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
                    descriptor_count: 1,
                    stage_flags: VK_SHADER_STAGE_VERTEX_BIT,
                    p_immutable_samplers: ptr::null(),
                });
            }
            if let Some(b) = fs_refl.ubo_binding {
                layout_bindings.push(VkDescriptorSetLayoutBinding {
                    binding: b,
                    descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
                    descriptor_count: 1,
                    stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT,
                    p_immutable_samplers: ptr::null(),
                });
            }
            for &b in &fs_refl.sampler_bindings {
                layout_bindings.push(VkDescriptorSetLayoutBinding {
                    binding: b,
                    descriptor_type: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    descriptor_count: 1,
                    stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT,
                    p_immutable_samplers: ptr::null(),
                });
            }
            let dslci = VkDescriptorSetLayoutCreateInfo {
                s_type: ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                binding_count: layout_bindings.len() as u32,
                p_bindings: layout_bindings.as_ptr(),
            };
            let mut dsl: VkDescriptorSetLayout = 0;
            let r =
                (self.fns.create_descriptor_set_layout)(self.device, &dslci, ptr::null(), &mut dsl);
            if r != VK_SUCCESS {
                unwind!();
                return Err(format!(
                    "gfx.compile_program_spirv: vkCreateDescriptorSetLayout failed ({r})"
                ));
            }
            let plci = VkPipelineLayoutCreateInfo {
                s_type: ST_PIPELINE_LAYOUT_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                set_layout_count: 1,
                p_set_layouts: &dsl,
                push_constant_range_count: 0,
                p_push_constant_ranges: ptr::null(),
            };
            let mut playout: VkPipelineLayout = 0;
            let r =
                (self.fns.create_pipeline_layout)(self.device, &plci, ptr::null(), &mut playout);
            if r != VK_SUCCESS {
                unwind!(dsl => destroy_descriptor_set_layout);
                return Err(format!(
                    "gfx.compile_program_spirv: vkCreatePipelineLayout failed ({r})"
                ));
            }

            // Descriptor pool + the program's one set.
            let ubo_count = layout_bindings
                .iter()
                .filter(|b| b.descriptor_type == VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER)
                .count() as u32;
            let sampler_count = fs_refl.sampler_bindings.len() as u32;
            let mut pool_sizes: Vec<VkDescriptorPoolSize> = Vec::new();
            if ubo_count > 0 {
                pool_sizes.push(VkDescriptorPoolSize {
                    ty: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
                    descriptor_count: ubo_count,
                });
            }
            if sampler_count > 0 {
                pool_sizes.push(VkDescriptorPoolSize {
                    ty: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    descriptor_count: sampler_count,
                });
            }
            let mut dpool: VkDescriptorPool = 0;
            let mut dset: VkDescriptorSet = 0;
            if !pool_sizes.is_empty() {
                let dpci = VkDescriptorPoolCreateInfo {
                    s_type: ST_DESCRIPTOR_POOL_CREATE_INFO,
                    p_next: ptr::null(),
                    flags: 0,
                    max_sets: 1,
                    pool_size_count: pool_sizes.len() as u32,
                    p_pool_sizes: pool_sizes.as_ptr(),
                };
                let r =
                    (self.fns.create_descriptor_pool)(self.device, &dpci, ptr::null(), &mut dpool);
                if r != VK_SUCCESS {
                    unwind!(playout => destroy_pipeline_layout, dsl => destroy_descriptor_set_layout);
                    return Err(format!(
                        "gfx.compile_program_spirv: vkCreateDescriptorPool failed ({r})"
                    ));
                }
                let dsai = VkDescriptorSetAllocateInfo {
                    s_type: ST_DESCRIPTOR_SET_ALLOCATE_INFO,
                    p_next: ptr::null(),
                    descriptor_pool: dpool,
                    descriptor_set_count: 1,
                    p_set_layouts: &dsl,
                };
                let r = (self.fns.allocate_descriptor_sets)(self.device, &dsai, &mut dset);
                if r != VK_SUCCESS {
                    unwind!(dpool => destroy_descriptor_pool, playout => destroy_pipeline_layout, dsl => destroy_descriptor_set_layout);
                    return Err(format!(
                        "gfx.compile_program_spirv: vkAllocateDescriptorSets failed ({r})"
                    ));
                }
            }

            // Per-stage UBO buffers (persistently mapped; synchronous
            // draws make single-buffer reuse race-free), written into the
            // descriptor set once here.
            let make_stage = |me: &mut Self,
                              refl: &SpirvReflection|
             -> Result<(StageUniforms, UboBuffer), String> {
                let ubo = if refl.ubo_size > 0 {
                    me.make_host_buffer(refl.ubo_size, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT)?
                } else {
                    UboBuffer {
                        buf: 0,
                        mem: 0,
                        ptr: ptr::null_mut(),
                    }
                };
                Ok((
                    StageUniforms {
                        binding: refl.ubo_binding,
                        size: refl.ubo_size,
                        members: refl.members.clone(),
                        staging: vec![0u8; refl.ubo_size],
                    },
                    ubo,
                ))
            };
            let (vs_uniforms, vs_ubo) = match make_stage(self, &vs_refl) {
                Ok(v) => v,
                Err(e) => {
                    unwind!(dpool => destroy_descriptor_pool, playout => destroy_pipeline_layout, dsl => destroy_descriptor_set_layout);
                    return Err(e);
                }
            };
            let (fs_uniforms, fs_ubo) = match make_stage(self, &fs_refl) {
                Ok(v) => v,
                Err(e) => {
                    self.free_ubo(&vs_ubo);
                    unwind!(dpool => destroy_descriptor_pool, playout => destroy_pipeline_layout, dsl => destroy_descriptor_set_layout);
                    return Err(e);
                }
            };
            for (uniforms, ubo) in [(&vs_uniforms, &vs_ubo), (&fs_uniforms, &fs_ubo)] {
                if let (Some(binding), true) = (uniforms.binding, ubo.buf != 0) {
                    let info = VkDescriptorBufferInfo {
                        buffer: ubo.buf,
                        offset: 0,
                        range: uniforms.size as u64,
                    };
                    let write = VkWriteDescriptorSet {
                        s_type: ST_WRITE_DESCRIPTOR_SET,
                        p_next: ptr::null(),
                        dst_set: dset,
                        dst_binding: binding,
                        dst_array_element: 0,
                        descriptor_count: 1,
                        descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
                        p_image_info: ptr::null(),
                        p_buffer_info: &info,
                        p_texel_buffer_view: ptr::null(),
                    };
                    (self.fns.update_descriptor_sets)(self.device, 1, &write, 0, ptr::null());
                }
            }

            let handle = self.next_program;
            self.next_program += 1;
            self.programs.insert(
                handle,
                Program {
                    vs: vs_mod,
                    fs: fs_mod,
                    dsl,
                    playout,
                    dpool,
                    dset,
                    vs_uniforms,
                    fs_uniforms,
                    vs_ubo,
                    fs_ubo,
                    texture_bindings: fs_refl.sampler_bindings.clone(),
                    pipelines: HashMap::new(),
                },
            );
            Ok(handle)
        }
    }

    unsafe fn make_shader_module(&mut self, words: &[u32]) -> Result<VkShaderModule, String> {
        let smci = VkShaderModuleCreateInfo {
            s_type: ST_SHADER_MODULE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            code_size: words.len() * 4,
            p_code: words.as_ptr(),
        };
        let mut module: VkShaderModule = 0;
        let r = (self.fns.create_shader_module)(self.device, &smci, ptr::null(), &mut module);
        if r != VK_SUCCESS {
            return Err(format!("vkCreateShaderModule failed ({r})"));
        }
        Ok(module)
    }

    /// A host-visible+coherent buffer, persistently mapped.
    unsafe fn make_host_buffer(&mut self, size: usize, usage: u32) -> Result<UboBuffer, String> {
        let bci = VkBufferCreateInfo {
            s_type: ST_BUFFER_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            size: size as u64,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
        };
        let mut buf: VkBuffer = 0;
        let r = (self.fns.create_buffer)(self.device, &bci, ptr::null(), &mut buf);
        if r != VK_SUCCESS {
            return Err(format!("window.create_vulkan: vkCreateBuffer failed ({r})"));
        }
        let mut req: VkMemoryRequirements = std::mem::zeroed();
        (self.fns.get_buffer_memory_requirements)(self.device, buf, &mut req);
        let mem = match self.alloc_memory(
            &req,
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
        ) {
            Ok(m) => m,
            Err(e) => {
                (self.fns.destroy_buffer)(self.device, buf, ptr::null());
                return Err(e);
            }
        };
        let r = (self.fns.bind_buffer_memory)(self.device, buf, mem, 0);
        if r != VK_SUCCESS {
            (self.fns.destroy_buffer)(self.device, buf, ptr::null());
            (self.fns.free_memory)(self.device, mem, ptr::null());
            return Err(format!(
                "window.create_vulkan: vkBindBufferMemory failed ({r})"
            ));
        }
        let mut ptr_out: *mut c_void = ptr::null_mut();
        let r = (self.fns.map_memory)(self.device, mem, 0, size as u64, 0, &mut ptr_out);
        if r != VK_SUCCESS {
            (self.fns.destroy_buffer)(self.device, buf, ptr::null());
            (self.fns.free_memory)(self.device, mem, ptr::null());
            return Err(format!("window.create_vulkan: vkMapMemory failed ({r})"));
        }
        Ok(UboBuffer {
            buf,
            mem,
            ptr: ptr_out as *mut u8,
        })
    }

    unsafe fn free_ubo(&mut self, ubo: &UboBuffer) {
        if ubo.buf != 0 {
            (self.fns.destroy_buffer)(self.device, ubo.buf, ptr::null());
        }
        if ubo.mem != 0 {
            // Freeing mapped memory implicitly unmaps it (spec).
            (self.fns.free_memory)(self.device, ubo.mem, ptr::null());
        }
    }

    unsafe fn destroy_program(&mut self, handle: u32) {
        let Some(prog) = self.programs.remove(&handle) else {
            return;
        };
        (self.fns.device_wait_idle)(self.device);
        for (_, pipeline) in prog.pipelines {
            (self.fns.destroy_pipeline)(self.device, pipeline, ptr::null());
        }
        self.free_ubo(&prog.vs_ubo);
        self.free_ubo(&prog.fs_ubo);
        if prog.dpool != 0 {
            // Destroying the pool frees its descriptor set.
            (self.fns.destroy_descriptor_pool)(self.device, prog.dpool, ptr::null());
        }
        (self.fns.destroy_pipeline_layout)(self.device, prog.playout, ptr::null());
        (self.fns.destroy_descriptor_set_layout)(self.device, prog.dsl, ptr::null());
        (self.fns.destroy_shader_module)(self.device, prog.vs, ptr::null());
        (self.fns.destroy_shader_module)(self.device, prog.fs, ptr::null());
    }

    unsafe fn destroy_gfx_buffer(&mut self, handle: u32) {
        let Some(b) = self.buffers.remove(&handle) else {
            return;
        };
        (self.fns.device_wait_idle)(self.device);
        if b.buf != 0 {
            (self.fns.destroy_buffer)(self.device, b.buf, ptr::null());
        }
        if b.mem != 0 {
            (self.fns.free_memory)(self.device, b.mem, ptr::null());
        }
    }

    unsafe fn destroy_gfx_texture(&mut self, handle: u32) {
        let Some(t) = self.textures.remove(&handle) else {
            return;
        };
        (self.fns.device_wait_idle)(self.device);
        if t.view != 0 {
            (self.fns.destroy_image_view)(self.device, t.view, ptr::null());
        }
        if t.image != 0 {
            (self.fns.destroy_image)(self.device, t.image, ptr::null());
        }
        if t.memory != 0 {
            (self.fns.free_memory)(self.device, t.memory, ptr::null());
        }
    }

    // ---- the gfx.* methods proper (forwarded by the platform shims) ----

    pub(crate) fn use_program(&mut self, program: u32) {
        self.current_program = program;
    }

    pub(crate) fn delete_program(&mut self, program: u32) {
        unsafe { self.destroy_program(program) };
    }

    pub(crate) fn create_buffer(&mut self) -> u32 {
        let handle = self.next_buffer;
        self.next_buffer += 1;
        self.buffers.insert(
            handle,
            GfxBuffer {
                buf: 0,
                mem: 0,
                ptr: ptr::null_mut(),
                cap: 0,
            },
        );
        handle
    }

    pub(crate) fn delete_buffer(&mut self, buffer: u32) {
        unsafe { self.destroy_gfx_buffer(buffer) };
    }

    pub(crate) fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match kind {
            crate::window::GfxBufferKind::Vertex => self.bound_array_buffer = buffer,
            // The element-array binding is VAO state on GL; mirror that.
            crate::window::GfxBufferKind::Index => {
                self.vaos.entry(self.bound_vao).or_default().element_buffer = buffer;
            }
        }
    }

    pub(crate) fn upload_buffer(
        &mut self,
        kind: crate::window::GfxBufferKind,
        data: &[u8],
        _dynamic: bool,
    ) {
        let handle = match kind {
            crate::window::GfxBufferKind::Vertex => self.bound_array_buffer,
            crate::window::GfxBufferKind::Index => self
                .vaos
                .get(&self.bound_vao)
                .map(|v| v.element_buffer)
                .unwrap_or(0),
        };
        if handle == 0 || !self.buffers.contains_key(&handle) || data.is_empty() {
            return;
        }
        // Grow-by-recreate when the store is too small (glBufferData
        // semantics: a fresh data store each call) — synchronous draws
        // mean the old buffer can never still be in flight.
        unsafe {
            let need = data.len();
            let cap = self.buffers[&handle].cap;
            if cap < need {
                (self.fns.device_wait_idle)(self.device);
                let old = self.buffers.get(&handle).unwrap();
                if old.buf != 0 {
                    (self.fns.destroy_buffer)(self.device, old.buf, ptr::null());
                }
                if old.mem != 0 {
                    (self.fns.free_memory)(self.device, old.mem, ptr::null());
                }
                let Ok(ubo) = self.make_host_buffer(
                    need,
                    VK_BUFFER_USAGE_VERTEX_BUFFER_BIT
                        | VK_BUFFER_USAGE_INDEX_BUFFER_BIT
                        | VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                ) else {
                    self.buffers.insert(
                        handle,
                        GfxBuffer {
                            buf: 0,
                            mem: 0,
                            ptr: ptr::null_mut(),
                            cap: 0,
                        },
                    );
                    return;
                };
                self.buffers.insert(
                    handle,
                    GfxBuffer {
                        buf: ubo.buf,
                        mem: ubo.mem,
                        ptr: ubo.ptr,
                        cap: need,
                    },
                );
            }
            let b = &self.buffers[&handle];
            ptr::copy_nonoverlapping(data.as_ptr(), b.ptr, data.len());
        }
    }

    pub(crate) fn create_vertex_array(&mut self) -> u32 {
        let handle = self.next_vao;
        self.next_vao += 1;
        self.vaos.insert(handle, VaoState::default());
        handle
    }

    pub(crate) fn bind_vertex_array(&mut self, vao: u32) {
        self.bound_vao = vao;
    }

    pub(crate) fn delete_vertex_array(&mut self, vao: u32) {
        self.vaos.remove(&vao);
        if self.bound_vao == vao {
            self.bound_vao = 0;
        }
    }

    pub(crate) fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        let buffer = self.bound_array_buffer;
        let vao = self.vaos.entry(self.bound_vao).or_default();
        vao.attribs.retain(|a| a.0 != index);
        vao.attribs.push((index, size, stride, offset, buffer));
        vao.attribs.sort_unstable_by_key(|a| a.0);
    }

    pub(crate) fn disable_vertex_attrib(&mut self, index: u32) {
        if let Some(vao) = self.vaos.get_mut(&self.bound_vao) {
            vao.attribs.retain(|a| a.0 != index);
        }
    }

    pub(crate) fn create_texture(&mut self) -> u32 {
        let handle = self.next_texture;
        self.next_texture += 1;
        self.textures.insert(
            handle,
            GfxTexture {
                image: 0,
                memory: 0,
                view: 0,
            },
        );
        handle
    }

    pub(crate) fn delete_texture(&mut self, tex: u32) {
        unsafe { self.destroy_gfx_texture(tex) };
        for unit in &mut self.texture_units {
            if *unit == tex {
                *unit = 0;
            }
        }
    }

    pub(crate) fn bind_texture(&mut self, tex: u32) {
        self.texture_units[self.active_unit] = tex;
    }

    pub(crate) fn active_texture_unit(&mut self, unit: u32) {
        self.active_unit = (unit as usize).min(self.texture_units.len() - 1);
    }

    pub(crate) fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        let handle = self.texture_units[self.active_unit];
        if handle == 0 || !self.textures.contains_key(&handle) || width <= 0 || height <= 0 {
            return;
        }
        // RGB uploads are expanded to RGBA CPU-side: 3-channel formats
        // have no guaranteed sampled-image support in Vulkan, and the
        // sampled result is identical with alpha forced to 1.
        let (w, h) = (width as usize, height as usize);
        let rgba: Vec<u8> = if has_alpha {
            data.to_vec()
        } else {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in data.chunks_exact(3).take(w * h) {
                out.extend_from_slice(px);
                out.push(255);
            }
            out
        };
        if rgba.len() < w * h * 4 {
            return;
        }
        unsafe {
            // Fresh image per upload (glTexImage2D allocates a new level
            // store each call); destroy any previous one.
            let old = self.textures.get(&handle).unwrap();
            if old.image != 0 {
                (self.fns.device_wait_idle)(self.device);
                if old.view != 0 {
                    (self.fns.destroy_image_view)(self.device, old.view, ptr::null());
                }
                (self.fns.destroy_image)(self.device, old.image, ptr::null());
                (self.fns.free_memory)(self.device, old.memory, ptr::null());
            }
            let extent = VkExtent2D {
                width: width as u32,
                height: height as u32,
            };
            let Ok((image, memory)) = self.create_image_2d(
                VK_FORMAT_R8G8B8A8_UNORM,
                extent,
                VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
            ) else {
                self.textures.insert(
                    handle,
                    GfxTexture {
                        image: 0,
                        memory: 0,
                        view: 0,
                    },
                );
                return;
            };
            let Ok(view) =
                self.create_view(image, VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_ASPECT_COLOR_BIT)
            else {
                (self.fns.destroy_image)(self.device, image, ptr::null());
                (self.fns.free_memory)(self.device, memory, ptr::null());
                return;
            };
            // Staging copy, then hand the image to the fragment stage.
            let Ok(staging) =
                self.make_host_buffer(rgba.len(), VK_BUFFER_USAGE_TRANSFER_DST_BIT | 0x1)
            else {
                (self.fns.destroy_image_view)(self.device, view, ptr::null());
                (self.fns.destroy_image)(self.device, image, ptr::null());
                (self.fns.free_memory)(self.device, memory, ptr::null());
                return;
            };
            ptr::copy_nonoverlapping(rgba.as_ptr(), staging.ptr, rgba.len());
            let copy = VkBufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: SUBRESOURCE_COLOR_LAYERS,
                image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                image_extent: VkExtent3D {
                    width: width as u32,
                    height: height as u32,
                    depth: 1,
                },
            };
            self.one_shot(|fns, cmd| {
                Self::barrier(
                    fns,
                    cmd,
                    image,
                    VK_IMAGE_LAYOUT_UNDEFINED,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    0,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                );
                (fns.cmd_copy_buffer_to_image)(
                    cmd,
                    staging.buf,
                    image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    1,
                    &copy,
                );
                Self::barrier(
                    fns,
                    cmd,
                    image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                );
            });
            self.free_ubo(&staging);
            self.textures.insert(
                handle,
                GfxTexture { image, memory, view },
            );
        }
    }

    /// Write `bytes` at `name`'s reflected offset in whichever stage
    /// declares it (either or both) — unknown names are silently ignored,
    /// GL's own unknown-uniform behavior. Also makes `program` current,
    /// matching the GL backend's `uniform_location` side effect.
    fn stage_uniform(&mut self, program: u32, name: &str, bytes: &[u8]) {
        self.current_program = program;
        let Some(prog) = self.programs.get_mut(&program) else {
            return;
        };
        for uniforms in [&mut prog.vs_uniforms, &mut prog.fs_uniforms] {
            for (member, offset, size) in &uniforms.members {
                if member == name {
                    let n = bytes.len().min(*size.max(&bytes.len()));
                    let end = (offset + n).min(uniforms.staging.len());
                    if *offset < end {
                        uniforms.staging[*offset..end].copy_from_slice(&bytes[..end - offset]);
                    }
                }
            }
        }
    }

    pub(crate) fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        self.stage_uniform(program, name, &v.to_le_bytes());
    }
    pub(crate) fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        self.stage_uniform(program, name, &v.to_le_bytes());
    }
    pub(crate) fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&x.to_le_bytes());
        b[4..].copy_from_slice(&y.to_le_bytes());
        self.stage_uniform(program, name, &b);
    }
    pub(crate) fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&x.to_le_bytes());
        b[4..8].copy_from_slice(&y.to_le_bytes());
        b[8..].copy_from_slice(&z.to_le_bytes());
        self.stage_uniform(program, name, &b);
    }
    pub(crate) fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&x.to_le_bytes());
        b[4..8].copy_from_slice(&y.to_le_bytes());
        b[8..12].copy_from_slice(&z.to_le_bytes());
        b[12..].copy_from_slice(&w.to_le_bytes());
        self.stage_uniform(program, name, &b);
    }
    pub(crate) fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        let mut b = [0u8; 64];
        for (i, v) in values.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.stage_uniform(program, name, &b);
    }

    pub(crate) fn draw_arrays(&mut self, first: i32, count: i32) {
        self.encode_draw(None, (first, count));
    }

    pub(crate) fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        self.encode_draw(Some(byte_offset), (0, count));
    }

    /// `gfx.clear`: color and depth, into the offscreen pair.
    pub(crate) fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        let color = [r, g, b, a];
        let off_image = self.off_image;
        let depth_image = self.depth_image;
        let off_layout = self.off_layout;
        let depth_layout = self.depth_layout;
        // Safety: same tracked-layout one-shot discipline as `clear`.
        unsafe {
            self.one_shot(|fns, cmd| {
                Self::barrier(
                    fns,
                    cmd,
                    off_image,
                    off_layout,
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
                Self::barrier_range(
                    fns,
                    cmd,
                    depth_image,
                    depth_layout,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    0,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    SUBRESOURCE_DEPTH,
                );
                // depth = 1.0 (the far plane), stencil unused on D32.
                let ds = [1.0f32, 0.0];
                (fns.cmd_clear_depth_stencil_image)(
                    cmd,
                    depth_image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    &ds,
                    1,
                    &SUBRESOURCE_DEPTH,
                );
            });
        }
        self.off_layout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        self.depth_layout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    }

    pub(crate) fn set_depth_test(&mut self, enabled: bool) {
        self.depth_test = enabled;
    }

    pub(crate) fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.viewport = Some((x, y, w, h));
    }

    /// `glReadPixels` parity: bottom-left-origin rect, bottom-up RGBA rows
    /// — row-reversed and (for the BGRA offscreen format) swizzled from
    /// the copied top-down image rows, the exact Metal read_pixels recipe.
    pub(crate) fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let len = (w.max(0) as usize) * (h.max(0) as usize) * 4;
        let mut out = vec![0u8; len];
        if len == 0 {
            return out;
        }
        // Clamp to the image (a copy outside bounds is invalid in Vulkan;
        // GL leaves out-of-window reads undefined, so zeros are fine).
        let iw = self.extent.width as i32;
        let ih = self.extent.height as i32;
        let vk_y = ih - y - h;
        if x < 0 || vk_y < 0 || w <= 0 || h <= 0 || x + w > iw || vk_y + h > ih {
            return out;
        }
        unsafe {
            let Ok(readback) = self.make_host_buffer(len, VK_BUFFER_USAGE_TRANSFER_DST_BIT) else {
                return out;
            };
            let off_image = self.off_image;
            let off_layout = self.off_layout;
            let copy = VkBufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: SUBRESOURCE_COLOR_LAYERS,
                image_offset: VkOffset3D { x, y: vk_y, z: 0 },
                image_extent: VkExtent3D {
                    width: w as u32,
                    height: h as u32,
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
                    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT | VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_ACCESS_TRANSFER_READ_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT | VK_PIPELINE_STAGE_TRANSFER_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                );
                (fns.cmd_copy_image_to_buffer)(
                    cmd,
                    off_image,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    readback.buf,
                    1,
                    &copy,
                );
            });
            self.off_layout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
            if ok {
                let src = std::slice::from_raw_parts(readback.ptr, len);
                let row = (w as usize) * 4;
                let bgra = self.format == VK_FORMAT_B8G8R8A8_UNORM;
                for j in 0..h as usize {
                    let src_row = &src[(h as usize - 1 - j) * row..][..row];
                    let dst_row = &mut out[j * row..][..row];
                    if bgra {
                        for (d, sp) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
                            d[0] = sp[2];
                            d[1] = sp[1];
                            d[2] = sp[0];
                            d[3] = sp[3];
                        }
                    } else {
                        dst_row.copy_from_slice(src_row);
                    }
                }
            }
            self.free_ubo(&readback);
        }
        out
    }

    /// Bake (or fetch) the pipeline for the current (program, vertex
    /// layout, depth state), then record one render pass around one draw —
    /// the Metal backend's encode_draw, in Vulkan.
    fn encode_draw(&mut self, indexed_offset: Option<i32>, (first, count): (i32, i32)) {
        if count <= 0 || self.extent.width == 0 || self.extent.height == 0 {
            return;
        }
        let program = self.current_program;
        if !self.programs.contains_key(&program) {
            return;
        }
        let vao = self.vaos.get(&self.bound_vao).cloned().unwrap_or_default();
        // Vertex buffers must exist with real stores before a draw can
        // reference them (GL would render garbage; Vulkan must not).
        for &(_, _, _, _, buffer) in &vao.attribs {
            if self.buffers.get(&buffer).map(|b| b.buf).unwrap_or(0) == 0 {
                return;
            }
        }
        let index_buffer = if indexed_offset.is_some() {
            let b = self
                .buffers
                .get(&vao.element_buffer)
                .map(|b| b.buf)
                .unwrap_or(0);
            if b == 0 {
                return;
            }
            b
        } else {
            0
        };

        // Fingerprint: what the pipeline's vertex-input state depends on
        // (per-attrib index/size/stride; offsets and buffer identities are
        // bind-time state). FNV-1a, the Metal backend's exact recipe.
        let mut fp: u64 = 0xcbf2_9ce4_8422_2325;
        for &(index, size, stride, _, _) in &vao.attribs {
            for v in [index as u64, size as u64, stride as u64] {
                fp ^= v;
                fp = fp.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let depth_test = self.depth_test;

        unsafe {
            if !self.ensure_pipeline(program, fp, &vao, depth_test) {
                return;
            }
            let prog = &self.programs[&program];
            let pipeline = prog.pipelines[&(fp, depth_test)];
            let playout = prog.playout;
            let dset = prog.dset;

            // Upload the staged uniforms (persistently mapped, coherent).
            for (uniforms, ubo) in [
                (&prog.vs_uniforms, &prog.vs_ubo),
                (&prog.fs_uniforms, &prog.fs_ubo),
            ] {
                if ubo.buf != 0 && uniforms.size > 0 {
                    ptr::copy_nonoverlapping(uniforms.staging.as_ptr(), ubo.ptr, uniforms.size);
                }
            }

            // Texture descriptors: binding N samples unit N-2 (SPEC's
            // Vulkan convention). Every declared sampler must resolve to
            // a live texture or the descriptor set would be invalid —
            // skip the draw if not (GL's result would be undefined).
            let texture_bindings = prog.texture_bindings.clone();
            let mut image_infos: Vec<VkDescriptorImageInfo> = Vec::new();
            for &b in &texture_bindings {
                let unit = (b as usize).saturating_sub(2);
                let tex = self.texture_units.get(unit).copied().unwrap_or(0);
                let Some(t) = self.textures.get(&tex) else {
                    return;
                };
                if t.view == 0 {
                    return;
                }
                image_infos.push(VkDescriptorImageInfo {
                    sampler: self.sampler,
                    image_view: t.view,
                    image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                });
            }
            let writes: Vec<VkWriteDescriptorSet> = texture_bindings
                .iter()
                .zip(&image_infos)
                .map(|(&binding, info)| VkWriteDescriptorSet {
                    s_type: ST_WRITE_DESCRIPTOR_SET,
                    p_next: ptr::null(),
                    dst_set: dset,
                    dst_binding: binding,
                    dst_array_element: 0,
                    descriptor_count: 1,
                    descriptor_type: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    p_image_info: info as *const VkDescriptorImageInfo as *const c_void,
                    p_buffer_info: ptr::null(),
                    p_texel_buffer_view: ptr::null(),
                })
                .collect();
            if !writes.is_empty() {
                (self.fns.update_descriptor_sets)(
                    self.device,
                    writes.len() as u32,
                    writes.as_ptr(),
                    0,
                    ptr::null(),
                );
            }

            let extent = self.extent;
            let off_image = self.off_image;
            let depth_image = self.depth_image;
            let off_layout = self.off_layout;
            let depth_layout = self.depth_layout;
            let render_pass = self.render_pass;
            let framebuffer = self.framebuffer;
            // GL's bottom-left-origin viewport, expressed as a
            // maintenance1 negative-height Vulkan viewport so clip-space
            // +Y stays up with no shader involvement.
            let (vx, vy, vw, vh) =
                self.viewport
                    .unwrap_or((0, 0, extent.width as i32, extent.height as i32));
            let vk_viewport = VkViewport {
                x: vx as f32,
                y: (extent.height as i32 - vy) as f32,
                width: vw as f32,
                height: -(vh as f32),
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let scissor = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent,
            };
            let vertex_binds: Vec<(u32, VkBuffer, u64)> = vao
                .attribs
                .iter()
                .map(|&(index, _, _, offset, buffer)| {
                    (index, self.buffers[&buffer].buf, offset as u64)
                })
                .collect();

            self.one_shot(|fns, cmd| {
                Self::barrier(
                    fns,
                    cmd,
                    off_image,
                    off_layout,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    VK_ACCESS_TRANSFER_WRITE_BIT | VK_ACCESS_TRANSFER_READ_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT | VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                );
                Self::barrier_range(
                    fns,
                    cmd,
                    depth_image,
                    depth_layout,
                    VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                    VK_ACCESS_TRANSFER_WRITE_BIT,
                    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT
                        | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT | VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT
                        | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                    SUBRESOURCE_DEPTH,
                );
                let rpbi = VkRenderPassBeginInfo {
                    s_type: ST_RENDER_PASS_BEGIN_INFO,
                    p_next: ptr::null(),
                    render_pass,
                    framebuffer,
                    render_area: VkRect2D {
                        offset: VkOffset2D { x: 0, y: 0 },
                        extent,
                    },
                    clear_value_count: 0,
                    p_clear_values: ptr::null(),
                };
                (fns.cmd_begin_render_pass)(cmd, &rpbi, 0 /* INLINE */);
                (fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
                (fns.cmd_set_viewport)(cmd, 0, 1, &vk_viewport);
                (fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
                (fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    playout,
                    0,
                    if dset != 0 { 1 } else { 0 },
                    &dset,
                    0,
                    ptr::null(),
                );
                for &(binding, buf, offset) in &vertex_binds {
                    (fns.cmd_bind_vertex_buffers)(cmd, binding, 1, &buf, &offset);
                }
                match indexed_offset {
                    Some(byte_offset) => {
                        (fns.cmd_bind_index_buffer)(
                            cmd,
                            index_buffer,
                            byte_offset as u64,
                            VK_INDEX_TYPE_UINT32,
                        );
                        (fns.cmd_draw_indexed)(cmd, count as u32, 1, 0, 0, 0);
                    }
                    None => {
                        (fns.cmd_draw)(cmd, count as u32, 1, first as u32, 0);
                    }
                }
                (fns.cmd_end_render_pass)(cmd);
            });
            self.off_layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
            self.depth_layout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        }
    }

    /// Bake the pipeline for (program, vertex-layout fingerprint, depth
    /// state) if this combination hasn't been seen — Vulkan fuses into one
    /// immutable object what GL binds independently, so this cache is the
    /// bridge (the Metal backend's PSO cache, exactly).
    unsafe fn ensure_pipeline(
        &mut self,
        program: u32,
        fp: u64,
        vao: &VaoState,
        depth_test: bool,
    ) -> bool {
        if self.programs[&program]
            .pipelines
            .contains_key(&(fp, depth_test))
        {
            return true;
        }
        let prog = &self.programs[&program];
        let entry = CString::new("main").unwrap();
        let stages = [
            VkPipelineShaderStageCreateInfo {
                s_type: ST_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_VERTEX_BIT,
                module: prog.vs,
                p_name: entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
            VkPipelineShaderStageCreateInfo {
                s_type: ST_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_FRAGMENT_BIT,
                module: prog.fs,
                p_name: entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
        ];
        let bindings: Vec<VkVertexInputBindingDescription> = vao
            .attribs
            .iter()
            .map(
                |&(index, size, stride, _, _)| VkVertexInputBindingDescription {
                    binding: index,
                    // GL allows stride 0 = tightly packed; Vulkan requires
                    // the real stride.
                    stride: if stride == 0 {
                        (size as u32) * 4
                    } else {
                        stride as u32
                    },
                    input_rate: VK_VERTEX_INPUT_RATE_VERTEX,
                },
            )
            .collect();
        let attributes: Vec<VkVertexInputAttributeDescription> = vao
            .attribs
            .iter()
            .map(
                |&(index, size, _, _, _)| VkVertexInputAttributeDescription {
                    location: index,
                    binding: index,
                    format: match size {
                        1 => VK_FORMAT_R32_SFLOAT,
                        2 => VK_FORMAT_R32G32_SFLOAT,
                        3 => VK_FORMAT_R32G32B32_SFLOAT,
                        _ => VK_FORMAT_R32G32B32A32_SFLOAT,
                    },
                    offset: 0,
                },
            )
            .collect();
        let vertex_input = VkPipelineVertexInputStateCreateInfo {
            s_type: ST_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            vertex_binding_description_count: bindings.len() as u32,
            p_vertex_binding_descriptions: bindings.as_ptr(),
            vertex_attribute_description_count: attributes.len() as u32,
            p_vertex_attribute_descriptions: attributes.as_ptr(),
        };
        let input_assembly = VkPipelineInputAssemblyStateCreateInfo {
            s_type: ST_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            topology: VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
            primitive_restart_enable: 0,
        };
        let viewport_state = VkPipelineViewportStateCreateInfo {
            s_type: ST_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: ptr::null(),
            scissor_count: 1,
            p_scissors: ptr::null(),
        };
        let raster = VkPipelineRasterizationStateCreateInfo {
            s_type: ST_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            depth_clamp_enable: 0,
            rasterizer_discard_enable: 0,
            polygon_mode: VK_POLYGON_MODE_FILL,
            cull_mode: VK_CULL_MODE_NONE,
            front_face: VK_FRONT_FACE_COUNTER_CLOCKWISE,
            depth_bias_enable: 0,
            depth_bias_constant_factor: 0.0,
            depth_bias_clamp: 0.0,
            depth_bias_slope_factor: 0.0,
            line_width: 1.0,
        };
        let multisample = VkPipelineMultisampleStateCreateInfo {
            s_type: ST_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            rasterization_samples: VK_SAMPLE_COUNT_1_BIT,
            sample_shading_enable: 0,
            min_sample_shading: 0.0,
            p_sample_mask: ptr::null(),
            alpha_to_coverage_enable: 0,
            alpha_to_one_enable: 0,
        };
        let stencil = VkStencilOpState {
            fail_op: 0,
            pass_op: 0,
            depth_fail_op: 0,
            compare_op: VK_COMPARE_OP_NEVER,
            compare_mask: 0,
            write_mask: 0,
            reference: 0,
        };
        let depth_stencil = VkPipelineDepthStencilStateCreateInfo {
            s_type: ST_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            depth_test_enable: depth_test as u32,
            depth_write_enable: depth_test as u32,
            depth_compare_op: VK_COMPARE_OP_LESS,
            depth_bounds_test_enable: 0,
            stencil_test_enable: 0,
            front: stencil,
            back: stencil,
            min_depth_bounds: 0.0,
            max_depth_bounds: 1.0,
        };
        let blend_attachment = VkPipelineColorBlendAttachmentState {
            blend_enable: 0,
            src_color_blend_factor: 0,
            dst_color_blend_factor: 0,
            color_blend_op: 0,
            src_alpha_blend_factor: 0,
            dst_alpha_blend_factor: 0,
            alpha_blend_op: 0,
            color_write_mask: 0xF,
        };
        let blend = VkPipelineColorBlendStateCreateInfo {
            s_type: ST_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            logic_op_enable: 0,
            logic_op: 0,
            attachment_count: 1,
            p_attachments: &blend_attachment,
            blend_constants: [0.0; 4],
        };
        let dynamic_states = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
        let dynamic = VkPipelineDynamicStateCreateInfo {
            s_type: ST_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            dynamic_state_count: 2,
            p_dynamic_states: dynamic_states.as_ptr(),
        };
        let gpci = VkGraphicsPipelineCreateInfo {
            s_type: ST_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            stage_count: 2,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vertex_input,
            p_input_assembly_state: &input_assembly,
            p_tessellation_state: ptr::null(),
            p_viewport_state: &viewport_state,
            p_rasterization_state: &raster,
            p_multisample_state: &multisample,
            p_depth_stencil_state: &depth_stencil,
            p_color_blend_state: &blend,
            p_dynamic_state: &dynamic,
            layout: prog.playout,
            render_pass: self.render_pass,
            subpass: 0,
            base_pipeline_handle: 0,
            base_pipeline_index: -1,
        };
        let mut pipeline: VkPipeline = 0;
        let r = (self.fns.create_graphics_pipelines)(
            self.device,
            0,
            1,
            &gpci,
            ptr::null(),
            &mut pipeline,
        );
        if r != VK_SUCCESS || pipeline == 0 {
            return false;
        }
        self.programs
            .get_mut(&program)
            .unwrap()
            .pipelines
            .insert((fp, depth_test), pipeline);
        true
    }

    /// [`Self::barrier`] with an explicit subresource range (depth).
    #[allow(clippy::too_many_arguments)]
    unsafe fn barrier_range(
        fns: &Fns,
        cmd: VkCommandBuffer,
        image: VkImage,
        old_layout: i32,
        new_layout: i32,
        src_access: u32,
        dst_access: u32,
        src_stage: u32,
        dst_stage: u32,
        range: VkImageSubresourceRange,
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
            subresource_range: range,
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
}

/// Validate one SPIR-V blob's framing (whole words, magic) and return the
/// words — `stage` names the failing argument in the `Err`.
fn spirv_words(stage: &str, bytes: &[u8]) -> Result<Vec<u32>, String> {
    if bytes.len() < 20 || !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "{stage} shader: not a SPIR-V module (length {} is not a whole number of words)",
            bytes.len()
        ));
    }
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if words[0] != 0x0723_0203 {
        return Err(format!("{stage} shader: not a SPIR-V module (bad magic)"));
    }
    Ok(words)
}

