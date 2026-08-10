//! Vulkan instance, surface, swapchain, and render loop for the standalone overlay.
//!
//! Uses an XCB surface under gamescope. The Gamescope WSI layer intercepts
//! `vkCreateXcbSurfaceKHR` and creates a Wayland surface on gamescope's socket
//! that presents the overlay's frames directly to gamescope (bypassing
//! XWayland) with pre-multiplied alpha. The window is marked as
//! `GAMESCOPE_EXTERNAL_OVERLAY`, so gamescope composites it on top of the game
//! as a separate plane.

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay::ui::UiRenderer;

struct SurfaceFns {
    create_xcb_surface: vk::PFN_vkVoidFunction,
    destroy_surface: vk::PFN_vkVoidFunction,
    get_physical_device_surface_capabilities: vk::PFN_vkVoidFunction,
    get_physical_device_surface_formats: vk::PFN_vkVoidFunction,
}

pub struct VulkanState {
    pub fns: DeviceFns,
    pub device: vk::Device,
    pub physical_device: vk::PhysicalDevice,
    pub cmd_pool: vk::CommandPool,
    pub cmd: vk::CommandBuffer,
    pub fence: vk::Fence,
    pub render_pass: vk::RenderPass,
    pub extent: vk::Extent2D,
    composite_alpha: vk::CompositeAlphaFlagsKHR,

    _entry: ash::Entry,
    instance: ash::Instance,
    surface: vk::SurfaceKHR,
    queue: vk::Queue,
    ash_device: ash::Device,
    swapchain_ext: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    framebuffers: Vec<vk::Framebuffer>,
    image_views: Vec<vk::ImageView>,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
}

