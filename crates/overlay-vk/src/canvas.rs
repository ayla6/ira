//! Canvas compositor — the layer's only UI job.
//!
//! The GTK host renders the panel out-of-process into the canvas SHM region
//! (premultiplied Cairo ARGB32, i.e. BGRA byte order on little-endian).
//! Each present this module uploads the frame if `write_seq` advanced, then
//! draws a single alpha-blended quad anchored by [`canvas_panel_rect`].
//! GPU construction lives in `canvas_res`; this file owns per-swapchain
//! state and the upload/draw orchestration.

use std::collections::HashMap;
use std::sync::Mutex;

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay_ipc::{canvas_panel_rect, CanvasShm, CANVAS_MAX_H, CANVAS_MAX_W};

use super::canvas_res::{self, CanvasTex, CursorTex};

/// 20-byte vertex: screen-space pos, texture uv, packed color.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CanvasVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

const _: () = assert!(std::mem::size_of::<CanvasVertex>() == 20);
/// Quad vertex-buffer footprint (4 verts) shared with `canvas_res`.
/// Derived from the type so the assert above guards it.
pub(crate) const QUAD_VBO_BYTES: u64 = 4 * std::mem::size_of::<CanvasVertex>() as u64;
/// Quad index-buffer footprint (6 u32 indices) shared with `canvas_res`.
pub(crate) const QUAD_IBO_BYTES: u64 = 6 * std::mem::size_of::<u32>() as u64;

/// 24-byte push constants, same layout as the glyph pipeline.
#[repr(C)]
struct CanvasPush {
    screen_size: [f32; 2],
    shape_size: [f32; 2],
    corner_radius: f32,
    is_shape: u32,
}

const _: () = assert!(std::mem::size_of::<CanvasPush>() == 24);

/// Panel rect + canvas frame size returned per composited frame.
#[derive(Clone, Copy, Debug)]
pub struct PanelDraw {
    /// Screen-space panel rect (x, y, w, h).
    pub rect: (f32, f32, f32, f32),
    /// Canvas content size (w, h).
    pub size: (u32, u32),
}

struct CanvasEntry {
    tex: CanvasTex,
    cursor: Option<CursorTex>,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
}

/// Per-swapchain canvas entries, keyed by raw swapchain handle.
static CANVASES: Mutex<Option<HashMap<u64, CanvasEntry>>> = Mutex::new(None);
/// Last composited panel (x, y, scale) for screen→canvas input translation.
/// `None` while nothing is drawn — input events are dropped then.
static LAST_PANEL: Mutex<Option<(f32, f32, f32)>> = Mutex::new(None);
/// Last seen pointer position in client pixels. Drives the layer-drawn
/// cursor; `None` until the first mouse event. All writers store the same
/// client space the panel rect lives in (shim input translated, Wayland
/// surface-local already client), so cursor and commands always agree.
static LAST_MOUSE: Mutex<Option<(f32, f32)>> = Mutex::new(None);

/// Publishes the swapchain extent for host window sizing: single writer
/// here (present hook), single reader on the host tick.
pub fn publish_extent(shm: &mut CanvasShm, extent: vk::Extent2D) {
    let hdr = shm.header_mut();
    hdr.screen_w
        .store(extent.width, std::sync::atomic::Ordering::SeqCst);
    hdr.screen_h
        .store(extent.height, std::sync::atomic::Ordering::SeqCst);
}

/// Records the pointer position (client pixels) for the layer cursor.
pub fn set_last_mouse(x: f32, y: f32) {
    *LAST_MOUSE.lock().unwrap() = Some((x, y));
}

/// Wall-clock milliseconds (wrapping). For throttling diagnostics.
pub fn now_ms() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// Translates screen pixels to canvas pixels. Returns `None` when no panel
/// is drawn (callers drop the event).
pub fn screen_to_canvas(x: f32, y: f32) -> Option<(f32, f32)> {
    let panel = *LAST_PANEL.lock().unwrap();
    let (px, py, scale) = panel?;
    if scale <= 0.0 {
        return None;
    }
    Some(((x - px) / scale, (y - py) / scale))
}

