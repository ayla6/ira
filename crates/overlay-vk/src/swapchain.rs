use std::collections::HashMap;
use std::sync::Mutex;

use ash::vk;
use ash::vk::Handle;

use crate::types::*;
use ira_overlay::types::DeviceFns;

pub(crate) unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    create_info: *const vk::SwapchainCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    let (fns, physical_device) = {
        let map = DEVICES.lock().unwrap();
        let Some(dd) = map
            .as_ref()
            .and_then(|m| m.get(&(device.as_raw() as usize)))
        else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let Some(fns) = dd.fns else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        (fns, dd.physical_device)
    };

    let ci = *create_info;
    // Don't force TRANSFER_SRC — some emulators (shadPS4) crash when the
    // swapchain has unexpected usage flags. Screenshot capture will use
    // a blit to a separate image instead.
    crate::present::DEVICE_LOST.store(false, std::sync::atomic::Ordering::Relaxed);
    let result = (fns.create_swapchain)(device, &ci, allocator, swapchain);
    if result != vk::Result::SUCCESS {
        eprintln!("ira-overlay: swapchain creation failed: {:?}", result);
        return result;
    }

    eprintln!(
        "ira-overlay: swapchain created {}x{}",
        ci.image_extent.width, ci.image_extent.height
    );

    // Reset the present counter so the overlay "ready" delay restarts after
    // swapchain recreation. Games like shadPS4 create multiple swapchains
    // during loading (1280x720 → 1920x1080); the overlay should only become
    // toggleable after the final swapchain is stable.
    crate::shim_bridge::reset_present_count();

    let create_info = &ci;
    let sc = *swapchain;

    let mut image_count = 0u32;
    let _ = (fns.get_swapchain_images)(device, sc, &mut image_count, std::ptr::null_mut());
    let mut images = vec![vk::Image::null(); image_count as usize];
    let _ = (fns.get_swapchain_images)(device, sc, &mut image_count, images.as_mut_ptr());

    let format = create_info.image_format;
    let extent = create_info.image_extent;

    let render_pass = create_render_pass(fns, device, format);
    // The canvas pipeline + texture replace the old toolkit renderer. Any
    // failure here is non-fatal: the game presents without overlay.
    let ui_enabled = std::env::var_os("IRA_OVERLAY_DISABLE_UI").is_none();
    let (pipeline, pipeline_layout, shader_vert, shader_frag) = if ui_enabled {
        crate::canvas::create_bundle(fns, device, physical_device, render_pass, sc.as_raw())
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let (framebuffers, image_views) =
        create_framebuffers(fns, device, render_pass, &images, extent, format);

    let cmd_pool = {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let mut pool = vk::CommandPool::null();
        let _ = (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut pool);
        pool
    };

    let mut cmd_buffers = vec![vk::CommandBuffer::null(); image_count as usize];
    {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(image_count);
        let _ = (fns.allocate_cmd_buffers)(device, &alloc_info, cmd_buffers.as_mut_ptr());
    }

    let mut semaphores = Vec::with_capacity(image_count as usize);
    let mut fences = Vec::with_capacity(image_count as usize);
    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
    for _ in 0..image_count {
        let mut sem = vk::Semaphore::null();
        let info = vk::SemaphoreCreateInfo::default();
        let _ = (fns.create_semaphore)(device, &info, std::ptr::null(), &mut sem);
        semaphores.push(sem);
        let mut fence = vk::Fence::null();
        let _ = (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fence);
        fences.push(fence);
    }

    ira_overlay::capture::init(fns, device, physical_device, extent, format);

    SWAPCHAINS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            sc.as_raw(),
            SwapchainData {
                device,
                fns,
                images,
                extent,
                render_pass,
                pipeline,
                pipeline_layout,
                shader_vert,
                shader_frag,
                framebuffers,
                image_views,
                cmd_pool,
                cmd_buffers,
                semaphores,
                fences,
                ui_enabled,
            },
        );

    // Bind the swapchain to its X11 window (if Xlib-backed) for input
    // translation: windowed games report root coordinates that need the
    // window origin subtracted to land in client space.
    crate::canvas::bind_swapchain_window(sc.as_raw(), ci.surface.as_raw());

    result
}

/// Swapchain resources waiting for the GPU to finish with them. Freed once
/// every fence signals (polled, never waited), so teardown can never hang
/// the game thread on a wedged queue.
struct DeadSwapchain {
    device: vk::Device,
    fns: DeviceFns,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    shader_vert: vk::ShaderModule,
    shader_frag: vk::ShaderModule,
    render_pass: vk::RenderPass,
    framebuffers: Vec<vk::Framebuffer>,
    image_views: Vec<vk::ImageView>,
    semaphores: Vec<vk::Semaphore>,
    fences: Vec<vk::Fence>,
    cmd_pool: vk::CommandPool,
    swapchain_key: u64,
}

static DEAD_SWAPCHAINS: Mutex<Option<Vec<DeadSwapchain>>> = Mutex::new(None);

