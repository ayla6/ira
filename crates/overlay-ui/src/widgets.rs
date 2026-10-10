//! Overlay widget builders: rows, prompts, cursor, and row input wiring.
//!
//! Pure construction plus the GTK controllers that forward row events to
//! the focus module. Game data snapshots come from `game.rs`; the shell
//! (sections, commands) lives in `ui.rs`.

use std::cell::RefCell;
use std::rc::Weak;

use glib::object::Cast as _;
use gtk4::prelude::{BoxExt, GestureExt, WidgetExt};
use ira_input::{ControllerFamily, GamepadButton, InputSource};
use ira_widgets::{glyphs, style as bp};

use super::ui::{OverlayUi, debug_log};

/// New scroll value keeping `row_y..row_y+row_h` inside the visible
/// `value..value+page` window: pulls up when the row hangs off the
/// bottom, down when it hangs off the top, still otherwise. Pure for
/// testability; the caller clamps to the adjustment range.
pub(super) fn scroll_target(row_y: f64, row_h: f64, value: f64, page: f64) -> f64 {
    if row_y < value {
        row_y
    } else if row_y + row_h > value + page {
        row_y + row_h - page
    } else {
        value
    }
}

/// Hint page: deliberately empty. Every section's name and progress
/// already sit in the sidebar; text here would only restate them.
pub(super) fn build_hint_panel() -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Vertical, 0)
}

/// Footer prompt: the control's glyph plus label, the big-picture
/// prompt pattern.
pub(super) fn build_prompt(
    button: GamepadButton,
    label: &str,
    family: ControllerFamily,
) -> gtk4::Box {
    let item = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    slot.set_size_request(32, 32);
    let glyph = gtk4::Image::new();
    glyph.set_pixel_size(30);
    glyph.set_halign(gtk4::Align::Center);
    glyph.set_valign(gtk4::Align::Center);
    slot.append(&glyph);
    let fallback = gtk4::Label::new(None);
    fallback.set_valign(gtk4::Align::Center);
    fallback.add_css_class(bp::CSS_BP_PROMPT);
    let source = InputSource::Button(button);
    let has_asset = glyphs::set_source_asset(&glyph, source, family);
    if !has_asset {
        fallback.set_text(&glyphs::source_badge(source, family));
    }
    fallback.set_visible(!has_asset);
    slot.append(&fallback);
    item.append(&slot);
    let text = gtk4::Label::new(Some(label));
    text.add_css_class(bp::CSS_BP_PROMPT);
    item.append(&text);
    item
}

pub(super) fn build_row(ach: &super::game::AchData) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.add_css_class(bp::CSS_BP_MENU_ROW);

    row.append(&icon_image(ach));

    // Everything centers on the text block: icon, lines and check share
    // one middle instead of hanging off the top.
    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_valign(gtk4::Align::Center);
    texts.set_hexpand(true);
    let title = gtk4::Label::new(Some(&ach.name));
    title.set_xalign(0.0);
    title.set_valign(gtk4::Align::Center);
    let desc = gtk4::Label::new(Some(&ach.desc));
    desc.set_xalign(0.0);
    desc.set_valign(gtk4::Align::Center);
    desc.set_wrap(true);
    desc.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    desc.add_css_class("dim-label");
    texts.append(&title);
    texts.append(&desc);

    row.append(&texts);
    row
}

pub(super) fn icon_image(ach: &super::game::AchData) -> gtk4::Image {
    // Synchronous decode through the shared image pipeline: the app's
    // async loader never drains inside this host (no pump on its path),
    // so async leaves placeholders up forever. Same decoder, same art.
    let path = if ach.earned {
        &ach.icon
    } else {
        &ach.icon_gray
    };
    if let Some((pixels, w, h)) = (!path.is_empty())
        .then(|| ira_parser::decode_to_rgba(std::path::Path::new(path)))
        .flatten()
    {
        let bytes = glib::Bytes::from_owned(pixels);
        let texture = gdk4::MemoryTexture::new(
            w as i32,
            h as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &bytes,
            (w * 4) as usize,
        );
        let img = gtk4::Image::from_paintable(Some(&texture));
        img.set_pixel_size(48);
        img.set_valign(gtk4::Align::Center);
        if !ach.earned {
            img.set_opacity(0.45);
        }
        return img;
    }
    let img = gtk4::Image::from_icon_name("trophy-symbolic");
    img.set_pixel_size(48);
    img.set_valign(gtk4::Align::Center);
    img
}

