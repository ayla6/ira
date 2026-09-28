use std::collections::HashMap;
use std::ffi::CStr;

use ash::vk;
use ash::vk::Handle;

use crate::negotiate::into_vk_void_fn;
use crate::types::*;

pub unsafe extern "system" fn get_instance_proc_addr(
    instance: vk::Instance,
    name: *const std::os::raw::c_char,
) -> vk::PFN_vkVoidFunction {
    let name = CStr::from_ptr(name);
    match name.to_bytes() {
        b"vkGetInstanceProcAddr" => into_vk_void_fn!(
            get_instance_proc_addr,
            unsafe extern "system" fn(
                vk::Instance,
                *const std::os::raw::c_char,
            ) -> vk::PFN_vkVoidFunction
        ),
        b"vkGetDeviceProcAddr" => into_vk_void_fn!(
            super::device::get_device_proc_addr,
            unsafe extern "system" fn(
                vk::Device,
                *const std::os::raw::c_char,
            ) -> vk::PFN_vkVoidFunction
        ),
        b"vkCreateInstance" => into_vk_void_fn!(
            create_instance,
            unsafe extern "system" fn(
                *const vk::InstanceCreateInfo,
                *const vk::AllocationCallbacks,
                *mut vk::Instance,
            ) -> vk::Result
        ),
        b"vkDestroyInstance" => into_vk_void_fn!(
            destroy_instance,
            unsafe extern "system" fn(vk::Instance, *const vk::AllocationCallbacks)
        ),
        b"vkCreateDevice" => into_vk_void_fn!(
            super::device::create_device,
            unsafe extern "system" fn(
                vk::PhysicalDevice,
                *const vk::DeviceCreateInfo,
                *const vk::AllocationCallbacks,
                *mut vk::Device,
            ) -> vk::Result
        ),
        b"vkCreateSwapchainKHR" => into_vk_void_fn!(
            super::swapchain::create_swapchain,
            unsafe extern "system" fn(
                vk::Device,
                *const vk::SwapchainCreateInfoKHR,
                *const vk::AllocationCallbacks,
                *mut vk::SwapchainKHR,
            ) -> vk::Result
        ),
        b"vkDestroySwapchainKHR" => into_vk_void_fn!(
            super::swapchain::destroy_swapchain,
            unsafe extern "system" fn(vk::Device, vk::SwapchainKHR, *const vk::AllocationCallbacks)
        ),
        b"vkQueuePresentKHR" => into_vk_void_fn!(
            super::present::queue_present,
            unsafe extern "system" fn(vk::Queue, *const vk::PresentInfoKHR) -> vk::Result
        ),
        b"vkDestroyDevice" => into_vk_void_fn!(
            super::device::destroy_device,
            unsafe extern "system" fn(vk::Device, *const vk::AllocationCallbacks)
        ),
        b"vkCreateWaylandSurfaceKHR" => {
            if crate::types::debug_log() {
                eprintln!("ira-overlay: vkCreateWaylandSurfaceKHR queried");
            }
            into_vk_void_fn!(
                create_wayland_surface,
                unsafe extern "system" fn(
                    vk::Instance,
                    *const vk::WaylandSurfaceCreateInfoKHR,
                    *const vk::AllocationCallbacks,
                    *mut vk::SurfaceKHR,
                ) -> vk::Result
            )
        }
        b"vkCreateXcbSurfaceKHR" => {
            if crate::types::debug_log() {
                eprintln!("ira-overlay: vkCreateXcbSurfaceKHR queried (XWayland)");
            }
            into_vk_void_fn!(
                create_xcb_surface,
                unsafe extern "system" fn(
                    vk::Instance,
                    *const vk::XcbSurfaceCreateInfoKHR,
                    *const vk::AllocationCallbacks,
                    *mut vk::SurfaceKHR,
                ) -> vk::Result
            )
        }
        b"vkCreateXlibSurfaceKHR" => {
            if crate::types::debug_log() {
                eprintln!("ira-overlay: vkCreateXlibSurfaceKHR queried (XWayland)");
            }
            into_vk_void_fn!(
                create_xlib_surface,
                unsafe extern "system" fn(
                    vk::Instance,
                    *const vk::XlibSurfaceCreateInfoKHR,
                    *const vk::AllocationCallbacks,
                    *mut vk::SurfaceKHR,
                ) -> vk::Result
            )
        }
        _ => loader_gipa(instance).and_then(|gipa| gipa(instance, name.as_ptr())),
    }
}

unsafe fn find_layer_link_info(
    create_info: *const vk::InstanceCreateInfo,
) -> *mut LayerInstanceCreateInfo {
    let mut chain = (*create_info).p_next as *mut vk::BaseInStructure;
    while !chain.is_null() {
        if (*chain).s_type == vk::StructureType::LOADER_INSTANCE_CREATE_INFO {
            let ci = chain as *mut LayerInstanceCreateInfo;
            if (*ci).function == 0 {
                return ci;
            }
        }
        chain = (*chain).p_next as *mut vk::BaseInStructure;
    }
    std::ptr::null_mut()
}