/// Frees retired swapchains whose fences all signaled (non-blocking).
/// Entries on a dead queue never signal and leak boundedly until process
/// exit instead of hanging teardown.
unsafe fn sweep_dead() {
    let mut dead = DEAD_SWAPCHAINS.lock().unwrap();
    let Some(list) = dead.as_mut() else { return };
    list.retain(|entry| {
        // SUCCESS = signaled. Anything else keeps the entry, except a lost
        // device where waiting is pointless and destruction is harmless.
        let done = entry.fences.iter().all(|fence| {
            matches!(
                (entry.fns.get_fence_status)(entry.device, *fence),
                vk::Result::SUCCESS | vk::Result::ERROR_DEVICE_LOST
            )
        });
        if !done {
            return true;
        }
        let fns = entry.fns;
        let device = entry.device;
        crate::canvas::destroy_canvas(fns, device, entry.swapchain_key);
        for fb in &entry.framebuffers {
            (fns.destroy_framebuffer)(device, *fb, std::ptr::null());
        }
        for iv in &entry.image_views {
            (fns.destroy_image_view)(device, *iv, std::ptr::null());
        }
        for sem in &entry.semaphores {
            (fns.destroy_semaphore)(device, *sem, std::ptr::null());
        }
        for fence in &entry.fences {
            (fns.destroy_fence)(device, *fence, std::ptr::null());
        }
        (fns.destroy_pipeline)(device, entry.pipeline, std::ptr::null());
        (fns.destroy_pipeline_layout)(device, entry.pipeline_layout, std::ptr::null());
        (fns.destroy_shader_module)(device, entry.shader_vert, std::ptr::null());
        (fns.destroy_shader_module)(device, entry.shader_frag, std::ptr::null());
        (fns.destroy_render_pass)(device, entry.render_pass, std::ptr::null());
        (fns.destroy_command_pool)(device, entry.cmd_pool, std::ptr::null());
        false
    });
}

pub(crate) unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    allocator: *const vk::AllocationCallbacks,
) {
    let sc_data = SWAPCHAINS
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&(swapchain.as_raw())));
        if let Some(sc) = sc_data {
        let fns = sc.fns;

        ira_overlay::capture::drain_pending();

        // Retire instead of destroying: freeing waits on fences below, and
        // device_wait_idle would hang the game thread on a wedged queue.
        sweep_dead();
        DEAD_SWAPCHAINS
            .lock()
            .unwrap()
            .get_or_insert_with(Vec::new)
            .push(DeadSwapchain {
                device: sc.device,
                fns,
                pipeline: sc.pipeline,
                pipeline_layout: sc.pipeline_layout,
                shader_vert: sc.shader_vert,
                shader_frag: sc.shader_frag,
                render_pass: sc.render_pass,
                framebuffers: sc.framebuffers,
                image_views: sc.image_views,
                semaphores: sc.semaphores,
                fences: sc.fences,
                cmd_pool: sc.cmd_pool,
                swapchain_key: swapchain.as_raw(),
            });
        // The actual swapchain belongs to the game; destroy it immediately.
        (fns.destroy_swapchain)(device, swapchain, allocator);
    } else {
        let fns = {
            let map = DEVICES.lock().unwrap();
            map.as_ref()
                .and_then(|m| m.get(&(device.as_raw() as usize)))
                .and_then(|d| d.fns)
        };
        if let Some(fns) = fns {
            (fns.destroy_swapchain)(device, swapchain, allocator);
        }
    }
}

unsafe fn create_render_pass(
    fns: DeviceFns,
    device: vk::Device,
    format: vk::Format,
) -> vk::RenderPass {
    let attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        // The injected layer renders into the game's existing swapchain image.
        // Clearing here would replace the game with transparent black before
        // the overlay is drawn.
        .load_op(vk::AttachmentLoadOp::LOAD)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

    let color_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_ref));

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::NONE)
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        );

    let rp_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(std::slice::from_ref(&dependency));

    let mut rp = vk::RenderPass::null();
    let _ = (fns.create_render_pass)(device, &rp_info, std::ptr::null(), &mut rp);
    rp
}

unsafe fn create_framebuffers(
    fns: DeviceFns,
    device: vk::Device,
    render_pass: vk::RenderPass,
    images: &[vk::Image],
    extent: vk::Extent2D,
    format: vk::Format,
) -> (Vec<vk::Framebuffer>, Vec<vk::ImageView>) {
    let (fbs, ivs): (Vec<_>, Vec<_>) = images
        .iter()
        .map(|image| {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(*image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let mut image_view = vk::ImageView::null();
            let _ = (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut image_view);

            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(std::slice::from_ref(&image_view))
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            let mut fb = vk::Framebuffer::null();
            let _ = (fns.create_framebuffer)(device, &fb_info, std::ptr::null(), &mut fb);

            (fb, image_view)
        })
        .unzip();

    (fbs, ivs)
}
