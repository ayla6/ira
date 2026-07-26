use ash::vk;

use crate::types::*;

#[no_mangle]
pub unsafe extern "system" fn vkNegotiateLoaderLayerInterfaceVersion(
    info: *mut NegotiateLoaderLayerInterface,
) -> vk::Result {
    eprintln!("ira-overlay: vkNegotiateLoaderLayerInterfaceVersion called (loader v{})", (*info).loader_layer_interface_version);
    if (*info).loader_layer_interface_version < 2 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    (*info).loader_layer_interface_version = 2;
    (*info).pfn_get_instance_proc_addr = Some(super::instance::get_instance_proc_addr);
    (*info).pfn_get_device_proc_addr = Some(super::device::get_device_proc_addr);
    (*info).pfn_get_physical_device_proc_addr = None;
    vk::Result::SUCCESS
}

pub unsafe fn transmute_fn<T>(v: vk::PFN_vkVoidFunction) -> T {
    assert!(v.is_some(), "failed to load Vulkan function");
    std::mem::transmute_copy(&v)
}

macro_rules! into_vk_void_fn {
    ($f:expr, $ty:ty) => {{
        let f: $ty = $f;
        Some(unsafe { std::mem::transmute::<$ty, unsafe extern "system" fn()>(f) })
    }};
}
pub(crate) use into_vk_void_fn;
