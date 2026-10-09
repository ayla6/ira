//! Vulkan frame capture (record path, present thread).
//!
//! Staging buffers rotate swapchain → staging → readback thread → free list:
//! `capture` records barriers + blit/copy into the game's command buffer
//! (called from the layer's present hook); `check_and_readback` moves
//! fence-signaled frames to the readback thread. Owning fences stays with
//! the caller (present hook), which resets them — this module only polls
//! `get_fence_status`.

use ash::vk;

use super::{IntermediateImage, PendingFrame, StagingBuffer};
use crate::types::DeviceFns;

/// Reclaims readback-recycled buffers whose size still matches; the rest
/// (stale extent) defer to `free_deferred`.
pub(super) fn reclaim_free(state: &mut super::State) {
    let expected = state.staging_size;
    while let Ok(staging) = state.free_rx.try_recv() {
        if staging.size == expected {
            state.free.push(staging);
        } else {
            super::DEFERRED_BUFFERS.lock().unwrap().push(staging);
        }
    }
}

pub fn check_and_readback() {
    let mut guard = super::STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };

    reclaim_free(state);

    let fns = state.fns;
    let device = state.device;

    let mut still_pending = Vec::new();
    for frame in state.pending.drain(..) {
        let status = unsafe { (fns.get_fence_status)(device, frame.fence) };
        if status == vk::Result::SUCCESS {
            let _ = state.ready_tx.send(frame);
        } else {
            still_pending.push(frame);
        }
    }
    state.pending = still_pending;
}

/// Flushes every pending frame to the readback thread without fence checks.
/// Teardown-only: the device is going away, so waiting is pointless.
pub fn drain_pending() {
    let mut guard = super::STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };
    reclaim_free(state);
    for frame in state.pending.drain(..) {
        let _ = state.ready_tx.send(frame);
    }
}

/// # Safety
/// `cmd` must be in the recording state. `src_image` must be a valid swapchain
/// image. `fence` must be unsignaled; the caller retains and resets it.
pub unsafe fn capture(
    cmd: vk::CommandBuffer,
    src_image: vk::Image,
    fence: vk::Fence,
    image_extent: vk::Extent2D,
) -> bool {
    let mut guard = super::STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return false;
    };

    if image_extent.width != state.extent.width || image_extent.height != state.extent.height {
        return false;
    }

    let is_screenshot = state.screenshot_requested;
    if !is_screenshot && !super::is_recording() && !super::threads::replay_buffer_active() {
        return false;
    }

    let Some(staging) = state.free.pop() else {
        return false;
    };

    if is_screenshot {
        state.screenshot_requested = false;
    }

    let fns = state.fns;
    let extent = state.extent;
    let use_blit = state.use_blit;
    let src_format = state.format;

    let color_sub = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    if use_blit {
        let intermediate = state
            .intermediate
            .as_ref()
            .expect("blit enabled but no intermediate");
        unsafe {
            record_blit(
                fns,
                cmd,
                src_image,
                &staging,
                intermediate,
                extent,
                color_sub,
            )
        };
    } else {
        unsafe { record_copy(fns, cmd, src_image, &staging, extent, color_sub) };
    }

    state.pending.push(PendingFrame {
        staging,
        width: extent.width,
        height: extent.height,
        use_blit,
        src_format,
        fence,
        is_screenshot,
    });

    true
}

/// Records one pipeline barrier. The boilerplate (no semaphores, one range)
/// is identical at every call site; only stages and barriers vary.
unsafe fn cmd_barrier(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
    barriers: &[vk::ImageMemoryBarrier<'_>],
) {
    (fns.cmd_pipeline_barrier)(
        cmd,
        src_stage,
        dst_stage,
        vk::DependencyFlags::empty(),
        0,
        std::ptr::null(),
        0,
        std::ptr::null(),
        barriers.len() as u32,
        barriers.as_ptr(),
    );
}

fn full_offsets(extent: vk::Extent2D) -> (vk::Offset3D, vk::Offset3D) {
    (
        vk::Offset3D { x: 0, y: 0, z: 0 },
        vk::Offset3D {
            x: extent.width as i32,
            y: extent.height as i32,
            z: 1,
        },
    )
}

/// Shared by both record paths.
fn copy_region(extent: vk::Extent2D) -> vk::BufferImageCopy {
    vk::BufferImageCopy::default()
        .buffer_offset(0)
        .buffer_row_length(extent.width)
        .buffer_image_height(extent.height)
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
}

/// Shared by the blit and direct-copy record paths.
fn src_to_transfer_barrier(
    src_image: vk::Image,
    color_sub: vk::ImageSubresourceRange,
) -> vk::ImageMemoryBarrier<'static> {
    vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_access_mask(vk::AccessFlags::NONE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(src_image)
        .subresource_range(color_sub)
}

unsafe fn record_blit(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    src_image: vk::Image,
    staging: &StagingBuffer,
    intermediate: &IntermediateImage,
    extent: vk::Extent2D,
    color_sub: vk::ImageSubresourceRange,
) {
    let src_barrier = src_to_transfer_barrier(src_image, color_sub);
    let dst_barrier = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_access_mask(vk::AccessFlags::NONE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(intermediate.image)
        .subresource_range(color_sub);

    let barriers = [src_barrier, dst_barrier];
    cmd_barrier(
        fns,
        cmd,
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::TRANSFER,
        &barriers,
    );

    let (origin, far) = full_offsets(extent);
    let blit_region = vk::ImageBlit::default()
        .src_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .src_offsets([origin, far])
        .dst_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .dst_offsets([origin, far]);

    (fns.cmd_blit_image)(
        cmd,
        src_image,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        intermediate.image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        1,
        &blit_region,
        vk::Filter::LINEAR,
    );

    let post_blit = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(intermediate.image)
        .subresource_range(color_sub);

    cmd_barrier(
        fns,
        cmd,
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::TRANSFER,
        std::slice::from_ref(&post_blit),
    );

    let copy_region = copy_region(extent);
    (fns.cmd_copy_image_to_buffer)(
        cmd,
        intermediate.image,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        staging.buffer,
        1,
        &copy_region,
    );
}

/// Format conversion happens on the CPU in the readback thread.
unsafe fn record_copy(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    src_image: vk::Image,
    staging: &StagingBuffer,
    extent: vk::Extent2D,
    color_sub: vk::ImageSubresourceRange,
) {
    let src_barrier = src_to_transfer_barrier(src_image, color_sub);
    cmd_barrier(
        fns,
        cmd,
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::TRANSFER,
        std::slice::from_ref(&src_barrier),
    );

    let copy_region = copy_region(extent);
    (fns.cmd_copy_image_to_buffer)(
        cmd,
        src_image,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        staging.buffer,
        1,
        &copy_region,
    );
}

pub(super) fn src_bpp(format: vk::Format) -> u64 {
    match format {
        vk::Format::R16G16B16A16_SFLOAT | vk::Format::R16G16B16A16_UNORM => 8,
        _ => 4,
    }
}

pub(super) fn check_blit_support(
    fns: DeviceFns,
    physical_device: vk::PhysicalDevice,
    format: vk::Format,
) -> bool {
    let mut props = vk::FormatProperties::default();
    unsafe {
        (fns.get_format_props)(physical_device, format, &mut props);
    }
    props
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::BLIT_SRC)
}
