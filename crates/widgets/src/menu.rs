//! The one big-picture menu construction both hosts share: section
//! headers and menu rows. The app and the overlay call these, so a row is
//! a row no matter which surface renders it — never two copies drifting.

use gtk4::prelude::{BoxExt, WidgetExt};

/// Section header (game name, "Sort by"): the padded menu-title class, so
/// its text starts exactly where row text starts (both carry the same
/// side padding). Pixel-snapped like every other menu label.
pub fn menu_header(text: &str) -> gtk4::Label {
    let header = gtk4::Label::new(Some(text));
    header.set_xalign(0.0);
    header.set_wrap(true);
    header.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    header.add_css_class(crate::style::CSS_BP_MENU_TITLE);
    crisp(&header);
    header
}

/// One menu row: plain box with a plain wrapping label, exactly what big
/// picture's own menus build. Selection state stays with the caller (ring
/// plus accent classes); geometry never does.
///
/// The label carries no text class, so row text renders full-bright like
/// BP — only headers dim.
pub fn menu_row(text: &str) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.add_css_class(crate::style::CSS_BP_MENU_ROW);
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    crisp(&label);
    row.append(&label);
    row
}

/// Pixel-snapped glyph positions: halves land on whole pixels instead of
/// blurring across two.
pub fn crisp(label: &gtk4::Label) {
    label.pango_context().set_round_glyph_positions(true);
}
