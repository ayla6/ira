use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ash::vk;
use ash::vk::Handle;

use crate::types::*;

pub(crate) static DEVICE_LOST: AtomicBool = AtomicBool::new(false);
/// Consecutive fence timeouts. A wedged queue times out forever; after a
/// short run the layer stops touching the GPU at all (same as DEVICE_LOST).
static FENCE_TIMEOUTS: AtomicU32 = AtomicU32::new(0);
const MAX_FENCE_TIMEOUTS: u32 = 5;

/// Fail closed: pipeline/render-pass creation ignores driver errors, so a
/// null handle must skip compositing rather than reach the driver.
fn ui_usable(render_pass: vk::RenderPass, pipeline: vk::Pipeline) -> bool {
    !render_pass.is_null() && !pipeline.is_null()
}

/// Permanently disables compositing for this session: every later present
/// chains straight through. A failed composite path must degrade to "no
/// overlay", never to a wedged game.
fn engage_bypass(reason: &str) {
    eprintln!("ira-overlay: disabling compositor ({reason}), game continues without overlay");
    DEVICE_LOST.store(true, Ordering::Relaxed);
}

/// Publishes the swapchain extent for host window sizing on every present
/// the layer sees — visible or not — so the size is known before the
/// first show instead of morphing after first input.
fn publish_extent_for_present(present_info: &vk::PresentInfoKHR) {
    if present_info.swapchain_count == 0 {
        return;
    }
    let swapchain = unsafe { present_info.p_swapchains.read().as_raw() };
    let extent = {
        let map = SWAPCHAINS.lock().unwrap();
        map.as_ref()
            .and_then(|m| m.get(&swapchain))
            .map(|sc| sc.extent)
    };
    if let Some(extent) = extent {
        crate::shim_bridge::with_canvas(|shm| crate::canvas::publish_extent(shm, extent));
    }
}

/// First swapchain of this present (0 when there is none): anchors
/// per-swapchain input translation before the swapchain map is consulted.
fn present_swapchain(present_info: &vk::PresentInfoKHR) -> u64 {
    if present_info.swapchain_count == 0 {
        return 0;
    }
    unsafe { present_info.p_swapchains.read().as_raw() }
}