impl VulkanState {
    pub fn new(xcb_conn: *mut c_void, xcb_window: u32) -> Result<Self, String> {
        let entry =
            unsafe { ash::Entry::load() }.map_err(|e| format!("failed to load Vulkan: {e}"))?;

        let ext_names: [*const c_char; 2] =
            [c"VK_KHR_surface".as_ptr(), c"VK_KHR_xcb_surface".as_ptr()];
        let app_info = vk::ApplicationInfo {
            p_application_name: c"ira-overlay-standalone".as_ptr(),
            api_version: vk::make_api_version(0, 1, 2, 0),
            ..Default::default()
        };
        let instance = unsafe {
            entry
                .create_instance(
                    &vk::InstanceCreateInfo {
                        p_application_info: &app_info,
                        enabled_extension_count: ext_names.len() as u32,
                        pp_enabled_extension_names: ext_names.as_ptr(),
                        ..Default::default()
                    },
                    None,
                )
                .map_err(|e| format!("vkCreateInstance: {e}"))?
        };

        let gipa =
            |name: &CStr| unsafe { entry.get_instance_proc_addr(instance.handle(), name.as_ptr()) };
        let surface_fns = SurfaceFns {
            create_xcb_surface: gipa(c"vkCreateXcbSurfaceKHR"),
            destroy_surface: gipa(c"vkDestroySurfaceKHR"),
            get_physical_device_surface_capabilities: gipa(
                c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
            ),
            get_physical_device_surface_formats: gipa(c"vkGetPhysicalDeviceSurfaceFormatsKHR"),
        };
        if surface_fns.create_xcb_surface.is_none() {
            return Err("vkCreateXcbSurfaceKHR not available".to_string());
        }

        let xcb_info = vk::XcbSurfaceCreateInfoKHR {
            connection: xcb_conn as *mut _,
            window: xcb_window,
            ..Default::default()
        };
        let mut surface = vk::SurfaceKHR::null();
        let result = unsafe {
            let f: unsafe extern "system" fn(
                vk::Instance,
                *const vk::XcbSurfaceCreateInfoKHR,
                *const vk::AllocationCallbacks,
                *mut vk::SurfaceKHR,
            ) -> vk::Result = std::mem::transmute(surface_fns.create_xcb_surface);
            f(instance.handle(), &xcb_info, std::ptr::null(), &mut surface)
        };
        if result != vk::Result::SUCCESS {
            return Err(format!("vkCreateXcbSurfaceKHR: {result:?}"));
        }

        let phys_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(|e| format!("enumerate_physical_devices: {e}"))?
        };
        let (physical_device, queue_family) = phys_devices
            .iter()
            .find_map(|&pd| {
                let props = unsafe { instance.get_physical_device_queue_family_properties(pd) };
                for (i, q) in props.iter().enumerate() {
                    if q.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                        return Some((pd, i as u32));
                    }
                }
                None
            })
            .ok_or("no queue family with graphics")?;
        let props = unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy();
        eprintln!("ira-overlay-standalone: using physical device '{device_name}'");

        let queue_priorities = [1.0f32];
        let device_ext_names: [*const c_char; 1] = [c"VK_KHR_swapchain".as_ptr()];
        let ash_device = unsafe {
            instance
                .create_device(
                    physical_device,
                    &vk::DeviceCreateInfo {
                        p_queue_create_infos: &vk::DeviceQueueCreateInfo {
                            queue_family_index: queue_family,
                            queue_count: 1,
                            p_queue_priorities: queue_priorities.as_ptr(),
                            ..Default::default()
                        },
                        queue_create_info_count: 1,
                        enabled_extension_count: device_ext_names.len() as u32,
                        pp_enabled_extension_names: device_ext_names.as_ptr(),
                        ..Default::default()
                    },
                    None,
                )
                .map_err(|e| format!("vkCreateDevice: {e}"))?
        };
        let device = ash_device.handle();
        let queue = unsafe { ash_device.get_device_queue(queue_family, 0) };

        let swapchain_ext = ash::khr::swapchain::Device::new(&instance, &ash_device);
        let fns = build_device_fns(&entry, &instance, device);

        let cmd_pool = unsafe {
            ash_device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo {
                        flags: vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
                        queue_family_index: queue_family,
                        ..Default::default()
                    },
                    None,
                )
                .map_err(|e| format!("create_command_pool: {e}"))?
        };
        let cmd = unsafe {
            ash_device
                .allocate_command_buffers(&vk::CommandBufferAllocateInfo {
                    command_pool: cmd_pool,
                    level: vk::CommandBufferLevel::PRIMARY,
                    command_buffer_count: 1,
                    ..Default::default()
                })
                .map_err(|e| format!("allocate_command_buffers: {e}"))?
        }[0];

        let fence = unsafe {
            ash_device
                .create_fence(
                    &vk::FenceCreateInfo {
                        flags: vk::FenceCreateFlags::SIGNALED,
                        ..Default::default()
                    },
                    None,
                )
                .map_err(|e| format!("create_fence: {e}"))?
        };
        let image_available = unsafe {
            ash_device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|e| format!("sem: {e}"))?
        };
        let render_finished = unsafe {
            ash_device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|e| format!("sem: {e}"))?
        };

        let mut caps = vk::SurfaceCapabilitiesKHR::default();
        let f: unsafe extern "system" fn(
            vk::PhysicalDevice,
            vk::SurfaceKHR,
            *mut vk::SurfaceCapabilitiesKHR,
        ) -> vk::Result =
            unsafe { std::mem::transmute(surface_fns.get_physical_device_surface_capabilities) };
        let mut caps_result = unsafe { f(physical_device, surface, &mut caps) };
        for attempt in 1..20 {
            if caps_result == vk::Result::SUCCESS {
                break;
            }
            if attempt >= 5 {
                eprintln!("ira-overlay-standalone: get_surface_caps attempt {attempt} failed: {caps_result:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            caps_result = unsafe { f(physical_device, surface, &mut caps) };
        }
        if caps_result != vk::Result::SUCCESS {
            return Err(format!("get_surface_caps: {caps_result:?}"));
        }
        eprintln!(
            "ira-overlay-standalone: surface caps ok (extent {}x{}, min_images {}, composite {:?})",
            caps.current_extent.width,
            caps.current_extent.height,
            caps.min_image_count,
            caps.supported_composite_alpha
        );

        let mut format_count = 0u32;
        let f: unsafe extern "system" fn(
            vk::PhysicalDevice,
            vk::SurfaceKHR,
            *mut u32,
            *mut vk::SurfaceFormatKHR,
        ) -> vk::Result =
            unsafe { std::mem::transmute(surface_fns.get_physical_device_surface_formats) };
        unsafe {
            f(
                physical_device,
                surface,
                &mut format_count,
                std::ptr::null_mut(),
            )
            .result()
            .ok()
        };
        let mut formats = Vec::with_capacity(format_count as usize);
        unsafe {
            f(
                physical_device,
                surface,
                &mut format_count,
                formats.as_mut_ptr(),
            )
            .result()
            .ok()
        };
        unsafe { formats.set_len(format_count as usize) };
        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .map(|f| f.format)
            .unwrap_or(vk::Format::B8G8R8A8_UNORM);

        let extent = if caps.current_extent.width == 0 || caps.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: 1920,
                height: 1080,
            }
        } else {
            caps.current_extent
        };

        let composite_alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
        {
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        } else {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        };
        let render_pass = create_render_pass(&ash_device, format);

        let swapchain = unsafe {
            swapchain_ext.create_swapchain(
                &vk::SwapchainCreateInfoKHR {
                    surface,
                    min_image_count: caps.min_image_count.max(2),
                    image_format: format,
                    image_color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
                    image_extent: extent,
                    image_array_layers: 1,
                    image_usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    pre_transform: caps.current_transform,
                    composite_alpha,
                    present_mode: vk::PresentModeKHR::MAILBOX,
                    clipped: vk::TRUE,
                    ..Default::default()
                },
                None,
            )
        }
        .map_err(|e| format!("create_swapchain: {e}"))?;

        let swapchain_images = unsafe { swapchain_ext.get_swapchain_images(swapchain) }
            .map_err(|e| format!("get_swapchain_images: {e}"))?;

        let (image_views, framebuffers) = create_views_and_framebuffers(
            &ash_device,
            &swapchain_images,
            render_pass,
            extent,
            format,
        );

        SURFACE_DESTROY_FN.store(
            unsafe { std::mem::transmute_copy(&surface_fns.destroy_surface) },
            std::sync::atomic::Ordering::Relaxed,
        );

        Ok(VulkanState {
            fns,
            device,
            physical_device,
            cmd_pool,
            cmd,
            fence,
            render_pass,
            extent,
            composite_alpha,
            _entry: entry,
            instance,
            surface,
            queue,
            ash_device,
            swapchain_ext,
            swapchain,
            swapchain_images,
            framebuffers,
            image_views,
            image_available,
            render_finished,
        })
    }

    pub unsafe fn render_frame(&self, ui: &UiRenderer) -> bool {
        unsafe {
            self.ash_device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .ok();
            self.ash_device.reset_fences(&[self.fence]).ok();
        }

        let mut image_index = 0u32;
        let acquire_result = unsafe {
            self.swapchain_ext.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            )
        };
        match acquire_result {
            Ok((idx, _)) => image_index = idx,
            Err(vk::Result::SUBOPTIMAL_KHR) => {}
            Err(_) => return false,
        }

        unsafe {
            self.ash_device
                .reset_command_buffer(self.cmd, vk::CommandBufferResetFlags::empty())
                .ok();
            self.ash_device
                .begin_command_buffer(self.cmd, &vk::CommandBufferBeginInfo::default())
                .ok();

            // Update font atlas (must be after begin_command_buffer)
            ui.update_atlas(self.fns, self.cmd, self.fence);

            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 0.0],
                },
            }];
            self.ash_device.cmd_begin_render_pass(
                self.cmd,
                &vk::RenderPassBeginInfo {
                    render_pass: self.render_pass,
                    framebuffer: self.framebuffers[image_index as usize],
                    render_area: vk::Rect2D {
                        offset: Default::default(),
                        extent: self.extent,
                    },
                    p_clear_values: clear.as_ptr(),
                    clear_value_count: 1,
                    ..Default::default()
                },
                vk::SubpassContents::INLINE,
            );

            ui.draw(self.fns, self.cmd, self.extent);

            self.ash_device.cmd_end_render_pass(self.cmd);
            self.ash_device.end_command_buffer(self.cmd).ok();
        }

        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let submit_info = vk::SubmitInfo {
            wait_semaphore_count: 1,
            p_wait_semaphores: &self.image_available,
            p_wait_dst_stage_mask: wait_stages.as_ptr(),
            command_buffer_count: 1,
            p_command_buffers: &self.cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: &self.render_finished,
            ..Default::default()
        };
        unsafe {
            self.ash_device
                .queue_submit(self.queue, &[submit_info], self.fence)
                .ok();
        }

        let present_info = vk::PresentInfoKHR {
            wait_semaphore_count: 1,
            p_wait_semaphores: &self.render_finished,
            swapchain_count: 1,
            p_swapchains: &self.swapchain,
            p_image_indices: &image_index,
            ..Default::default()
        };
        match unsafe { self.swapchain_ext.queue_present(self.queue, &present_info) } {
            Ok(_) => true,
            Err(vk::Result::SUBOPTIMAL_KHR) => true,
            Err(_) => false,
        }
    }

    pub unsafe fn recreate_swapchain(&mut self, width: u32, height: u32) {
        unsafe { self.ash_device.device_wait_idle().ok() };
        for fb in &self.framebuffers {
            unsafe { self.ash_device.destroy_framebuffer(*fb, None) }
        }
        for view in &self.image_views {
            unsafe { self.ash_device.destroy_image_view(*view, None) }
        }
        self.framebuffers.clear();
        self.image_views.clear();

        unsafe { self.swapchain_ext.destroy_swapchain(self.swapchain, None) };

        let extent = vk::Extent2D { width, height };
        let format = vk::Format::B8G8R8A8_UNORM;
        let swapchain = match unsafe {
            self.swapchain_ext.create_swapchain(
                &vk::SwapchainCreateInfoKHR {
                    surface: self.surface,
                    min_image_count: 2,
                    image_format: format,
                    image_color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
                    image_extent: extent,
                    image_array_layers: 1,
                    image_usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    pre_transform: vk::SurfaceTransformFlagsKHR::IDENTITY,
                    composite_alpha: self.composite_alpha,
                    present_mode: vk::PresentModeKHR::MAILBOX,
                    clipped: vk::TRUE,
                    ..Default::default()
                },
                None,
            )
        } {
            Ok(sc) => sc,
            Err(e) => {
                eprintln!("ira-overlay-standalone: recreate swapchain: {e:?}");
                return;
            }
        };
        self.swapchain = swapchain;

        let images =
            unsafe { self.swapchain_ext.get_swapchain_images(swapchain) }.unwrap_or_default();
        self.swapchain_images = images;

        let (views, fbs) = create_views_and_framebuffers(
            &self.ash_device,
            &self.swapchain_images,
            self.render_pass,
            extent,
            format,
        );
        self.image_views = views;
        self.framebuffers = fbs;
        self.extent = extent;
    }
}

