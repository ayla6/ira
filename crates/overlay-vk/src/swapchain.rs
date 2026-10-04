use std::collections::HashMap;
use std::sync::Mutex;

use ash::vk;
use ash::vk::Handle;

use crate::types::*;
use ira_overlay::types::DeviceFns;

/// Layer function table + physical device for a hooked device.
/// None chains as initialization failure.
unsafe fn device_fns(device: vk::Device) -> Option<(DeviceFns, vk::PhysicalDevice)> {
    let map = DEVICES.lock().unwrap();
    let dd = map
        .as_ref()
        .and_then(|m| m.get(&(device.as_raw() as usize)))?;
    Some((dd.fns?, dd.physical_device))
}

/// One semaphore + one signaled fence per swapchain image.
unsafe fn alloc_sync_objects(
    fns: DeviceFns,
    device: vk::Device,
    image_count: u32,
) -> (Vec<vk::Semaphore>, Vec<vk::Fence>) {
    let mut semaphores = Vec::with_capacity(image_count as usize);
    let mut fences = Vec::with_capacity(image_count as usize);
    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
    for _ in 0..image_count {
        let mut sem = vk::Semaphore::null();
        let info = vk::SemaphoreCreateInfo::default();
        log_vk(
            (fns.create_semaphore)(device, &info, std::ptr::null(), &mut sem),
            "create_semaphore",
        );
        semaphores.push(sem);
        let mut fence = vk::Fence::null();
        log_vk(
            (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fence),
            "create_fence",
        );
        fences.push(fence);
    }
    (semaphores, fences)
}

/// Swapchain creation is rare (resolution changes), so driver failures log
/// unconditionally instead of surfacing frames later as null handles.
/// Nulls stay fail-open: the present hook checks handles before use.
fn log_vk(result: vk::Result, what: &str) {
    if result != vk::Result::SUCCESS {
        eprintln!("ira-overlay: swapchain {what} failed: {result:?}");
    }
}

/// Fetches the swapchain images. Fail-open: empty on driver failure,
/// the present hook checks handles before use.
unsafe fn fetch_images(fns: DeviceFns, device: vk::Device, sc: vk::SwapchainKHR) -> Vec<vk::Image> {
    let mut image_count = 0u32;
    log_vk(
        (fns.get_swapchain_images)(device, sc, &mut image_count, std::ptr::null_mut()),
        "get_swapchain_images(count)",
    );
    let mut images = vec![vk::Image::null(); image_count as usize];
    log_vk(
        (fns.get_swapchain_images)(device, sc, &mut image_count, images.as_mut_ptr()),
        "get_swapchain_images",
    );
    images
}

unsafe fn create_cmd_pool(fns: DeviceFns, device: vk::Device) -> vk::CommandPool {
    let pool_info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let mut pool = vk::CommandPool::null();
    log_vk(
        (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut pool),
        "create_command_pool",
    );
    pool
}

unsafe fn alloc_cmd_buffers(
    fns: DeviceFns,
    device: vk::Device,
    pool: vk::CommandPool,
    count: u32,
) -> Vec<vk::CommandBuffer> {
    let mut cmd_buffers = vec![vk::CommandBuffer::null(); count as usize];
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(count);
    log_vk(
        (fns.allocate_cmd_buffers)(device, &alloc_info, cmd_buffers.as_mut_ptr()),
        "allocate_cmd_buffers",
    );
    cmd_buffers
}

pub(crate) unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    create_info: *const vk::SwapchainCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    let Some((fns, physical_device)) = device_fns(device) else {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    };

    let ci = *create_info;
    // Don't force TRANSFER_SRC — some emulators (shadPS4) crash when the
    // swapchain has unexpected usage flags. Screenshot capture will use
    // a blit to a separate image instead.
    // Re-arm the compositor: a fresh swapchain usually means fresh queue
    // state, so a previously wedged queue gets one more chance instead of
    // staying bypassed for the rest of the session.
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

    let images = fetch_images(fns, device, sc);
    let image_count = images.len() as u32;

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

    let cmd_pool = create_cmd_pool(fns, device);
    let cmd_buffers = alloc_cmd_buffers(fns, device, cmd_pool, image_count);

    let (semaphores, fences) = alloc_sync_objects(fns, device, image_count);

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
    crate::xwin::bind_swapchain_window(sc.as_raw(), ci.surface.as_raw());

    result
}

/// Retired swapchain data waiting for the GPU to finish with it: (raw key,
/// full data). Freed once every fence signals (polled, never waited), so
/// teardown can never hang the game thread on a wedged queue. Storing the
/// full `SwapchainData` keeps one struct instead of a parallel shadow that
/// must be kept in sync; sweep simply ignores the live-only fields
/// (images, extent, command buffers — the pool owns those).
static DEAD_SWAPCHAINS: Mutex<Option<Vec<(u64, SwapchainData)>>> = Mutex::new(None);

/// Frees retired swapchains whose fences all signaled (non-blocking).
/// Entries on a dead queue never signal and leak boundedly until process
/// exit instead of hanging teardown.
unsafe fn sweep_dead() {
    let mut dead = DEAD_SWAPCHAINS.lock().unwrap();
    let Some(list) = dead.as_mut() else { return };
    list.retain(|(key, sc)| {
        // SUCCESS = signaled. Anything else keeps the entry, except a lost
        // device where waiting is pointless and destruction is harmless.
        let done = sc.fences.iter().all(|fence| {
            matches!(
                (sc.fns.get_fence_status)(sc.device, *fence),
                vk::Result::SUCCESS | vk::Result::ERROR_DEVICE_LOST
            )
        });
        if !done {
            return true;
        }
        crate::canvas::destroy_canvas(sc.fns, sc.device, *key);
        free_retired(sc);
        false
    });
}

/// Destroys one retired swapchain's layer resources (not the swapchain
/// itself — that belongs to the game and dies immediately).
unsafe fn free_retired(sc: &SwapchainData) {
    let fns = sc.fns;
    let device = sc.device;
    for fb in &sc.framebuffers {
        (fns.destroy_framebuffer)(device, *fb, std::ptr::null());
    }
    for iv in &sc.image_views {
        (fns.destroy_image_view)(device, *iv, std::ptr::null());
    }
    for sem in &sc.semaphores {
        (fns.destroy_semaphore)(device, *sem, std::ptr::null());
    }
    for fence in &sc.fences {
        (fns.destroy_fence)(device, *fence, std::ptr::null());
    }
    (fns.destroy_pipeline)(device, sc.pipeline, std::ptr::null());
    (fns.destroy_pipeline_layout)(device, sc.pipeline_layout, std::ptr::null());
    (fns.destroy_shader_module)(device, sc.shader_vert, std::ptr::null());
    (fns.destroy_shader_module)(device, sc.shader_frag, std::ptr::null());
    (fns.destroy_render_pass)(device, sc.render_pass, std::ptr::null());
    (fns.destroy_command_pool)(device, sc.cmd_pool, std::ptr::null());
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
            .push((swapchain.as_raw(), sc));
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
            log_vk(
                (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut image_view),
                "create_image_view",
            );

            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(std::slice::from_ref(&image_view))
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            let mut fb = vk::Framebuffer::null();
            log_vk(
                (fns.create_framebuffer)(device, &fb_info, std::ptr::null(), &mut fb),
                "create_framebuffer",
            );

            (fb, image_view)
        })
        .unzip();

    (fbs, ivs)
}
