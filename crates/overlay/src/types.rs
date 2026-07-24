use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use ash::vk;

pub static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

pub type PfnGetInstanceProcAddr = Option<
    unsafe extern "system" fn(vk::Instance, *const std::os::raw::c_char) -> vk::PFN_vkVoidFunction,
>;
pub type PfnGetDeviceProcAddr = Option<
    unsafe extern "system" fn(vk::Device, *const std::os::raw::c_char) -> vk::PFN_vkVoidFunction,
>;
pub type PfnGetPhysicalDeviceProcAddr = Option<
    unsafe extern "system" fn(vk::PhysicalDevice, *const std::os::raw::c_char) -> vk::PFN_vkVoidFunction,
>;

#[repr(C)]
pub struct NegotiateLoaderLayerInterface {
    pub s_type: vk::StructureType,
    pub p_next: *const std::os::raw::c_void,
    pub loader_layer_interface_version: u32,
    pub pfn_get_instance_proc_addr: PfnGetInstanceProcAddr,
    pub pfn_get_device_proc_addr: PfnGetDeviceProcAddr,
    pub pfn_get_physical_device_proc_addr: PfnGetPhysicalDeviceProcAddr,
}

#[repr(C)]
pub struct LayerInstanceCreateInfo {
    pub s_type: vk::StructureType,
    pub p_next: *const std::os::raw::c_void,
    pub function: u32,
    pub p_layer_info: *mut LayerInstanceLink,
}

#[repr(C)]
pub struct LayerInstanceLink {
    pub p_next: *mut LayerInstanceLink,
    pub next_gipa: PfnGetInstanceProcAddr,
    pub next_gpdpa: PfnGetPhysicalDeviceProcAddr,
}

#[repr(C)]
pub struct LayerDeviceCreateInfo {
    pub s_type: vk::StructureType,
    pub p_next: *const std::os::raw::c_void,
    pub function: u32,
    pub p_layer_info: *mut LayerDeviceLink,
}

#[repr(C)]
pub struct LayerDeviceLink {
    pub p_next: *mut LayerDeviceLink,
    pub next_gipa: PfnGetInstanceProcAddr,
    pub next_gdpa: PfnGetDeviceProcAddr,
}

pub struct InstanceData {
    pub instance: vk::Instance,
    pub loader_gipa: PfnGetInstanceProcAddr,
}

pub struct DeviceData {
    pub next_gdpa: PfnGetDeviceProcAddr,
    pub destroy_device: vk::PFN_vkDestroyDevice,
    pub physical_device: vk::PhysicalDevice,
    pub fns: Option<DeviceFns>,
}

#[derive(Clone)]
pub struct SwapchainData {
    pub device: vk::Device,
    pub fns: DeviceFns,
    pub images: Vec<vk::Image>,
    pub extent: vk::Extent2D,
    pub render_pass: vk::RenderPass,
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub shader_vert: vk::ShaderModule,
    pub shader_frag: vk::ShaderModule,
    pub framebuffers: Vec<vk::Framebuffer>,
    pub image_views: Vec<vk::ImageView>,
    pub cmd_pool: vk::CommandPool,
    pub cmd_buffers: Vec<vk::CommandBuffer>,
    pub semaphores: Vec<vk::Semaphore>,
    pub fences: Vec<vk::Fence>,
    pub ui_renderer: Option<crate::ui::UiRenderer>,
}