static SURFACE_DESTROY_FN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Drop for VulkanState {
    fn drop(&mut self) {
        unsafe {
            self.ash_device.device_wait_idle().ok();
            for fb in &self.framebuffers {
                self.ash_device.destroy_framebuffer(*fb, None)
            }
            for view in &self.image_views {
                self.ash_device.destroy_image_view(*view, None)
            }
            let destroy_sc: unsafe extern "system" fn(
                vk::Device,
                vk::SwapchainKHR,
                *const vk::AllocationCallbacks,
            ) = std::mem::transmute(self.fns.destroy_swapchain);
            destroy_sc(self.device, self.swapchain, std::ptr::null());
            self.ash_device.destroy_render_pass(self.render_pass, None);
            self.ash_device
                .destroy_semaphore(self.image_available, None);
            self.ash_device
                .destroy_semaphore(self.render_finished, None);
            self.ash_device.destroy_fence(self.fence, None);
            self.ash_device.destroy_command_pool(self.cmd_pool, None);
            self.ash_device.destroy_device(None);
            let destroy_surface: unsafe extern "system" fn(
                vk::Instance,
                vk::SurfaceKHR,
                *const vk::AllocationCallbacks,
            ) = std::mem::transmute(SURFACE_DESTROY_FN.load(std::sync::atomic::Ordering::Relaxed));
            destroy_surface(self.instance.handle(), self.surface, std::ptr::null());
            self.instance.destroy_instance(None);
        }
    }
}

