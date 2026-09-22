//! Canvas compositor — the layer's only UI job.
//!
//! The GTK host renders the panel out-of-process into the canvas SHM region
//! (premultiplied Cairo ARGB32, i.e. BGRA byte order on little-endian).
//! Each present this module uploads the frame if `write_seq` advanced, then
//! draws a single alpha-blended quad anchored by [`canvas_panel_rect`].
//! GPU construction lives in `canvas_res`; this file owns per-swapchain
//! state and the upload/draw orchestration.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay_ipc::{
    canvas_panel_rect, CanvasShm, CANVAS_MAX_H, CANVAS_MAX_W,
};

use super::canvas_res::{self, CanvasTex, CursorTex};

/// 20-byte vertex: screen-space pos, texture uv, packed color.
#[repr(C)]
#[derive(Clone, Copy)]
struct CanvasVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

const _: () = assert!(std::mem::size_of::<CanvasVertex>() == 20);

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
pub type PanelDraw = ((f32, f32, f32, f32), (u32, u32));

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

/// Game X11 window behind a swapchain, with its cached root-space origin.
/// X11 reports pointer motion in root (screen) coordinates, but the panel
/// rect lives in the swapchain's client space: for fullscreen games at the
/// origin these coincide, for moved windows they differ by the origin.
/// The origin refreshes about once a second (windows rarely move); a
/// failed refresh keeps the previous one. No entry (non-X11 surfaces)
/// means root coordinates pass through untouched.
struct Xwin {
    dpy: usize,
    window: u64,
    ox: i32,
    oy: i32,
    age: u32,
}

/// Created VkSurface → (Display*, Window), filled by the surface hooks.
static SURFACE_XWIN: Mutex<Option<HashMap<u64, (usize, u64)>>> = Mutex::new(None);
/// Swapchain → game window translation state.
static SWAPCHAIN_XWIN: Mutex<Option<HashMap<u64, Xwin>>> = Mutex::new(None);

/// Remembers which X11 window a created VkSurface wraps. Called from the
/// surface-creation hooks with the raw create-info pointers.
pub fn record_xlib_surface(surface: u64, dpy: usize, window: u64) {
    SURFACE_XWIN
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(surface, (dpy, window));
}

/// Binds a swapchain to its surface's X11 window for input translation.
/// Unknown surfaces (Wayland, XCB) leave no entry: coordinates pass
/// through exactly as before.
pub fn bind_swapchain_window(swapchain: u64, surface: u64) {
    let xwin = SURFACE_XWIN
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&surface))
        .map(|&(dpy, window)| Xwin {
            dpy,
            window,
            ox: 0,
            oy: 0,
            // Refresh on first use: windowed games need their origin
            // before the first mapped event, not a second later.
            age: 60,
        });
    if let Some(xwin) = xwin {
        SWAPCHAIN_XWIN
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(swapchain, xwin);
    }
}

/// Pure core of the translation (unit-tested): subtract the origin.
fn apply_origin(rx: f32, ry: f32, ox: i32, oy: i32) -> (f32, f32) {
    (rx - ox as f32, ry - oy as f32)
}

type TranslateCoordsFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u64,
    u64,
    i32,
    i32,
    *mut i32,
    *mut i32,
    *mut u64,
) -> i32;
type DefaultRootFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> u64;

fn x11_fns() -> Option<(TranslateCoordsFn, DefaultRootFn)> {
    static FNS: OnceLock<Option<(TranslateCoordsFn, DefaultRootFn)>> = OnceLock::new();
    *FNS.get_or_init(|| unsafe {
        let translate = libc::dlsym(libc::RTLD_DEFAULT, c"XTranslateCoordinates".as_ptr());
        let root = libc::dlsym(libc::RTLD_DEFAULT, c"XDefaultRootWindow".as_ptr());
        if translate.is_null() || root.is_null() {
            return None;
        }
        let translate: TranslateCoordsFn = std::mem::transmute(translate);
        let root: DefaultRootFn = std::mem::transmute(root);
        Some((translate, root))
    })
}