#[derive(Clone, Copy)]
pub struct DeviceFns {
    pub create_swapchain: vk::PFN_vkCreateSwapchainKHR,
    pub destroy_swapchain: vk::PFN_vkDestroySwapchainKHR,
    pub get_swapchain_images: vk::PFN_vkGetSwapchainImagesKHR,
    pub queue_present: vk::PFN_vkQueuePresentKHR,
    pub queue_submit: vk::PFN_vkQueueSubmit,
    pub create_command_pool: vk::PFN_vkCreateCommandPool,
    pub destroy_command_pool: vk::PFN_vkDestroyCommandPool,
    pub allocate_cmd_buffers: vk::PFN_vkAllocateCommandBuffers,
    pub begin_cmd_buffer: vk::PFN_vkBeginCommandBuffer,
    pub end_cmd_buffer: vk::PFN_vkEndCommandBuffer,
    pub reset_cmd_buffer: vk::PFN_vkResetCommandBuffer,
    pub create_render_pass: vk::PFN_vkCreateRenderPass,
    pub destroy_render_pass: vk::PFN_vkDestroyRenderPass,
    pub create_graphics_pipelines: vk::PFN_vkCreateGraphicsPipelines,
    pub destroy_pipeline: vk::PFN_vkDestroyPipeline,
    pub create_shader_module: vk::PFN_vkCreateShaderModule,
    pub destroy_shader_module: vk::PFN_vkDestroyShaderModule,
    pub create_pipeline_layout: vk::PFN_vkCreatePipelineLayout,
    pub destroy_pipeline_layout: vk::PFN_vkDestroyPipelineLayout,
    pub create_framebuffer: vk::PFN_vkCreateFramebuffer,
    pub destroy_framebuffer: vk::PFN_vkDestroyFramebuffer,
    pub create_semaphore: vk::PFN_vkCreateSemaphore,
    pub destroy_semaphore: vk::PFN_vkDestroySemaphore,
    pub cmd_pipeline_barrier: vk::PFN_vkCmdPipelineBarrier,
    pub cmd_begin_render_pass: vk::PFN_vkCmdBeginRenderPass,
    pub cmd_end_render_pass: vk::PFN_vkCmdEndRenderPass,
    pub cmd_bind_pipeline: vk::PFN_vkCmdBindPipeline,
    pub cmd_set_viewport: vk::PFN_vkCmdSetViewport,
    pub cmd_set_scissor: vk::PFN_vkCmdSetScissor,
    pub cmd_draw: vk::PFN_vkCmdDraw,
    pub create_fence: vk::PFN_vkCreateFence,
    pub destroy_fence: vk::PFN_vkDestroyFence,
    pub wait_for_fences: vk::PFN_vkWaitForFences,
    pub reset_fences: vk::PFN_vkResetFences,
    pub create_image: vk::PFN_vkCreateImage,
    pub destroy_image: vk::PFN_vkDestroyImage,
    pub get_image_memory_requirements: vk::PFN_vkGetImageMemoryRequirements,
    pub bind_image_memory: vk::PFN_vkBindImageMemory,
    pub create_image_view: vk::PFN_vkCreateImageView,
    pub destroy_image_view: vk::PFN_vkDestroyImageView,
    pub create_sampler: vk::PFN_vkCreateSampler,
    pub destroy_sampler: vk::PFN_vkDestroySampler,
    pub create_buffer: vk::PFN_vkCreateBuffer,
    pub destroy_buffer: vk::PFN_vkDestroyBuffer,
    pub get_buffer_memory_requirements: vk::PFN_vkGetBufferMemoryRequirements,
    pub bind_buffer_memory: vk::PFN_vkBindBufferMemory,
    pub allocate_memory: vk::PFN_vkAllocateMemory,
    pub free_memory: vk::PFN_vkFreeMemory,
    pub map_memory: vk::PFN_vkMapMemory,
    pub unmap_memory: vk::PFN_vkUnmapMemory,
    pub create_descriptor_set_layout: vk::PFN_vkCreateDescriptorSetLayout,
    pub destroy_descriptor_set_layout: vk::PFN_vkDestroyDescriptorSetLayout,
    pub create_descriptor_pool: vk::PFN_vkCreateDescriptorPool,
    pub destroy_descriptor_pool: vk::PFN_vkDestroyDescriptorPool,
    pub allocate_descriptor_sets: vk::PFN_vkAllocateDescriptorSets,
    pub update_descriptor_sets: vk::PFN_vkUpdateDescriptorSets,
    pub cmd_bind_descriptor_sets: vk::PFN_vkCmdBindDescriptorSets,
    pub cmd_bind_vertex_buffers: vk::PFN_vkCmdBindVertexBuffers,
    pub cmd_bind_index_buffer: vk::PFN_vkCmdBindIndexBuffer,
    pub cmd_draw_indexed: vk::PFN_vkCmdDrawIndexed,
    pub cmd_push_constants: vk::PFN_vkCmdPushConstants,
    pub cmd_copy_buffer_to_image: vk::PFN_vkCmdCopyBufferToImage,
    pub cmd_copy_image_to_buffer: vk::PFN_vkCmdCopyImageToBuffer,
    pub cmd_blit_image: vk::PFN_vkCmdBlitImage,
    pub get_fence_status: vk::PFN_vkGetFenceStatus,
    pub get_mem_props: vk::PFN_vkGetPhysicalDeviceMemoryProperties,
    pub get_format_props: vk::PFN_vkGetPhysicalDeviceFormatProperties,
    pub get_device_queue: vk::PFN_vkGetDeviceQueue,
    pub free_cmd_buffers: vk::PFN_vkFreeCommandBuffers,
    pub device_wait_idle: vk::PFN_vkDeviceWaitIdle,
}

pub static INSTANCES: Mutex<Option<HashMap<usize, InstanceData>>> = Mutex::new(None);
pub static DEVICES: Mutex<Option<HashMap<usize, DeviceData>>> = Mutex::new(None);
pub static SWAPCHAINS: Mutex<Option<HashMap<u64, SwapchainData>>> = Mutex::new(None);

pub const VERT_SPV: &[u8] = include_bytes!("../shaders/vert.spv");
pub const FRAG_SPV: &[u8] = include_bytes!("../shaders/frag.spv");
pub const UI_VERT_SPV: &[u8] = include_bytes!("../shaders/ui_vert.spv");
pub const UI_FRAG_SPV: &[u8] = include_bytes!("../shaders/ui_frag.spv");