fn create_render_pass(device: &ash::Device, format: vk::Format) -> vk::RenderPass {
    let attachments = [vk::AttachmentDescription {
        format,
        samples: vk::SampleCountFlags::TYPE_1,
        load_op: vk::AttachmentLoadOp::CLEAR,
        store_op: vk::AttachmentStoreOp::STORE,
        initial_layout: vk::ImageLayout::UNDEFINED,
        final_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        ..Default::default()
    }];
    let color_refs = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let subpass = vk::SubpassDescription {
        pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
        p_color_attachments: color_refs.as_ptr(),
        color_attachment_count: 1,
        ..Default::default()
    };
    unsafe {
        device
            .create_render_pass(
                &vk::RenderPassCreateInfo {
                    p_attachments: attachments.as_ptr(),
                    attachment_count: 1,
                    p_subpasses: &subpass,
                    subpass_count: 1,
                    ..Default::default()
                },
                None,
            )
            .unwrap_or_else(|e| panic!("create_render_pass: {e}"))
    }
}

fn create_views_and_framebuffers(
    device: &ash::Device,
    images: &[vk::Image],
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    format: vk::Format,
) -> (Vec<vk::ImageView>, Vec<vk::Framebuffer>) {
    let mut views = Vec::with_capacity(images.len());
    let mut fbs = Vec::with_capacity(images.len());
    for &img in images {
        let view = unsafe {
            device
                .create_image_view(
                    &vk::ImageViewCreateInfo {
                        image: img,
                        view_type: vk::ImageViewType::TYPE_2D,
                        format,
                        subresource_range: vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            level_count: 1,
                            layer_count: 1,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    None,
                )
                .unwrap_or_else(|e| panic!("create_image_view: {e}"))
        };
        let fb = unsafe {
            device
                .create_framebuffer(
                    &vk::FramebufferCreateInfo {
                        render_pass,
                        p_attachments: &view,
                        attachment_count: 1,
                        width: extent.width,
                        height: extent.height,
                        layers: 1,
                        ..Default::default()
                    },
                    None,
                )
                .unwrap_or_else(|e| panic!("create_framebuffer: {e}"))
        };
        views.push(view);
        fbs.push(fb);
    }
    (views, fbs)
}

fn build_device_fns(entry: &ash::Entry, instance: &ash::Instance, device: vk::Device) -> DeviceFns {
    let load = |name: &CStr| -> vk::PFN_vkVoidFunction {
        unsafe { instance.get_device_proc_addr(device, name.as_ptr()) }
    };
    let get_mem_props = transmute_fn(unsafe {
        entry.get_instance_proc_addr(
            instance.handle(),
            c"vkGetPhysicalDeviceMemoryProperties".as_ptr(),
        )
    });
    let get_format_props = transmute_fn(unsafe {
        entry.get_instance_proc_addr(
            instance.handle(),
            c"vkGetPhysicalDeviceFormatProperties".as_ptr(),
        )
    });

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
        cmd_copy_buffer_to_image: transmute_fn(load(c"vkCmdCopyBufferToImage")),
        cmd_copy_image_to_buffer: transmute_fn(load(c"vkCmdCopyImageToBuffer")),
        cmd_blit_image: transmute_fn(load(c"vkCmdBlitImage")),
        get_fence_status: transmute_fn(load(c"vkGetFenceStatus")),
        get_mem_props,
        get_format_props,
        get_device_queue: transmute_fn(load(c"vkGetDeviceQueue")),
        free_cmd_buffers: transmute_fn(load(c"vkFreeCommandBuffers")),
        device_wait_idle: transmute_fn(load(c"vkDeviceWaitIdle")),
    }
}

fn transmute_fn<T>(p: vk::PFN_vkVoidFunction) -> T {
    unsafe { std::mem::transmute_copy(&p) }
}