/// The loader's proc-addr for an instance, if we hooked its creation.
/// Shared by the surface forwarders and teardown below.
unsafe fn loader_gipa(instance: vk::Instance) -> PfnGetInstanceProcAddr {
    let map = INSTANCES.lock().unwrap();
    map.as_ref()
        .and_then(|m| m.get(&(instance.as_raw() as usize)))
        .and_then(|d| d.loader_gipa)
}

/// Resolves the driver's real surface creator through the loader chain.
/// None (unknown instance, or driver lacks the platform) chains as
/// initialization failure instead of panicking in the game process.
unsafe fn driver_surface_fn<T>(instance: vk::Instance, fn_name: &CStr) -> Option<T> {
    let gipa = loader_gipa(instance)?;
    let ptr = gipa(instance, fn_name.as_ptr());
    if ptr.is_some() {
        // `ptr` is known-Some: reinterpret its bits as the target table type.
        Some(unsafe { std::mem::transmute_copy::<vk::PFN_vkVoidFunction, T>(&ptr) })
    } else {
        None
    }
}

unsafe extern "system" fn create_instance(
    create_info: *const vk::InstanceCreateInfo,
    allocator: *const vk::AllocationCallbacks,
    instance: *mut vk::Instance,
) -> vk::Result {
    let layer_ci = find_layer_link_info(create_info);
    if layer_ci.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let link = (*layer_ci).p_layer_info;
    if link.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let gipa = (*link).next_gipa.expect("next_gipa should not be null");
    (*layer_ci).p_layer_info = (*link).p_next;

    let create_instance_fn: vk::PFN_vkCreateInstance =
        super::negotiate::transmute_fn(gipa(vk::Instance::null(), c"vkCreateInstance".as_ptr()));

    let result = create_instance_fn(create_info, allocator, instance);
    if result != vk::Result::SUCCESS {
        return result;
    }

    let inst = *instance;
    INSTANCES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            inst.as_raw() as usize,
            InstanceData {
                instance: inst,
                loader_gipa: Some(gipa),
            },
        );

    if !(*create_info).p_application_info.is_null() {
        let app = &*(*create_info).p_application_info;
        if !app.p_application_name.is_null() {
            eprintln!(
                "ira-overlay: instance created for {:?}",
                CStr::from_ptr(app.p_application_name)
            );
        } else {
            eprintln!("ira-overlay: instance created (no app name)");
        }
    } else {
        eprintln!("ira-overlay: instance created (no app info)");
    }

    result
}

unsafe extern "system" fn destroy_instance(
    instance: vk::Instance,
    allocator: *const vk::AllocationCallbacks,
) {
    let destroy_fn = {
        let gipa = loader_gipa(instance);
        if let Some(gipa) = gipa {
            let f: vk::PFN_vkDestroyInstance =
                super::negotiate::transmute_fn(gipa(instance, c"vkDestroyInstance".as_ptr()));
            Some(f)
        } else {
            None
        }
    };

    INSTANCES
        .lock()
        .unwrap()
        .as_mut()
        .map(|m| m.remove(&(instance.as_raw() as usize)));

    if let Some(f) = destroy_fn {
        f(instance, allocator);
    }
}

unsafe extern "system" fn create_wayland_surface(
    instance: vk::Instance,
    create_info: *const vk::WaylandSurfaceCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    surface: *mut vk::SurfaceKHR,
) -> vk::Result {
    eprintln!("ira-overlay: Wayland surface created");
    let Some(func) = driver_surface_fn::<vk::PFN_vkCreateWaylandSurfaceKHR>(
        instance,
        c"vkCreateWaylandSurfaceKHR",
    ) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };

    let result = func(instance, create_info, allocator, surface);
    if result == vk::Result::SUCCESS {
        crate::wayland::init((*create_info).display);
    }
    result
}

unsafe extern "system" fn create_xcb_surface(
    instance: vk::Instance,
    create_info: *const vk::XcbSurfaceCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    surface: *mut vk::SurfaceKHR,
) -> vk::Result {
    eprintln!(
        "ira-overlay: Xcb surface created (XWayland — events via X11 shim; \
        no Display* in XCB create-info, so no origin translation)"
    );
    let Some(func) =
        driver_surface_fn::<vk::PFN_vkCreateXcbSurfaceKHR>(instance, c"vkCreateXcbSurfaceKHR")
    else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    func(instance, create_info, allocator, surface)
}

unsafe extern "system" fn create_xlib_surface(
    instance: vk::Instance,
    create_info: *const vk::XlibSurfaceCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    surface: *mut vk::SurfaceKHR,
) -> vk::Result {
    eprintln!("ira-overlay: Xlib surface created (XWayland — keyboard/mouse via X11 shim)");
    let Some(func) =
        driver_surface_fn::<vk::PFN_vkCreateXlibSurfaceKHR>(instance, c"vkCreateXlibSurfaceKHR")
    else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };
    let result = func(instance, create_info, allocator, surface);
    // Remember the X window behind this surface for input translation:
    // X11 reports pointer motion in root coordinates, but the panel rect
    // lives in client space. Windowed games need the origin subtracted.
    if result == vk::Result::SUCCESS {
        let info = *create_info;
        crate::xwin::record_xlib_surface((*surface).as_raw(), info.dpy as usize, info.window);
    }
    result
}