/// Creates the full canvas bundle for one swapchain: texture resources +
/// textured-quad pipeline. Inserts the entry into the per-swapchain map and
/// returns the pipeline handles for swapchain bookkeeping.
pub unsafe fn create_bundle(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    render_pass: vk::RenderPass,
    swapchain: u64,
) -> Option<(
    vk::Pipeline,
    vk::PipelineLayout,
    vk::ShaderModule,
    vk::ShaderModule,
)> {
    let tex = canvas_res::create(fns, device, physical_device)?;
    let (pipeline_layout, pipeline, shader_vert, shader_frag) =
        canvas_res::create_pipeline(fns, device, render_pass, tex.set_layout);
    // A failed cursor only loses the pointer, never the panel.
    let cursor =
        canvas_res::create_cursor(fns, device, physical_device, tex.sampler, tex.set_layout);
    CANVASES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            swapchain,
            CanvasEntry {
                tex,
                cursor,
                pipeline,
                pipeline_layout,
            },
        );
    Some((pipeline, pipeline_layout, shader_vert, shader_frag))
}

/// Last reported frame meta, so the per-frame early-out stays silent
/// unless something actually changes.
static LAST_META_LOG: Mutex<(u32, u32, u32, u32)> = Mutex::new((u32::MAX, 0, 0, 0));

/// Uploads a new frame if `write_seq` advanced and returns the panel rect +
/// frame size to draw. Also stores the ack and the input-translation panel.
pub unsafe fn prepare_upload(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    swapchain: u64,
    extent: vk::Extent2D,
    position: u32,
    shm: &mut CanvasShm,
) -> Option<PanelDraw> {
    let (w, h, seq, vis) = shm.frame_meta();
    if vis == 0 || w == 0 || h == 0 || seq == 0 {
        *LAST_PANEL.lock().unwrap() = None;
        if crate::types::debug_log() {
            let mut last = LAST_META_LOG.lock().unwrap();
            if *last != (w, h, seq, vis) {
                eprintln!("ira-overlay: no frame (w={w} h={h} seq={seq} vis={vis})");
                *last = (w, h, seq, vis);
            }
        }
        return None;
    }
    let mut map = CANVASES.lock().unwrap();
    let entry = map.as_mut()?.get_mut(&swapchain)?;
    let tex = &mut entry.tex;
    if seq != tex.core.last_seq {
        if crate::types::debug_log() {
            eprintln!("ira-overlay: upload frame {w}x{h} seq={seq}");
        }
        let len = w as usize * h as usize * 4;
        if len > shm.pixels().len() {
            return None;
        }
        transition_and_copy(
            fns,
            cmd,
            tex.core.upload_image(),
            &shm.pixels()[..len],
            w,
            h,
        );
        tex.core.last_seq = seq;
    }
    let rect = canvas_panel_rect(extent.width, extent.height, w, h, position);
    *LAST_PANEL.lock().unwrap() = Some((rect.0, rect.1, rect.2 / w as f32));
    if crate::types::debug_log() {
        static LAST_GEOM_LOG: Mutex<(u32, u32, u32, u32, u32)> = Mutex::new((u32::MAX, 0, 0, 0, 0));
        let mut last = LAST_GEOM_LOG.lock().unwrap();
        let geom = (extent.width, extent.height, w, h, position);
        if *last != geom {
            *last = geom;
            eprintln!(
                "ira-overlay: panel extent={}x{} frame={}x{} rect=({:.0},{:.0},{:.0}x{:.0}) pos={}",
                extent.width, extent.height, w, h, rect.0, rect.1, rect.2, rect.3, position
            );
        }
    }
    shm.header_mut()
        .ack_seq
        .store(seq, std::sync::atomic::Ordering::SeqCst);
    Some(PanelDraw { rect, size: (w, h) })
}

/// Draws the previously uploaded frame. Call inside an active render pass.
pub unsafe fn draw_quad(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    swapchain: u64,
    extent: vk::Extent2D,
    panel: PanelDraw,
) {
    let map = CANVASES.lock().unwrap();
    if let Some(entry) = map.as_ref().and_then(|m| m.get(&swapchain)) {
        draw(fns, cmd, entry, extent, panel.rect, panel.size);
    }
}

pub unsafe fn destroy_canvas(fns: DeviceFns, device: vk::Device, swapchain: u64) {
    let entry = CANVASES
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&swapchain));
    if let Some(entry) = entry {
        if let Some(cursor) = &entry.cursor {
            canvas_res::destroy_cursor(fns, device, cursor);
        }
        canvas_res::destroy(fns, device, &entry.tex);
    }
}

