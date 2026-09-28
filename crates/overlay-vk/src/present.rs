use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ash::vk;
use ash::vk::Handle;

use crate::types::*;

/// Fail-closed kill switches: a failed composite path must degrade to "no
/// overlay", never to a wedged game.
pub(crate) static DEVICE_LOST: AtomicBool = AtomicBool::new(false);
/// Consecutive fence timeouts. A wedged queue times out forever; after a
/// short run the layer stops touching the GPU at all (same as DEVICE_LOST).
static FENCE_TIMEOUTS: AtomicU32 = AtomicU32::new(0);
const MAX_FENCE_TIMEOUTS: u32 = 5;

/// Max game wait semaphores forwarded through our submit. More than this and
/// we chain straight through: `wait_stages` in `submit_and_present` is sized
/// to match — the two must stay in sync.
const MAX_WAIT_SEMS: usize = 8;

/// Per-frame render targets resolved once per present and threaded through
/// the record/submit helpers together.
#[derive(Clone, Copy)]
struct Frame {
    cmd: vk::CommandBuffer,
    sem: vk::Semaphore,
    fence: vk::Fence,
    fb: vk::Framebuffer,
    image: vk::Image,
}

/// Host-frame watchdog state (see `watch_host_frames`): kept at file scope
/// instead of function statics so the policy reads top-down.
static WATCH_LAST_SEQ: AtomicU32 = AtomicU32::new(u32::MAX);
static WATCH_LAST_CHANGE_MS: AtomicU32 = AtomicU32::new(0);
static WATCH_WARNED: AtomicBool = AtomicBool::new(false);

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
    let now = crate::canvas::now_ms();
    if !overlay_visible {
        WATCH_LAST_CHANGE_MS.store(now, Ordering::Relaxed);
        WATCH_WARNED.store(false, Ordering::Relaxed);
        return;
    }
    if let Some(seq) = crate::shim_bridge::with_canvas(|shm| shm.frame_meta().2) {
        if seq != WATCH_LAST_SEQ.load(Ordering::Relaxed) {
            WATCH_LAST_SEQ.store(seq, Ordering::Relaxed);
            WATCH_LAST_CHANGE_MS.store(now, Ordering::Relaxed);
            WATCH_WARNED.store(false, Ordering::Relaxed);
            return;
        }
    }
    if now.wrapping_sub(WATCH_LAST_CHANGE_MS.load(Ordering::Relaxed)) > 5000
        && !WATCH_WARNED.swap(true, Ordering::Relaxed)
    {
        eprintln!(
            "ira-overlay: HOST SILENT 5s while visible (no new frames): host not spawned, \
             crashed, or window never mapped — input is dead until frames flow"
        );
    }
}

fn poll_frame_inputs() {
    crate::wayland::dispatch();
    crate::evdev::init();
    if !crate::shim_bridge::has_sdl_hooks() {
        crate::evdev::poll();
    }
    crate::shim_bridge::increment_present_count();
}

/// Polls shim input, drains host actions, and syncs visibility.
/// Returns (overlay_visible, capture_active).
fn forward_frame_events(present_info: &vk::PresentInfoKHR) -> (bool, bool) {
    crate::shim_bridge::poll_and_forward(present_swapchain(present_info));
    crate::shim_bridge::drain_host_actions();
    crate::shim_bridge::sync_visible_to_shm();
    let overlay_visible = crate::shim_bridge::is_visible();
    let capture_active = ira_overlay::capture::is_capture_active();
    watch_host_frames(overlay_visible);
    (overlay_visible, capture_active)
}

/// Looks up the first present swapchain. Returns (swapchain, image index,
/// data); None chains straight through.
fn load_swapchain(
    present_info: &vk::PresentInfoKHR,
) -> Option<(vk::SwapchainKHR, u32, SwapchainData)> {
    // Read up front: input polling translates coordinates per swapchain.
    let swapchain = unsafe { present_info.p_swapchains.read() };
    let image_index = unsafe { *present_info.p_image_indices };
    let map = SWAPCHAINS.lock().unwrap();
    let sc = map
        .as_ref()
        .and_then(|m| m.get(&swapchain.as_raw()))
        .cloned()?;
    Some((swapchain, image_index, sc))
}

/// Waits the frame fence, runs capture readback, and begins the command
/// buffer. False chains straight through.
unsafe fn begin_frame(sc: &SwapchainData, cmd: vk::CommandBuffer, fence: vk::Fence) -> bool {
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
        return false;
    }
    FENCE_TIMEOUTS.store(0, Ordering::Relaxed);
    ira_overlay::capture::check_and_readback();
    let _ = (sc.fns.reset_fences)(sc.device, 1, &fence);
    let _ = (sc.fns.reset_cmd_buffer)(cmd, vk::CommandBufferResetFlags::empty());

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let _ = (sc.fns.begin_cmd_buffer)(cmd, &begin_info);
    true
}

/// Uploads the panel frame and cursor texture before the swapchain
/// barriers: layout transitions on the canvas image must stay outside the
/// render pass. Returns the panel draw when there is one.
unsafe fn upload_canvas(
    sc: &SwapchainData,
    cmd: vk::CommandBuffer,
    swapchain: vk::SwapchainKHR,
) -> Option<crate::canvas::PanelDraw> {
    let position = crate::shim_bridge::overlay_position();
    let panel = crate::shim_bridge::with_canvas(|shm| unsafe {
        crate::canvas::prepare_upload(sc.fns, cmd, swapchain.as_raw(), sc.extent, position, shm)
    })
    .flatten();
    // Cursor texture upload rides along; drawn per-present when ready.
    crate::shim_bridge::with_canvas(|shm| unsafe {
        crate::canvas::prepare_cursor(sc.fns, cmd, swapchain.as_raw(), shm)
    });
    panel
}