/// Queries a window's root-space origin: where the window's own (0,0)
/// lands in root coordinates. `None` when Xlib is absent or the query
/// fails — callers keep the previous origin (or none).
fn query_origin(dpy: usize, window: u64) -> Option<(i32, i32)> {
    let (translate, root_of) = x11_fns()?;
    unsafe {
        let dpy = dpy as *mut std::ffi::c_void;
        let root = root_of(dpy);
        let (mut x, mut y) = (0i32, 0i32);
        let mut child = 0u64;
        // Direction matters: source is the window, destination is root.
        // (The reverse answers "where is root-(0,0) in window space",
        // i.e. the negated origin — verified against a live server.)
        let ok = translate(dpy, window, root, 0, 0, &mut x, &mut y, &mut child);
        (ok != 0).then_some((x, y))
    }
}

/// Publishes the swapchain extent for host window sizing: single writer
/// here (present hook), single reader on the host tick.
pub fn publish_extent(shm: &mut CanvasShm, extent: vk::Extent2D) {
    let hdr = shm.header_mut();
    hdr.screen_w
        .store(extent.width, std::sync::atomic::Ordering::SeqCst);
    hdr.screen_h
        .store(extent.height, std::sync::atomic::Ordering::SeqCst);
}

/// Translates root-space pointer coordinates into the swapchain's client
/// space for input mapping (cursor + commands share the result, so they
/// can never disagree). Fullscreen games sit at the origin and translate
/// to themselves; unknown swapchains pass through untouched.
pub fn translate_to_client(swapchain: u64, rx: f32, ry: f32) -> (f32, f32) {
    let mut map = SWAPCHAIN_XWIN.lock().unwrap();
    let Some(xwin) = map.as_mut().and_then(|m| m.get_mut(&swapchain)) else {
        return (rx, ry);
    };
    // Refresh about once a second (60 presents): windows rarely move, and
    // every refresh is a synchronous X round-trip on the game thread.
    xwin.age = xwin.age.wrapping_add(1);
    if xwin.age >= 60 {
        if let Some((ox, oy)) = query_origin(xwin.dpy, xwin.window) {
            if (ox, oy) != (xwin.ox, xwin.oy) && debug_log() {
                eprintln!("ira-overlay: window origin now ({ox},{oy})");
            }
            xwin.ox = ox;
            xwin.oy = oy;
        }
        xwin.age = 0;
    }
    apply_origin(rx, ry, xwin.ox, xwin.oy)
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

/// One-shot env check for compositor diagnostics (`IRA_OVERLAY_DEBUG=1`).
fn debug_log() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("IRA_OVERLAY_DEBUG").is_some())
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
        if debug_log() {
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
    if seq != tex.last_seq {
        if debug_log() {
            eprintln!("ira-overlay: upload frame {w}x{h} seq={seq}");
        }
        let len = w as usize * h as usize * 4;
        if len > shm.pixels().len() {
            return None;
        }
        upload(fns, cmd, tex, &shm.pixels()[..len], w, h);
        tex.last_seq = seq;
    }
    let rect = canvas_panel_rect(extent.width, extent.height, w, h, position);
    *LAST_PANEL.lock().unwrap() = Some((rect.0, rect.1, rect.2 / w as f32));
    if debug_log() {
        static LAST_GEOM_LOG: Mutex<(u32, u32, u32, u32, u32)> =
            Mutex::new((u32::MAX, 0, 0, 0, 0));
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
    Some((rect, (w, h)))
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
        let (rect, frame) = panel;
        draw(fns, cmd, entry, extent, rect, frame);
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
    if seq != cursor.last_seq {
        if debug_log() {
            eprintln!("ira-overlay: upload cursor {w}x{h} seq={seq}");
        }
        transition_and_copy(fns, cmd, cursor.upload_image(), pixels, w, h);
        cursor.last_seq = seq;
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
    let (entry, (mx, my)) = match (
        map.as_ref().and_then(|m| m.get(&swapchain)),
        mouse,
    ) {
        (Some(e), Some(m)) => (e, m),
        _ => return,
    };
    // Outside the game window the shim hears no motion (it only sees the
    // game's windows), so a stale position would freeze the cursor here
    // while the freed real cursor roams on. Hide instead: positions are
    // client space, the extent is the client space.
    if mx < 0.0
        || my < 0.0
        || mx >= extent.width as f32
        || my >= extent.height as f32
    {
        return;
    }
    // Outside the panel the real cursor is the interaction point; drawing
    // ours next to it (a frame behind, possibly mid-transition) is what
    // reads as fake. The panel rect is this same frame's, so the two can
    // never disagree about where its edge is.
    let (px, py, pw, ph) = panel;
    if mx < px || my < py || mx >= px + pw || my >= py + ph {
        return;
    };
    let cursor = match &entry.cursor {
        Some(c) if c.w > 0 && c.h > 0 => c,
        _ => return,
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
    std::ptr::copy_nonoverlapping(
        verts.as_ptr() as *const u8,
        cursor.vbo_ptr,
        std::mem::size_of_val(&verts),
    );

    (fns.cmd_bind_pipeline)(cmd, vk::PipelineBindPoint::GRAPHICS, entry.pipeline);
    let buffers = [cursor.vbo];
    let offsets = [0u64];
    (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, buffers.as_ptr(), offsets.as_ptr());
    (fns.cmd_bind_index_buffer)(cmd, cursor.ibo, 0, vk::IndexType::UINT32);
    (fns.cmd_bind_descriptor_sets)(
        cmd,
        vk::PipelineBindPoint::GRAPHICS,
        entry.pipeline_layout,
        0,
        1,
        &cursor.set,
        0,
        std::ptr::null(),
    );
    let push = CanvasPush {
        screen_size: [extent.width as f32, extent.height as f32],
        shape_size: [0.0, 0.0],
        corner_radius: 0.0,
        is_shape: 0,
    };
    (fns.cmd_push_constants)(
        cmd,
        entry.pipeline_layout,
        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        0,
        24,
        &push as *const CanvasPush as *const std::ffi::c_void,
    );
    (fns.cmd_draw_indexed)(cmd, 6, 1, 0, 0, 0);
}

/// Copies a packed `w×h` RGBA frame into the texture (records barriers +
/// copy into `cmd`, which must be recording). Seq bookkeeping is the
/// caller's job via `last_seq`.
unsafe fn upload(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    tex: &mut CanvasTex,
    pixels: &[u8],
    width: u32,
    height: u32,
) {
    transition_and_copy(fns, cmd, tex.upload_image(), pixels, width, height);
}

/// Shared staging upload: CPU copy, layout transitions, buffer→image.
/// Used by both the panel and cursor paths.
unsafe fn transition_and_copy(
    fns: DeviceFns,
    cmd: vk::CommandBuffer,
    target: super::canvas_res::UploadImage<'_>,    pixels: &[u8],
    width: u32,
    height: u32,
) {
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), target.staging_ptr, pixels.len());
    let subresource = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
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
    let to_dst = vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_access_mask(src_access)
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image)
        .subresource_range(subresource);
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

    let to_read = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image)
        .subresource_range(subresource);
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
    std::ptr::copy_nonoverlapping(
        verts.as_ptr() as *const u8,
        tex.vbo_ptr,
        std::mem::size_of_val(&verts),
    );

    (fns.cmd_bind_pipeline)(cmd, vk::PipelineBindPoint::GRAPHICS, entry.pipeline);
    let buffers = [tex.vbo];
    let offsets = [0u64];
    (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, buffers.as_ptr(), offsets.as_ptr());
    (fns.cmd_bind_index_buffer)(cmd, tex.ibo, 0, vk::IndexType::UINT32);
    (fns.cmd_bind_descriptor_sets)(
        cmd,
        vk::PipelineBindPoint::GRAPHICS,
        entry.pipeline_layout,
        0,
        1,
        &tex.set,
        0,
        std::ptr::null(),
    );
    let push = CanvasPush {
        screen_size: [extent.width as f32, extent.height as f32],
        shape_size: [0.0, 0.0],
        corner_radius: 0.0,
        is_shape: 0,
    };
    (fns.cmd_push_constants)(
        cmd,
        entry.pipeline_layout,
        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        0,
        24,
        &push as *const CanvasPush as *const std::ffi::c_void,
    );
    (fns.cmd_draw_indexed)(cmd, 6, 1, 0, 0, 0);
}

#[cfg(test)]
mod tests {
    use super::apply_origin;

    #[test]
    fn test_apply_origin_fullscreen_at_origin_is_identity() {
        assert_eq!(apply_origin(398.0, 21.0, 0, 0), (398.0, 21.0));
    }

    #[test]
    fn test_apply_origin_subtracts_window_offset() {
        // Windowed game at root offset (100, 50): root (398, 21) is
        // client (298, -29).
        assert_eq!(apply_origin(398.0, 21.0, 100, 50), (298.0, -29.0));
    }
}
