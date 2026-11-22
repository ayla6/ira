//! Cap-band centering for icons inside widgets Ira does not build — the
//! tab picker's toggles, whose icon and label live in libadwaita's own
//! box — by nudging the icon's margins after layout. Every row Ira builds
//! itself is an `IconRow`, which places its icons during allocation.

use gtk4::prelude::*;

/// Frames a centering pass may take to settle.
const SETTLE_FRAMES: u32 = 12;

/// Frames from a margin change until its layout can be measured: the
/// first tick after a change still reports the old allocation.
const LAYOUT_LAG: u32 = 2;

/// Center `icon` on `label`'s cap band once both are laid out, and again
/// whenever this is called after a size or text change. The icon is
/// switched to centered alignment; its vertical margins become the shift.
pub(super) fn center_on_text(icon: &impl IsA<gtk4::Widget>, label: &gtk4::Label) {
    let icon = icon.as_ref().clone();
    icon.set_valign(gtk4::Align::Center);
    let label = label.clone();
    let frames = std::cell::Cell::new(0u32);
    let measure_at = std::cell::Cell::new(LAYOUT_LAG);
    icon.clone().add_tick_callback(move |_, _| {
        frames.set(frames.get() + 1);
        if frames.get() > SETTLE_FRAMES {
            return glib::ControlFlow::Break;
        }
        if frames.get() < measure_at.get() {
            return glib::ControlFlow::Continue;
        }
        // Not laid out yet (a page mapping late): try again next frame.
        let Some(offset) = cap_band_offset(&icon, &label) else {
            return glib::ControlFlow::Continue;
        };
        let (top, bottom) = shifted_margins(icon.margin_top(), icon.margin_bottom(), offset);
        if (top, bottom) == (icon.margin_top(), icon.margin_bottom()) {
            return glib::ControlFlow::Break;
        }
        icon.set_margin_top(top);
        icon.set_margin_bottom(bottom);
        measure_at.set(frames.get() + LAYOUT_LAG);
        glib::ControlFlow::Continue
    });
}

/// How far the cap band's center lies below the icon's center, in
/// pixels. `None` before either widget has a size.
fn cap_band_offset(icon: &gtk4::Widget, label: &gtk4::Label) -> Option<f32> {
    if label.height() == 0 || icon.height() == 0 {
        return None;
    }
    let bounds = icon.compute_bounds(label)?;
    let cap_center = super::icon_row::cap_line(label, label.upcast_ref())?;
    Some(cap_center - (bounds.y() + bounds.height() / 2.0))
}

/// Margins moving a vertically centered widget down by `offset` pixels.
/// A centered widget moves by half its top/bottom margin difference, so
/// the difference changes by twice the offset; only one side is ever
/// non-zero, keeping the widget's footprint as small as possible.
fn shifted_margins(top: i32, bottom: i32, offset: f32) -> (i32, i32) {
    let net = top - bottom + (offset * 2.0).round() as i32;
    (net.max(0), (-net).max(0))
}

#[cfg(test)]
mod tests {
    use super::shifted_margins;

    #[test]
    fn test_shifted_margins_moves_down_through_top_margin() {
        assert_eq!(shifted_margins(0, 0, 1.5), (3, 0));
    }

    #[test]
    fn test_shifted_margins_moves_up_through_bottom_margin() {
        assert_eq!(shifted_margins(0, 0, -2.0), (0, 4));
        assert_eq!(shifted_margins(2, 0, -2.0), (0, 2));
    }

    #[test]
    fn test_shifted_margins_settled_keeps_margins() {
        assert_eq!(shifted_margins(3, 0, 0.2), (3, 0));
        assert_eq!(shifted_margins(0, 0, 0.0), (0, 0));
    }
}
