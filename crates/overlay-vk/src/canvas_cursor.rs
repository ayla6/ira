//! Layer-drawn cursor — upload + draw, split from `canvas.rs`.
//!
//! The cursor texture rides the panel upload path (`transition_and_copy`)
//! but draws separately: hotspot-adjusted, native size, panel-clipped so
//! the freed real cursor owns the screen outside the panel.

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay_ipc::CanvasShm;

use super::canvas::{quad_verts, CanvasEntry, QuadBind, CANVASES, LAST_MOUSE};

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
        super::canvas::transition_and_copy(fns, cmd, cursor.core.upload_image(), pixels, w, h);
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
    super::canvas::draw_textured_quad(fns, cmd, bind, &verts, extent);
}

/// Resolves the cursor quad for this frame: mouse + entry + visibility +
/// panel clipping. None means draw nothing.
fn cursor_verts(
    entry: &CanvasEntry,
    mouse: Option<(f32, f32)>,
    extent: vk::Extent2D,
    panel: (f32, f32, f32, f32),
) -> Option<([super::canvas::CanvasVertex; 4], QuadBind)> {
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
    let verts = quad_verts(x, y, w, h, u1, v1);
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
