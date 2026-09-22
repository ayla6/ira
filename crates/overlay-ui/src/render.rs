//! Offscreen frame rendering: widget tree → canvas pixels.
//!
//! Validated by the spike (`/tmp/opencode/gtk-spike`): `GtkWidgetPaintable`
//! → `Snapshot::to_node` → `GskRenderer::render_texture` → `Texture::download`.
//! The snapshotted widget must belong to a mapped window (an unmapped widget
//! yields no render node), so the host keeps a helper window.
//!
//! Pixel format: `gdk_texture_download` returns `CAIRO_FORMAT_ARGB32` —
//! premultiplied alpha, native endian (bytes B,G,R,A on little-endian).
//! The Vulkan layer must swizzle + blend accordingly
//! (`ONE, ONE_MINUS_SRC_ALPHA`).

use gdk4::prelude::{PaintableExt, TextureExt, TextureExtManual};
use gsk4::prelude::GskRendererExt;
use gtk4::prelude::{NativeExt, SnapshotExt, WidgetExt};

/// Canvas size hint. The helper window starts at this size; the snapshot
/// below always uses the widget's real allocation instead, so drawn pixels
/// match layout exactly even if content resizes the window. Frames must
/// stay within `CANVAS_MAX_W/H` (640×1024) for the layer SHM.
pub const CANVAS_W: f64 = 512.0;
pub const CANVAS_H: f64 = 800.0;

pub struct FrameRenderer {
    renderer: gsk4::Renderer,
}

impl FrameRenderer {
    /// Binds to a mapped window's surface. `for_surface` returns an
    /// already-realized renderer — do not realize it again.
    pub fn for_window(window: &gtk4::Window) -> Option<Self> {
        let surface = window.surface()?;
        let renderer = gsk4::Renderer::for_surface(&surface)?;
        Some(Self { renderer })
    }

    /// Renders `widget` to packed pixels. Returns (width, height, pixels).
    /// Snapshots at the widget's real allocation (not a fixed size): the
    /// paintable scales content to the requested size, so anything else
    /// would stretch drawn pixels away from layout and input coordinates.
    /// When `IRA_OVERLAY_UI_DEBUG_PNG` is set, frames are also saved there
    /// (with the frame number): every 60th, or every frame with
    /// `IRA_OVERLAY_UI_DEBUG_PNG_ALL` set (noisy, for visual debugging).
    pub fn render(&self, widget: &gtk4::Widget, frame_no: u64) -> Option<(u32, u32, Vec<u8>)> {
        let w = widget.width().max(1) as f64;
        let h = widget.height().max(1) as f64;
        let paintable = gtk4::WidgetPaintable::new(Some(widget));
        let snapshot = gtk4::Snapshot::new();
        paintable.snapshot(&snapshot, w, h);
        let node = snapshot.to_node()?;
        let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        let texture = self.renderer.render_texture(&node, Some(&bounds));
        if let Ok(path) = std::env::var("IRA_OVERLAY_UI_DEBUG_PNG")
            && (frame_no.is_multiple_of(60)
                || std::env::var_os("IRA_OVERLAY_UI_DEBUG_PNG_ALL").is_some())
        {
            let _ = texture.save_to_png(format!("{path}-{frame_no}.png"));
        }
        let width = texture.width();
        let height = texture.height();
        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        texture.download(&mut pixels, stride);
        Some((width as u32, height as u32, pixels))
    }
}