/// Barriers the swapchain image into the render pass, draws the panel +
/// cursor, and closes the pass. `src_layout` reflects whether capture
/// already moved the image to TRANSFER_SRC_OPTIMAL.
unsafe fn record_ui_pass(
    sc: &SwapchainData,
    swapchain: vk::SwapchainKHR,
    frame: Frame,
    panel: crate::canvas::PanelDraw,
    captured: bool,
    src_layout: vk::ImageLayout,
) {
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
    let barrier = crate::types::image_barrier(
        frame.image,
        src_layout,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        src_access,
        vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
    );
    (sc.fns.cmd_pipeline_barrier)(
        frame.cmd,
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
        .framebuffer(frame.fb)
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: sc.extent,
        })
        .clear_values(std::slice::from_ref(&clear));
    (sc.fns.cmd_begin_render_pass)(frame.cmd, &rp_begin, vk::SubpassContents::INLINE);

    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: sc.extent.width as f32,
        height: sc.extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    (sc.fns.cmd_set_viewport)(frame.cmd, 0, 1, &viewport as *const vk::Viewport);
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: sc.extent,
    };
    (sc.fns.cmd_set_scissor)(frame.cmd, 0, 1, &scissor as *const vk::Rect2D);
    crate::canvas::draw_quad(sc.fns, frame.cmd, swapchain.as_raw(), sc.extent, panel);
    crate::canvas::draw_cursor(sc.fns, frame.cmd, swapchain.as_raw(), sc.extent, panel.rect);

    (sc.fns.cmd_end_render_pass)(frame.cmd);
}

/// Returns a capture-only frame to PRESENT_SRC_KHR after the blit moved it
/// to TRANSFER_SRC_OPTIMAL. No render pass, no UI.
unsafe fn record_capture_barrier(sc: &SwapchainData, cmd: vk::CommandBuffer, image: vk::Image) {
    let barrier = crate::types::image_barrier(
        image,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        vk::ImageLayout::PRESENT_SRC_KHR,
        vk::AccessFlags::TRANSFER_READ,
        vk::AccessFlags::MEMORY_READ,
    );
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

/// Ends the command buffer, submits it chained behind the game's wait
/// semaphores, and presents waiting on our own semaphore instead.
/// A failed submit bypasses permanently: the fence was already reset, so
/// chaining on would orphan it and stall every later frame.
unsafe fn submit_and_present(
    sc: &SwapchainData,
    queue: vk::Queue,
    cmd: vk::CommandBuffer,
    sem: vk::Semaphore,
    fence: vk::Fence,
    present_info: &vk::PresentInfoKHR,
) -> vk::Result {
    let _ = (sc.fns.end_cmd_buffer)(cmd);

    let wait_sem_count = present_info.wait_semaphore_count;
    let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS; MAX_WAIT_SEMS];
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
        engage_bypass(&format!("submit failed ({submit_result:?})"));
        return chain_present(queue, present_info);
    }

    let mut new_present_info = *present_info;
    new_present_info.p_wait_semaphores = &sem;
    new_present_info.wait_semaphore_count = 1;

    (sc.fns.queue_present)(queue, &new_present_info)
}

pub unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    present_info: *const vk::PresentInfoKHR,
) -> vk::Result {
    let present_info = &*present_info;

    poll_frame_inputs();
    if DEVICE_LOST.load(Ordering::Relaxed) {
        return chain_present(queue, present_info);
    }
    let (overlay_visible, capture_active) = forward_frame_events(present_info);
    if overlay_visible {
        crate::shim_bridge::enforce_cursor();
    }

    if present_info.swapchain_count == 0 {
        return chain_present(queue, present_info);
    }
    // Published on every present the layer sees — visible or not — so the
    // host knows the window size before the first show instead of morphing
    // after first input.
    publish_extent_for_present(present_info);
    if !overlay_visible && !capture_active {
        return chain_present(queue, present_info);
    }

    let Some((swapchain, image_index, sc)) = load_swapchain(present_info) else {
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
    if present_info.wait_semaphore_count as usize > MAX_WAIT_SEMS {
        return chain_present(queue, present_info);
    }

    let frame = Frame {
        cmd: sc.cmd_buffers[idx],
        sem: sc.semaphores[idx],
        fence: sc.fences[idx],
        fb: sc.framebuffers[idx],
        image: sc.images[idx],
    };

    if !begin_frame(&sc, frame.cmd, frame.fence) {
        return chain_present(queue, present_info);
    }

    // Canvas upload: runs before the swapchain barriers: layout transitions
    // on the canvas image must stay outside the render pass.
    let panel = if render_ui {
        upload_canvas(&sc, frame.cmd, swapchain)
    } else {
        None
    };
    let captured = ira_overlay::capture::capture(frame.cmd, frame.image, frame.fence, sc.extent);
    let src_layout = if captured {
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL
    } else {
        vk::ImageLayout::PRESENT_SRC_KHR
    };

    // Fail closed: a null pipeline/render-pass skips compositing rather
    // than reaching the driver.
    match panel {
        Some(panel) if ui_usable(sc.render_pass, sc.pipeline) => {
            record_ui_pass(&sc, swapchain, frame, panel, captured, src_layout)
        }
        _ if captured => record_capture_barrier(&sc, frame.cmd, frame.image),
        _ => {}
    }

    submit_and_present(&sc, queue, frame.cmd, frame.sem, frame.fence, present_info)
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
