use std::sync::atomic::{AtomicBool, Ordering};

use ash::vk;
use ash::vk::Handle;

use crate::types::*;

pub(crate) static DEVICE_LOST: AtomicBool = AtomicBool::new(false);

pub unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    present_info: *const vk::PresentInfoKHR,
) -> vk::Result {
    let present_info = &*present_info;

    crate::wayland::dispatch();
    crate::evdev::init();
    if !crate::shim_bridge::has_sdl_hooks() {
        crate::evdev::poll();
    }
    crate::shim_bridge::increment_present_count();

    if DEVICE_LOST.load(Ordering::Relaxed) {
        return chain_present(queue, present_info);
    }

    // Capture hotkeys must be drained while the panel is hidden too.
    crate::shim_bridge::poll_and_forward();
    let overlay_visible = crate::shim_bridge::is_visible();
    let screenshot_requested = ira_overlay::ui::capture::is_screenshot_requested()
        || ira_overlay::ui::capture::is_recording();

    if !overlay_visible && !screenshot_requested {
        return chain_present(queue, present_info);
    }

    if present_info.swapchain_count == 0 {
        return chain_present(queue, present_info);
    }

    let swapchain = present_info.p_swapchains.read();
    let image_index = *present_info.p_image_indices;

    let sc_data = {
        let map = SWAPCHAINS.lock().unwrap();
        map.as_ref()
            .and_then(|m| m.get(&(swapchain.as_raw())))
            .cloned()
    };

    let Some(sc) = sc_data else {
        return chain_present(queue, present_info);
    };

    let idx = image_index as usize;
    if idx >= sc.cmd_buffers.len() {
        return (sc.fns.queue_present)(queue, present_info);
    }

    let cmd = sc.cmd_buffers[idx];
    let sem = sc.semaphores[idx];
    let fence = sc.fences[idx];
    let fb = sc.framebuffers[idx];
    let image = sc.images[idx];

    let fence_result = (sc.fns.wait_for_fences)(sc.device, 1, &fence, vk::TRUE, 200_000_000);
    if fence_result != vk::Result::SUCCESS {
        eprintln!(
            "ira-overlay: fence wait {:?}, presenting without overlay",
            fence_result
        );
        if fence_result == vk::Result::ERROR_DEVICE_LOST {
            DEVICE_LOST.store(true, Ordering::Relaxed);
        }
        return chain_present(queue, present_info);
    }
    ira_overlay::ui::capture::check_and_readback();
    let _ = (sc.fns.reset_fences)(sc.device, 1, &fence);
    let _ = (sc.fns.reset_cmd_buffer)(cmd, vk::CommandBufferResetFlags::empty());

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let _ = (sc.fns.begin_cmd_buffer)(cmd, &begin_info);

    if overlay_visible {
        if let Some(ui) = sc.ui_renderer {
            ui.prepare(sc.extent);
            ui.update_atlas(sc.fns, cmd, fence);
        }
    }

    let subresource = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let captured = ira_overlay::ui::capture::capture(cmd, image, fence, sc.extent);

    let src_layout = if captured {
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL
    } else {
        vk::ImageLayout::PRESENT_SRC_KHR
    };

    if overlay_visible {
        let (src_stage, src_access) = if captured {
            (
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_READ,
            )
        } else {
            (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::AccessFlags::empty(),
            )
        };
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(src_layout)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .src_access_mask(src_access)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            )
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource);
        (sc.fns.cmd_pipeline_barrier)(
            cmd,
            src_stage,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier,
        );

        let clear = vk::ClearValue::default();
        let rp_begin = vk::RenderPassBeginInfo::default()
            .render_pass(sc.render_pass)
            .framebuffer(fb)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.extent,
            })
            .clear_values(std::slice::from_ref(&clear));
        (sc.fns.cmd_begin_render_pass)(cmd, &rp_begin, vk::SubpassContents::INLINE);

        if let Some(ui) = sc.ui_renderer {
            ui.draw(sc.fns, cmd, sc.extent);
        } else {
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: sc.extent.width as f32,
                height: sc.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            (sc.fns.cmd_set_viewport)(cmd, 0, 1, &viewport as *const vk::Viewport);
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.extent,
            };
            (sc.fns.cmd_set_scissor)(cmd, 0, 1, &scissor as *const vk::Rect2D);
            (sc.fns.cmd_bind_pipeline)(cmd, vk::PipelineBindPoint::GRAPHICS, sc.pipeline);
            (sc.fns.cmd_draw)(cmd, 6, 1, 0, 0);
        }

        (sc.fns.cmd_end_render_pass)(cmd);
    } else if captured {
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource);
        (sc.fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier,
        );
    }

    let _ = (sc.fns.end_cmd_buffer)(cmd);

    let wait_sem_count = present_info.wait_semaphore_count;
    let wait_sems = present_info.p_wait_semaphores;
    let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS; 8];
    let wait_stage_slice = &wait_stages[..wait_sem_count as usize];

    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(std::slice::from_raw_parts(
            wait_sems,
            wait_sem_count as usize,
        ))
        .wait_dst_stage_mask(wait_stage_slice)
        .command_buffers(std::slice::from_ref(&cmd))
        .signal_semaphores(std::slice::from_ref(&sem));
    let submit_result =
        (sc.fns.queue_submit)(queue, 1, &submit_info as *const vk::SubmitInfo, fence);
    if submit_result != vk::Result::SUCCESS {
        if submit_result == vk::Result::ERROR_DEVICE_LOST {
            DEVICE_LOST.store(true, Ordering::Relaxed);
        }
        return submit_result;
    }

    let mut new_present_info = *present_info;
    new_present_info.p_wait_semaphores = &sem;
    new_present_info.wait_semaphore_count = 1;

    (sc.fns.queue_present)(queue, &new_present_info)
}

unsafe fn chain_present(queue: vk::Queue, present_info: &vk::PresentInfoKHR) -> vk::Result {
    let fns = {
        let map = DEVICES.lock().unwrap();
        map.as_ref()
            .and_then(|m| m.values().next())
            .and_then(|d| d.fns)
    };
    if let Some(fns) = fns {
        return (fns.queue_present)(queue, present_info);
    }
    vk::Result::ERROR_INITIALIZATION_FAILED
}
