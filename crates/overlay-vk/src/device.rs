use std::collections::HashMap;
use std::ffi::CStr;

use ash::vk;
use ash::vk::Handle;

use crate::negotiate::{into_vk_void_fn, transmute_fn};
use crate::types::*;
use ira_overlay::types::DeviceFns;

pub unsafe extern "system" fn get_device_proc_addr(
    device: vk::Device,
    name: *const std::os::raw::c_char,
) -> vk::PFN_vkVoidFunction {
    let name = CStr::from_ptr(name);
    match name.to_bytes() {
        b"vkGetDeviceProcAddr" => into_vk_void_fn!(
            get_device_proc_addr,
            unsafe extern "system" fn(
                vk::Device,
                *const std::os::raw::c_char,
            ) -> vk::PFN_vkVoidFunction
        ),
        b"vkDestroyDevice" => into_vk_void_fn!(
            destroy_device,
            unsafe extern "system" fn(vk::Device, *const vk::AllocationCallbacks)
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
        _ => {
            let map = DEVICES.lock().unwrap();
            if let Some(map) = map.as_ref() {
                if let Some(data) = map.get(&(device.as_raw() as usize)) {
                    if let Some(gdpa) = data.next_gdpa {
                        return gdpa(device, name.as_ptr());
                    }
                }
            }
            None
        }
    }
}

unsafe fn find_layer_link_info(
    create_info: *const vk::DeviceCreateInfo,
) -> *mut LayerDeviceCreateInfo {
    let mut chain = (*create_info).p_next as *mut vk::BaseInStructure;
    while !chain.is_null() {
        if (*chain).s_type == vk::StructureType::LOADER_DEVICE_CREATE_INFO {
            let ci = chain as *mut LayerDeviceCreateInfo;
            if (*ci).function == 0 {
                return ci;
            }
        }
        chain = (*chain).p_next as *mut vk::BaseInStructure;
    }
    std::ptr::null_mut()
}

pub unsafe fn load_device_fns(
    gdpa: vk::PFN_vkGetDeviceProcAddr,
    device: vk::Device,
    get_mem_props: vk::PFN_vkGetPhysicalDeviceMemoryProperties,
    get_format_props: vk::PFN_vkGetPhysicalDeviceFormatProperties,
) -> DeviceFns {
    let load = |name: &CStr| -> vk::PFN_vkVoidFunction { gdpa(device, name.as_ptr()) };
    DeviceFns {
        create_swapchain: transmute_fn(load(c"vkCreateSwapchainKHR")),
        destroy_swapchain: transmute_fn(load(c"vkDestroySwapchainKHR")),
        get_swapchain_images: transmute_fn(load(c"vkGetSwapchainImagesKHR")),
        queue_present: transmute_fn(load(c"vkQueuePresentKHR")),
        queue_submit: transmute_fn(load(c"vkQueueSubmit")),
        create_command_pool: transmute_fn(load(c"vkCreateCommandPool")),
        destroy_command_pool: transmute_fn(load(c"vkDestroyCommandPool")),
        allocate_cmd_buffers: transmute_fn(load(c"vkAllocateCommandBuffers")),
        begin_cmd_buffer: transmute_fn(load(c"vkBeginCommandBuffer")),
        end_cmd_buffer: transmute_fn(load(c"vkEndCommandBuffer")),
        reset_cmd_buffer: transmute_fn(load(c"vkResetCommandBuffer")),
        create_render_pass: transmute_fn(load(c"vkCreateRenderPass")),
        destroy_render_pass: transmute_fn(load(c"vkDestroyRenderPass")),
        create_graphics_pipelines: transmute_fn(load(c"vkCreateGraphicsPipelines")),
        destroy_pipeline: transmute_fn(load(c"vkDestroyPipeline")),
        create_shader_module: transmute_fn(load(c"vkCreateShaderModule")),
        destroy_shader_module: transmute_fn(load(c"vkDestroyShaderModule")),
        create_pipeline_layout: transmute_fn(load(c"vkCreatePipelineLayout")),
        destroy_pipeline_layout: transmute_fn(load(c"vkDestroyPipelineLayout")),
        create_framebuffer: transmute_fn(load(c"vkCreateFramebuffer")),
        destroy_framebuffer: transmute_fn(load(c"vkDestroyFramebuffer")),
        create_semaphore: transmute_fn(load(c"vkCreateSemaphore")),
        destroy_semaphore: transmute_fn(load(c"vkDestroySemaphore")),
        cmd_pipeline_barrier: transmute_fn(load(c"vkCmdPipelineBarrier")),
        cmd_begin_render_pass: transmute_fn(load(c"vkCmdBeginRenderPass")),
        cmd_end_render_pass: transmute_fn(load(c"vkCmdEndRenderPass")),
        cmd_bind_pipeline: transmute_fn(load(c"vkCmdBindPipeline")),
        cmd_set_viewport: transmute_fn(load(c"vkCmdSetViewport")),
        cmd_set_scissor: transmute_fn(load(c"vkCmdSetScissor")),
        cmd_draw: transmute_fn(load(c"vkCmdDraw")),
        create_fence: transmute_fn(load(c"vkCreateFence")),
        destroy_fence: transmute_fn(load(c"vkDestroyFence")),
        wait_for_fences: transmute_fn(load(c"vkWaitForFences")),
        reset_fences: transmute_fn(load(c"vkResetFences")),
        create_image: transmute_fn(load(c"vkCreateImage")),
        destroy_image: transmute_fn(load(c"vkDestroyImage")),
        get_image_memory_requirements: transmute_fn(load(c"vkGetImageMemoryRequirements")),
        bind_image_memory: transmute_fn(load(c"vkBindImageMemory")),
        create_image_view: transmute_fn(load(c"vkCreateImageView")),
        destroy_image_view: transmute_fn(load(c"vkDestroyImageView")),
        create_sampler: transmute_fn(load(c"vkCreateSampler")),
        destroy_sampler: transmute_fn(load(c"vkDestroySampler")),
        create_buffer: transmute_fn(load(c"vkCreateBuffer")),
        destroy_buffer: transmute_fn(load(c"vkDestroyBuffer")),
        get_buffer_memory_requirements: transmute_fn(load(c"vkGetBufferMemoryRequirements")),
        bind_buffer_memory: transmute_fn(load(c"vkBindBufferMemory")),
        allocate_memory: transmute_fn(load(c"vkAllocateMemory")),
        free_memory: transmute_fn(load(c"vkFreeMemory")),
        map_memory: transmute_fn(load(c"vkMapMemory")),
        unmap_memory: transmute_fn(load(c"vkUnmapMemory")),
        create_descriptor_set_layout: transmute_fn(load(c"vkCreateDescriptorSetLayout")),
        destroy_descriptor_set_layout: transmute_fn(load(c"vkDestroyDescriptorSetLayout")),
        create_descriptor_pool: transmute_fn(load(c"vkCreateDescriptorPool")),
        destroy_descriptor_pool: transmute_fn(load(c"vkDestroyDescriptorPool")),
        allocate_descriptor_sets: transmute_fn(load(c"vkAllocateDescriptorSets")),
        update_descriptor_sets: transmute_fn(load(c"vkUpdateDescriptorSets")),
        cmd_bind_descriptor_sets: transmute_fn(load(c"vkCmdBindDescriptorSets")),
        cmd_bind_vertex_buffers: transmute_fn(load(c"vkCmdBindVertexBuffers")),
        cmd_bind_index_buffer: transmute_fn(load(c"vkCmdBindIndexBuffer")),
        cmd_draw_indexed: transmute_fn(load(c"vkCmdDrawIndexed")),
        cmd_push_constants: transmute_fn(load(c"vkCmdPushConstants")),
        get_fence_status: transmute_fn(load(c"vkGetFenceStatus")),
        get_mem_props,
        get_format_props,
        cmd_copy_buffer_to_image: transmute_fn(load(c"vkCmdCopyBufferToImage")),
        cmd_copy_image_to_buffer: transmute_fn(load(c"vkCmdCopyImageToBuffer")),
        cmd_blit_image: transmute_fn(load(c"vkCmdBlitImage")),
        get_device_queue: transmute_fn(load(c"vkGetDeviceQueue")),
        free_cmd_buffers: transmute_fn(load(c"vkFreeCommandBuffers")),
        device_wait_idle: transmute_fn(load(c"vkDeviceWaitIdle")),
    }
}

pub(crate) unsafe extern "system" fn create_device(
    physical_device: vk::PhysicalDevice,
    create_info: *const vk::DeviceCreateInfo,
    allocator: *const vk::AllocationCallbacks,
    device: *mut vk::Device,
) -> vk::Result {
    let create_info = &mut *(create_info as *mut vk::DeviceCreateInfo);

    let layer_ci = find_layer_link_info(create_info);
    if layer_ci.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let link = (*layer_ci).p_layer_info;
    if link.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let link_gdpa = (*link).next_gdpa;
    (*layer_ci).p_layer_info = (*link).p_next;

    let (instance, gipa) = {
        let map = INSTANCES.lock().unwrap();
        match map.as_ref().and_then(|m| m.values().next()) {
            Some(d) => (d.instance, d.loader_gipa),
            None => return vk::Result::ERROR_INITIALIZATION_FAILED,
        }
    };

    let gipa = gipa.expect("gipa should not be null");
    let create_device_fn: vk::PFN_vkCreateDevice =
        transmute_fn(gipa(instance, c"vkCreateDevice".as_ptr()));

    let result = create_device_fn(physical_device, create_info, allocator, device);
    if result != vk::Result::SUCCESS {
        return result;
    }

    let dev = *device;

    let real_gdpa: vk::PFN_vkGetDeviceProcAddr = match link_gdpa {
        Some(g) => g,
        None => transmute_fn(gipa(instance, c"vkGetDeviceProcAddr".as_ptr())),
    };

    let destroy_dev_fn = transmute_fn(real_gdpa(dev, c"vkDestroyDevice".as_ptr()));

    let swapchain_test = real_gdpa(dev, c"vkCreateSwapchainKHR".as_ptr());
    let fns = if swapchain_test.is_some() {
        let get_mem_props = transmute_fn(gipa(
            instance,
            c"vkGetPhysicalDeviceMemoryProperties".as_ptr(),
        ));
        let get_format_props = transmute_fn(gipa(
            instance,
            c"vkGetPhysicalDeviceFormatProperties".as_ptr(),
        ));
        Some(load_device_fns(
            real_gdpa,
            dev,
            get_mem_props,
            get_format_props,
        ))
    } else {
        None
    };

    DEVICES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            dev.as_raw() as usize,
            DeviceData {
                next_gdpa: Some(real_gdpa),
                destroy_device: destroy_dev_fn,
                physical_device,
                fns,
            },
        );

    result
}

pub(crate) unsafe extern "system" fn destroy_device(
    device: vk::Device,
    allocator: *const vk::AllocationCallbacks,
) {
    let destroy_fn = {
        let map = DEVICES.lock().unwrap();
        let Some(dd) = map
            .as_ref()
            .and_then(|m| m.get(&(device.as_raw() as usize)))
        else {
            return;
        };
        if let Some(fns) = dd.fns {
            let _ = (fns.device_wait_idle)(device);
            ira_overlay::ui::capture::destroy(fns, device);
            ira_overlay::ui::capture::free_deferred(fns, device);
        }
        dd.destroy_device
    };

    DEVICES
        .lock()
        .unwrap()
        .as_mut()
        .map(|m| m.remove(&(device.as_raw() as usize)));

    destroy_fn(device, allocator);
}
