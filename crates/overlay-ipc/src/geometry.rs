//! Canvas panel geometry — pure functions, no SHM dependency: testable
//! without mappings, with different callers than `canvas.rs` (host sizes
//! its window, layer positions its quad).

use super::canvas::{CANVAS_MAX_H, CANVAS_MAX_W};
use crate::config::OverlayPosition;

/// Computes the on-screen panel rect (x, y, w, h) for a canvas frame.
/// Uniformly fit-scales the frame into the screen minus a 16px margin and
/// anchors it per `position` (`OverlayPosition` as u32; unknown = top-left).
pub fn canvas_panel_rect(
    screen_w: u32,
    screen_h: u32,
    frame_w: u32,
    frame_h: u32,
    position: u32,
) -> (f32, f32, f32, f32) {
    const MARGIN: f32 = 16.0;
    let fw = frame_w.max(1) as f32;
    let fh = frame_h.max(1) as f32;
    let avail_w = (screen_w as f32 - 2.0 * MARGIN).max(1.0);
    let avail_h = (screen_h as f32 - 2.0 * MARGIN).max(1.0);
    let scale = (avail_w / fw).min(avail_h / fh).min(1.0);
    let w = fw * scale;
    let h = fh * scale;
    let (x, y) = match OverlayPosition::from_u32(position) {
        OverlayPosition::TopRight => (screen_w as f32 - MARGIN - w, MARGIN),
        OverlayPosition::BottomLeft => (MARGIN, screen_h as f32 - MARGIN - h),
        OverlayPosition::BottomRight => (
            screen_w as f32 - MARGIN - w,
            screen_h as f32 - MARGIN - h,
        ),
        OverlayPosition::Center => ((screen_w as f32 - w) / 2.0, (screen_h as f32 - h) / 2.0),
        OverlayPosition::TopLeft => (MARGIN, MARGIN),
    };
    (x.max(0.0), y.max(0.0), w, h)
}

/// Window size for a screen, keeping the 512:800 panel aspect: fills the
/// screen minus a margin, clamped to the canvas maximum. The host applies
/// this to its window so the panel grows and shrinks with the game.
pub fn fit_panel_size(screen_w: u32, screen_h: u32) -> (u32, u32) {
    const BASE_W: f64 = 512.0;
    const BASE_H: f64 = 800.0;
    const MARGIN: f64 = 32.0;
    let avail_w = (screen_w as f64 - MARGIN).max(1.0);
    let avail_h = (screen_h as f64 - MARGIN).max(1.0);
    let scale = (avail_w / BASE_W).min(avail_h / BASE_H);
    let w = ((BASE_W * scale) as u32).clamp(1, CANVAS_MAX_W as u32);
    let h = ((BASE_H * scale) as u32).clamp(1, CANVAS_MAX_H as u32);
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_rect_anchors() {
        // 512×800 panel on 1920×1080: fits unscaled.
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 0),
            (16.0, 16.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 1),
            (1920.0 - 16.0 - 512.0, 16.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 3),
            (1920.0 - 16.0 - 512.0, 1080.0 - 16.0 - 800.0, 512.0, 800.0)
        );
        assert_eq!(
            canvas_panel_rect(1920, 1080, 512, 800, 4),
            ((1920.0 - 512.0) / 2.0, (1080.0 - 800.0) / 2.0, 512.0, 800.0)
        );
    }

    #[test]
    fn test_panel_rect_fit_scales_small_screen() {
        // 512×800 panel on 1280×720: scale = min(1248/512, 688/800) = 0.86.
        let (x, y, w, h) = canvas_panel_rect(1280, 720, 512, 800, 0);
        assert!((w - 512.0 * 0.86).abs() < 0.01);
        assert!((h - 800.0 * 0.86).abs() < 0.01);
        assert_eq!((x, y), (16.0, 16.0));
        assert!(x + w <= 1280.0 && y + h <= 720.0);
    }

    #[test]
    fn test_fit_panel_size_1080p() {
        // 1920×1080: scale = min(1888/512, 1048/800) = 1.31 → 670×1048.
        assert_eq!(fit_panel_size(1920, 1080), (670, 1048));
    }

    #[test]
    fn test_fit_panel_size_shrinks_small_screen() {
        // 800×600: scale = min(768/512, 568/800) = 0.71 → 363×568.
        assert_eq!(fit_panel_size(800, 600), (363, 568));
    }

    #[test]
    fn test_fit_panel_size_clamps_to_max() {
        // 4K would ask for 1362×2128: clamped to the canvas maximum.
        assert_eq!(
            fit_panel_size(3840, 2160),
            (CANVAS_MAX_W as u32, CANVAS_MAX_H as u32)
        );
    }

    #[test]
    fn test_fit_panel_size_keeps_aspect() {
        let (w, h) = fit_panel_size(1920, 1080);
        let ratio = w as f64 / h as f64;
        assert!((ratio - 512.0 / 800.0).abs() < 0.01);
    }

    #[test]
    fn test_panel_rect_handles_zero_and_unknown() {
        let (x, y, w, h) = canvas_panel_rect(0, 0, 0, 0, 99);
        for v in [x, y, w, h] {
            assert!(v.is_finite() && v >= 0.0);
        }
        assert!((w, h) != (0.0, 0.0));
        let (x, y, _, _) = canvas_panel_rect(1920, 1080, 512, 800, 99);
        assert_eq!((x, y), (16.0, 16.0));
    }
}
