//! Mouse cursor visibility tracking for Steam Input's "switch action set
//! when the cursor is shown/hidden" behavior.
//!
//! Games hide the OS cursor while playing and show it in menus/inventories;
//! XFixes reports the active cursor image, and a hidden cursor shows up as
//! fully transparent pixels (the empty-cursor trick every engine uses) or
//! as no image at all. Like the focus watcher this is X11-only: XWayland
//! games are covered, Wayland-native surfaces always report "visible" and
//! the feature degrades off.

use x11rb::protocol::xfixes::ConnectionExt as XFixesExt;

pub struct CursorWatcher {
    conn: x11rb::rust_connection::RustConnection,
}

impl CursorWatcher {
    /// None when there is no X server (or the XFixes handshake fails);
    /// callers treat that as "cursor always visible".
    pub fn create() -> Option<Self> {
        let (conn, _) = x11rb::rust_connection::RustConnection::connect(None).ok()?;
        conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
        Some(Self { conn })
    }

    /// Whether the pointer currently shows a visible cursor. `None` when
    /// the X server cannot be asked right now; callers keep the previous
    /// state instead of switching on unknowns.
    pub fn cursor_visible(&self) -> Option<bool> {
        let image = self
            .conn
            .xfixes_get_cursor_image()
            .ok()?
            .reply()
            .ok()?;
        Some(image_is_visible(image.width, &image.cursor_image))
    }
}

/// A hidden cursor is either no image at all or fully transparent pixels —
/// the empty-cursor trick every engine uses. A software-drawn in-game
/// cursor also reads as hidden; Steam Input switches on the OS cursor the
/// same way.
fn image_is_visible(width: u16, pixels: &[u32]) -> bool {
    width > 0 && pixels.iter().any(|pixel| pixel >> 24 != 0)
}

#[cfg(test)]
mod tests {
    use super::image_is_visible;

    #[test]
    fn test_image_visibility_uses_the_alpha_channel() {
        assert!(image_is_visible(2, &[0xFF000000, 0x00FFFFFF]));
        assert!(!image_is_visible(2, &[0x00FFFFFF, 0x00111111]));
        assert!(!image_is_visible(1, &[]));
        assert!(!image_is_visible(0, &[0xFF000000]));
    }
}
