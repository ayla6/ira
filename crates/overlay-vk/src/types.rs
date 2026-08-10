use std::collections::HashMap;
use std::sync::Mutex;

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay::ui::UiRenderer;

pub type PfnGetInstanceProcAddr = Option<
    unsafe extern "system" fn(vk::Instance, *const std::os::raw::c_char) -> vk::PFN_vkVoidFunction,
>;
pub type PfnGetDeviceProcAddr = Option<
    unsafe extern "system" fn(vk::Device, *const std::os::raw::c_char) -> vk::PFN_vkVoidFunction,
>;
pub type PfnGetPhysicalDeviceProcAddr = Option<
    unsafe extern "system" fn(
        vk::PhysicalDevice,
        *const std::os::raw::c_char,
    ) -> vk::PFN_vkVoidFunction,
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
    pub ui_renderer: Option<UiRenderer>,
}

pub static INSTANCES: Mutex<Option<HashMap<usize, InstanceData>>> = Mutex::new(None);
pub static DEVICES: Mutex<Option<HashMap<usize, DeviceData>>> = Mutex::new(None);
pub static SWAPCHAINS: Mutex<Option<HashMap<u64, SwapchainData>>> = Mutex::new(None);

pub const VERT_SPV: &[u8] = include_bytes!("../shaders/vert.spv");
pub const FRAG_SPV: &[u8] = include_bytes!("../shaders/frag.spv");