/// Uploads the cursor image if `cursor_seq` advanced. Returns true when a
/// cursor is ready to draw (published at least once).
pub unsafe fn prepare_cursor(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    swapchain: u64,
    shm: &CanvasShm,
) -> bool {
    let (w, h, xhot, yhot, seq, pixels) = shm.cursor_frame();
    if w == 0 || h == 0 || seq == 0 || pixels.is_empty() {
        return false;
    }
    let mut map = CANVASES.lock().unwrap();
    let Some(entry) = map.as_mut().and_then(|m| m.get_mut(&swapchain)) else {
        return false;
    };
    let Some(cursor) = entry.cursor.as_mut() else {
        return false;
    };
    if seq != cursor.core.last_seq {
        if crate::types::debug_log() {
            eprintln!("ira-overlay: upload cursor {w}x{h} seq={seq}");
        }
        transition_and_copy(fns, cmd, cursor.core.upload_image(), pixels, w, h);
        cursor.core.last_seq = seq;
        cursor.w = w;
        cursor.h = h;
        cursor.xhot = xhot;
        cursor.yhot = yhot;
    }
    true
}

/// Draws the cursor at the last seen pointer position (hotspot-adjusted,
/// native size), but only over the panel itself: outside it the freed real
/// cursor owns the screen, and a lagging double exposes exactly how drawn
/// the overlay one is. Call inside an active render pass after the panel.
pub unsafe fn draw_cursor(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    swapchain: u64,
    extent: vk::Extent2D,
    panel: (f32, f32, f32, f32),
) {
    let mouse = *LAST_MOUSE.lock().unwrap();
    let map = CANVASES.lock().unwrap();
    let Some(entry) = map.as_ref().and_then(|m| m.get(&swapchain)) else {
        return;
    };
    let Some((verts, bind)) = cursor_verts(entry, mouse, extent, panel) else {
        return;
    };
    draw_textured_quad(fns, cmd, bind, &verts, extent);
}

/// Resolves the cursor quad for this frame: mouse + entry + visibility +
/// panel clipping. None means draw nothing.
fn cursor_verts(
    entry: &CanvasEntry,
    mouse: Option<(f32, f32)>,
    extent: vk::Extent2D,
    panel: (f32, f32, f32, f32),
) -> Option<([CanvasVertex; 4], QuadBind)> {
    let (mx, my) = mouse?;
    // Outside the game window the shim hears no motion (it only sees the
    // game's windows), so a stale position would freeze the cursor here
    // while the freed real cursor roams on. Hide instead: positions are
    // client space, the extent is the client space.
    if mx < 0.0 || my < 0.0 || mx >= extent.width as f32 || my >= extent.height as f32 {
        return None;
    }
    // Outside the panel the real cursor is the interaction point; drawing
    // ours next to it (a frame behind, possibly mid-transition) is what
    // reads as fake. The panel rect is this same frame's, so the two can
    // never disagree about where its edge is.
    let (px, py, pw, ph) = panel;
    if mx < px || my < py || mx >= px + pw || my >= py + ph {
        return None;
    };
    let cursor = match &entry.cursor {
        Some(c) if c.w > 0 && c.h > 0 => c,
        _ => return None,
    };
    let x = mx - cursor.xhot as f32;
    let y = my - cursor.yhot as f32;
    let w = cursor.w as f32;
    let h = cursor.h as f32;
    let u1 = cursor.w as f32 / ira_overlay_ipc::CURSOR_MAX as f32;
    let v1 = cursor.h as f32 / ira_overlay_ipc::CURSOR_MAX as f32;
    let white = [255, 255, 255, 255];
    let verts = [
        CanvasVertex {
            pos: [x, y],
            uv: [0.0, 0.0],
            color: white,
        },
        CanvasVertex {
            pos: [x + w, y],
            uv: [u1, 0.0],
            color: white,
        },
        CanvasVertex {
            pos: [x + w, y + h],
            uv: [u1, v1],
            color: white,
        },
        CanvasVertex {
            pos: [x, y + h],
            uv: [0.0, v1],
            color: white,
        },
    ];
    Some((
        verts,
        QuadBind {
            pipeline: entry.pipeline,
            layout: entry.pipeline_layout,
            vbo: cursor.core.vbo,
            vbo_ptr: cursor.core.vbo_ptr,
            ibo: cursor.core.ibo,
            set: cursor.core.set,
        },
    ))
}