/// Watchdog: while the overlay is visible, host frames must flow. If the
/// write sequence stalls for 5s, input is dead (motions drop off-panel)
/// and exactly one loud hint explains why instead of silent nothing.
/// Resets on any progress; silent while hidden (no frames expected).
fn watch_host_frames(overlay_visible: bool) {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    static LAST_SEQ: AtomicU32 = AtomicU32::new(u32::MAX);
    static LAST_CHANGE_MS: AtomicU32 = AtomicU32::new(0);
    static WARNED: AtomicBool = AtomicBool::new(false);

    let now = crate::canvas::now_ms();
    if !overlay_visible {
        LAST_CHANGE_MS.store(now, Ordering::Relaxed);
        WARNED.store(false, Ordering::Relaxed);
        return;
    }
    if let Some(seq) = crate::shim_bridge::with_canvas(|shm| shm.frame_meta().2) {
        if seq != LAST_SEQ.load(Ordering::Relaxed) {
            LAST_SEQ.store(seq, Ordering::Relaxed);
            LAST_CHANGE_MS.store(now, Ordering::Relaxed);
            WARNED.store(false, Ordering::Relaxed);
            return;
        }
    }
    if now.wrapping_sub(LAST_CHANGE_MS.load(Ordering::Relaxed)) > 5000
        && !WARNED.swap(true, Ordering::Relaxed)
    {
        eprintln!(
            "ira-overlay: HOST SILENT 5s while visible (no new frames): host not spawned, \
             crashed, or window never mapped — input is dead until frames flow"
        );
    }
}

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

    // Capture hotkeys must be drained while the panel is hidden too — and
    // host button actions arrive even when the layer draws nothing
    // (gamescope capture mode runs with UI disabled).
    crate::shim_bridge::poll_and_forward(present_swapchain(present_info));
    crate::shim_bridge::drain_host_actions();
    crate::shim_bridge::sync_visible_to_shm();
    let overlay_visible = crate::shim_bridge::is_visible();
    let capture_active = ira_overlay::capture::is_capture_active();

    watch_host_frames(overlay_visible);

    if overlay_visible {
        crate::shim_bridge::enforce_cursor();
    }

    if !overlay_visible && !capture_active {
        // Still publish the extent: the host sizes its window from it,
        // and the size must be known before the first show, not after
        // first input.
        publish_extent_for_present(present_info);
        return chain_present(queue, present_info);
    }

    if present_info.swapchain_count == 0 {
        return chain_present(queue, present_info);
    }

    // Read up front: input polling translates coordinates per swapchain.
    let swapchain = present_info.p_swapchains.read();
    let image_index = *present_info.p_image_indices;

    let sc_data = {
        let map = SWAPCHAINS.lock().unwrap();
        map.as_ref()
            .and_then(|m| m.get(&(swapchain.as_raw())))
            .cloned()
    };

    // Same publish on the visible path (cheap idempotent stores).
    publish_extent_for_present(present_info);

    let Some(sc) = sc_data else {
        return chain_present(queue, present_info);
    };

    let render_ui = overlay_visible && sc.ui_enabled;
    if !render_ui && !capture_active {
        return chain_present(queue, present_info);
    }

    let idx = image_index as usize;
    if idx >= sc.cmd_buffers.len() {
        return (sc.fns.queue_present)(queue, present_info);
    }

    // Decided before touching fences: chaining after a fence reset without
    // a submit would orphan the fence and stall every later frame.
    if present_info.wait_semaphore_count as usize > 8 {
        return chain_present(queue, present_info);
    }

    let cmd = sc.cmd_buffers[idx];
    let sem = sc.semaphores[idx];
    let fence = sc.fences[idx];
    let fb = sc.framebuffers[idx];
    let image = sc.images[idx];

    let fence_result = (sc.fns.wait_for_fences)(sc.device, 1, &fence, vk::TRUE, 200_000_000);
    if fence_result != vk::Result::SUCCESS {
        if fence_result == vk::Result::TIMEOUT {
            // Occasional timeouts happen under load; a run of them means
            // the queue is wedged — stop submitting instead of adding a
            // 200ms stall to every frame forever.
            if FENCE_TIMEOUTS.fetch_add(1, Ordering::Relaxed) + 1 >= MAX_FENCE_TIMEOUTS {
                engage_bypass("fence timeouts");
            }
        } else {
            eprintln!(
                "ira-overlay: fence wait {:?}, presenting without overlay",
                fence_result
            );
            if fence_result == vk::Result::ERROR_DEVICE_LOST {
                engage_bypass("device lost");
            }
        }
        return chain_present(queue, present_info);
    }
    FENCE_TIMEOUTS.store(0, Ordering::Relaxed);
    ira_overlay::capture::check_and_readback();
    let _ = (sc.fns.reset_fences)(sc.device, 1, &fence);
    let _ = (sc.fns.reset_cmd_buffer)(cmd, vk::CommandBufferResetFlags::empty());

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let _ = (sc.fns.begin_cmd_buffer)(cmd, &begin_info);

    // Canvas upload: runs before the swapchain barriers: layout transitions
    // on the canvas image must stay outside the render pass.
    let panel = if render_ui {
        let position = crate::shim_bridge::overlay_position();
        crate::shim_bridge::with_canvas(|shm| unsafe {
            crate::canvas::prepare_upload(
                sc.fns,
                cmd,
                swapchain.as_raw(),
                sc.extent,
                position,
                shm,
            )
        })
        .flatten()
    } else {
        None
    };
    // Cursor texture upload rides along; drawn per-present when ready.
    if render_ui {
        crate::shim_bridge::with_canvas(|shm| unsafe {
            crate::canvas::prepare_cursor(sc.fns, cmd, swapchain.as_raw(), shm)
        });
    }
    let show_ui = panel.is_some() && ui_usable(sc.render_pass, sc.pipeline);
    let subresource = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let captured = ira_overlay::capture::capture(cmd, image, fence, sc.extent);

    let src_layout = if captured {
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL
    } else {
        vk::ImageLayout::PRESENT_SRC_KHR
    };

    if show_ui {
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

        if let Some(panel) = panel {
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
            crate::canvas::draw_quad(sc.fns, cmd, swapchain.as_raw(), sc.extent, panel);
            crate::canvas::draw_cursor(sc.fns, cmd, swapchain.as_raw(), sc.extent, panel.0);
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
    let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS; 8];
    let wait_sems = present_info.p_wait_semaphores;
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
        // The fence was reset but nothing was submitted: it stays unsignaled
        // forever, so every later frame would time out. Bypass permanently
        // instead of wedging the game into a 200ms-per-frame slideshow.
        engage_bypass(&format!("submit failed ({submit_result:?})"));
        return chain_present(queue, present_info);
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