/// The theme cursor image for the composited pointer, with its hotspot.
/// Falls back to an empty (invisible) image when no cursor theme is
/// installed: a missing file must never break the overlay.
pub(super) fn cursor_image() -> (gtk4::Image, (f64, f64)) {
    let size = super::cursor::cursor_size();
    if let Some(img) = super::cursor::load_cursor("left_ptr", size) {
        let bytes = glib::Bytes::from_owned(img.pixels);
        let texture = gdk4::MemoryTexture::new(
            img.width as i32,
            img.height as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &bytes,
            (img.width * 4) as usize,
        );
        let hot = (f64::from(img.xhot), f64::from(img.yhot));
        return (gtk4::Image::from_paintable(Some(&texture)), hot);
    }
    (gtk4::Image::new(), (0.0, 0.0))
}

/// Removes every child widget. The overlay tracks its own rows instead of
/// asking a list widget, so rebuilds clear a plain box.
pub(super) fn clear_children(container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Forwards a row-relative controller event to window coordinates.
fn forward_row(
    weak: &Weak<RefCell<OverlayUi>>,
    row: &gtk4::Box,
    ex: f64,
    ey: f64,
    f: impl FnOnce(&mut OverlayUi, f64, f64),
) {
    if let Some(ui) = weak.upgrade() {
        let mut u = ui.borrow_mut();
        let widget = row.upcast_ref::<gtk4::Widget>();
        let (wx, wy) = u.to_window(widget, ex, ey);
        f(&mut u, wx, wy);
    }
}

/// Wires a row to the real pointer: motion hovers, press focuses, release
/// activates. Sidebar rows activate like gamepad A and can hide the
/// overlay; achievement rows are informational (Steam parity: press
/// focuses, release does nothing).
pub(super) fn watch_row(row: &gtk4::Box, weak: &Weak<RefCell<OverlayUi>>, activatable: bool) {
    let motion_row = row.clone();
    let hover = weak.clone();
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion(move |_, ex, ey| {
        forward_row(&hover, &motion_row, ex, ey, |u, wx, wy| {
            u.pointer_move(wx, wy)
        });
    });
    row.add_controller(motion);
    let press_row = row.clone();
    let press = weak.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |gesture, _, ex, ey| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        forward_row(&press, &press_row, ex, ey, |u, wx, wy| {
            u.pointer_press(wx, wy, 1)
        });
    });
    let release_row = row.clone();
    let release = weak.clone();
    click.connect_released(move |gesture, _, ex, ey| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        if let Some(ui) = release.upgrade() {
            let mut u = ui.borrow_mut();
            let widget = release_row.upcast_ref::<gtk4::Widget>();
            let (wx, wy) = u.to_window(widget, ex, ey);
            if activatable {
                if debug_log() {
                    eprintln!("ira-overlay-ui: pointer release at {wx:.0},{wy:.0}");
                }
                u.click_release(wx, wy);
            } else {
                u.pointer_release(wx, wy, 1);
            }
        }
    });
    row.add_controller(click);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_target_keeps_visible_rows_still() {
        // Row fully inside the window: no movement.
        assert_eq!(scroll_target(100.0, 60.0, 80.0, 400.0), 80.0);
        // Hanging off the bottom: pull up to its bottom edge.
        assert_eq!(scroll_target(500.0, 60.0, 80.0, 400.0), 160.0);
        // Hanging off the top: pull down to its top edge.
        assert_eq!(scroll_target(20.0, 60.0, 80.0, 400.0), 20.0);
    }
}