/// Shared staging upload: CPU copy, layout transitions, buffer→image.
/// Used by both the panel and cursor paths.
unsafe fn transition_and_copy(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    target: super::canvas_res::UploadImage<'_>,
    pixels: &[u8],
    width: u32,
    height: u32,
) {
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), target.staging_ptr, pixels.len());
    let (old_layout, src_stage, src_access) = if *target.initialized {
        (
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::AccessFlags::SHADER_READ,
        )
    } else {
        (
            vk::ImageLayout::UNDEFINED,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::AccessFlags::empty(),
        )
    };
    let to_dst = crate::types::image_barrier(
        target.image,
        old_layout,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        src_access,
        vk::AccessFlags::TRANSFER_WRITE,
    );
    (fns.cmd_pipeline_barrier)(
        cmd,
        src_stage,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        0,
        std::ptr::null(),
        0,
        std::ptr::null(),
        1,
        &to_dst,
    );

    let region = vk::BufferImageCopy::default()
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
        .image_extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        });
    (fns.cmd_copy_buffer_to_image)(
        cmd,
        target.staging,
        target.image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        1,
        &region,
    );

    let to_read = crate::types::image_barrier(
        target.image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        vk::AccessFlags::TRANSFER_WRITE,
        vk::AccessFlags::SHADER_READ,
    );
    (fns.cmd_pipeline_barrier)(
        cmd,
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        0,
        std::ptr::null(),
        0,
        std::ptr::null(),
        1,
        &to_read,
    );
    *target.initialized = true;
}

/// Bindings for one textured-quad draw: pipeline + vertex/index buffers +
/// descriptor set. Panel and cursor draws fill this from their own texture
/// resources and share the record path below.
#[derive(Clone, Copy)]
struct QuadBind {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    vbo: vk::Buffer,
    vbo_ptr: *mut u8,
    ibo: vk::Buffer,
    set: vk::DescriptorSet,
}

/// Binds one textured quad (vertices, pipeline, descriptor, push constants)
/// and draws it. Shared by the panel and cursor paths, which differ only in
/// geometry and texture resources.
unsafe fn draw_textured_quad(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    bind: QuadBind,
    verts: &[CanvasVertex; 4],
    extent: vk::Extent2D,
) {
    std::ptr::copy_nonoverlapping(
        verts.as_ptr() as *const u8,
        bind.vbo_ptr,
        std::mem::size_of_val(verts),
    );

    (fns.cmd_bind_pipeline)(cmd, vk::PipelineBindPoint::GRAPHICS, bind.pipeline);
    let buffers = [bind.vbo];
    let offsets = [0u64];
    (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, buffers.as_ptr(), offsets.as_ptr());
    (fns.cmd_bind_index_buffer)(cmd, bind.ibo, 0, vk::IndexType::UINT32);
    (fns.cmd_bind_descriptor_sets)(
        cmd,
        vk::PipelineBindPoint::GRAPHICS,
        bind.layout,
        0,
        1,
        &bind.set,
        0,
        std::ptr::null(),
    );
    let push = CanvasPush {
        screen_size: [extent.width as f32, extent.height as f32],
        // The shader shares its push layout with the glyph pipeline (24
        // bytes); only screen_size carries meaning for textured quads.
        shape_size: [0.0, 0.0],
        corner_radius: 0.0,
        is_shape: 0,
    };
    (fns.cmd_push_constants)(
        cmd,
        bind.layout,
        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        0,
        24,
        &push as *const CanvasPush as *const std::ffi::c_void,
    );
    (fns.cmd_draw_indexed)(cmd, 6, 1, 0, 0, 0);
}

/// Draws the canvas quad. `rect` is the screen-space panel rect (x, y, w, h),
/// `frame` the canvas content size. Must be called inside an active render
/// pass with full-screen viewport/scissor already set.
unsafe fn draw(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    entry: &CanvasEntry,
    extent: vk::Extent2D,
    rect: (f32, f32, f32, f32),
    frame: (u32, u32),
) {
    let tex = &entry.tex;
    let (x, y, w, h) = rect;
    let u1 = frame.0 as f32 / CANVAS_MAX_W as f32;
    let v1 = frame.1 as f32 / CANVAS_MAX_H as f32;
    let white = [255, 255, 255, 255];
    let verts = [
        CanvasVertex {
            pos: [x, y],
            uv: [0.0, 0.0],
            color: white,
        },
        CanvasVertex {
            pos: [x + w, y],
            uv: [u1, 0.0],
            color: white,
        },
        CanvasVertex {
            pos: [x + w, y + h],
            uv: [u1, v1],
            color: white,
        },
        CanvasVertex {
            pos: [x, y + h],
            uv: [0.0, v1],
            color: white,
        },
    ];
    draw_textured_quad(
        fns,
        cmd,
        QuadBind {
            pipeline: entry.pipeline,
            layout: entry.pipeline_layout,
            vbo: tex.core.vbo,
            vbo_ptr: tex.core.vbo_ptr,
            ibo: tex.core.ibo,
            set: tex.core.set,
        },
        &verts,
        extent,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_quad_buffer_sizes_match_vertex_layout() {
        assert_eq!(super::QUAD_VBO_BYTES, 80);
        assert_eq!(super::QUAD_IBO_BYTES, 24);
    }
}
