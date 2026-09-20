//! Steam-style building blocks for the input editor pages: titled setting
//! groups rendered as boxed lists, full-width slider rows with a live value
//! label, and small row factories shared by the setting sheets.

use super::css::{CSS_BOXED_LIST, CSS_DIM_LABEL, CSS_HEADING};
use adw::prelude::*;

/// A titled group of setting rows drawn as one boxed list. Equivalent to
/// `adw::PreferencesGroup`, but it also accepts custom rows such as sliders.
#[derive(Clone)]
pub(crate) struct SettingGroup {
    pub root: gtk4::Box,
    list: gtk4::ListBox,
}

impl SettingGroup {
    pub(crate) fn new(title: Option<&str>, description: Option<&str>) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        if let Some(title) = title {
            let label = gtk4::Label::new(Some(title));
            label.set_xalign(0.0);
            label.add_css_class(CSS_HEADING);
            root.append(&label);
        }
        if let Some(description) = description {
            let label = gtk4::Label::new(Some(description));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class(CSS_DIM_LABEL);
            root.append(&label);
        }
        let list = gtk4::ListBox::new();
        list.add_css_class(CSS_BOXED_LIST);
        list.set_selection_mode(gtk4::SelectionMode::None);
        root.append(&list);
        Self { root, list }
    }

    pub(crate) fn add(&self, row: &impl IsA<gtk4::Widget>) {
        self.list.append(row);
    }

    pub(crate) fn remove(&self, row: &impl IsA<gtk4::Widget>) {
        self.list.remove(row);
    }
}

/// min / max / step / value for one slider.
pub(crate) struct SliderSpec(pub f64, pub f64, pub f64, pub f64);

/// One full-width slider row: the title with the live value on the right,
/// the scale underneath, and an optional description — the Steam Input
/// slider layout.
pub(crate) fn slider_row(
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    on_change: impl Fn(f64) + 'static,
) -> gtk4::ListBoxRow {
    slider_row_with_scale(title, subtitle, spec, on_change).0
}

/// [`slider_row`] plus the raw scale, for callers that need to move the
/// value from outside (e.g. when the edited device changes).
pub(crate) fn slider_row_with_scale(
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    on_change: impl Fn(f64) + 'static,
) -> (gtk4::ListBoxRow, gtk4::Scale) {
    let SliderSpec(min, max, step, value) = *spec;
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // The value doubles as a manual entry: drag for feel, type for
    // precision — one control instead of a read-only label.
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let title_label = gtk4::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_hexpand(true);
    let entry = gtk4::SpinButton::with_range(min, max, step);
    entry.set_value(value);
    entry.set_digits(step_digits(step).max(0) as u32);
    entry.set_valign(gtk4::Align::Center);
    entry.set_width_chars(7);
    header.append(&title_label);
    header.append(&entry);

    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    scale.set_draw_value(false);
    scale.set_round_digits(step_digits(step));
    scale.set_value(value);

    content.append(&header);
    content.append(&scale);
    if let Some(subtitle) = subtitle {
        let subtitle_label = gtk4::Label::new(Some(subtitle));
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_wrap(true);
        subtitle_label.add_css_class(CSS_DIM_LABEL);
        content.append(&subtitle_label);
    }
    row.set_child(Some(&content));

    // The scale's signal is the single source of change reports; typing
    // drives the scale, so the two never feed back into each other.
    let entry_for_scale = entry.clone();
    scale.connect_value_changed(move |scale| {
        entry_for_scale.set_value(scale.value());
        on_change(scale.value());
    });
    let scale_for_entry = scale.clone();
    entry.connect_value_changed(move |entry| {
        if (entry.value() - scale_for_entry.value()).abs() > f64::EPSILON {
            scale_for_entry.set_value(entry.value());
        }
    });
    (row, scale)
}

/// [`slider_row`] with a manual entry: the header's value is an editable
/// spin button synced both ways with the scale, for precise values the
/// slider cannot comfortably reach.
pub(crate) fn slider_entry_row(
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    on_change: impl Fn(f64) + 'static,
) -> gtk4::ListBoxRow {
    slider_row(title, subtitle, spec, on_change)
}

/// One choice in a described option list: a title plus an optional
/// explanation. Combo rows show the title in the popup and the description
/// as the row's subtitle.
#[derive(Clone)]
pub(crate) struct OptionChoice {
    pub title: String,
    pub description: Option<String>,
}

/// A libadwaita switch row wired straight to a config field.
pub(crate) fn switch_row(
    title: &str,
    subtitle: Option<&str>,
    active: bool,
    on_change: impl Fn(bool) + 'static,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .active(active)
        .build();
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.connect_active_notify(move |row| on_change(row.is_active()));
    row
}

/// "42 %" — slider format for 0..1 fractions.
/// A section heading inside an expander child list: dim, small, not
/// selectable — the hierarchy marker for flattened settings.
pub(crate) fn section_title_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("heading");
    label.add_css_class(CSS_DIM_LABEL);
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}

/// Number of decimal places in a slider step increment.
pub(crate) fn step_digits(step: f64) -> i32 {
    let mut digits = 0i32;
    let mut scaled = step;
    while scaled.round() != scaled && digits < 6 {
        scaled *= 10.0;
        digits += 1;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::step_digits;

    #[test]
    fn test_step_digits_counts_decimals() {
        assert_eq!(step_digits(1.0), 0);
        assert_eq!(step_digits(0.05), 2);
        assert_eq!(step_digits(0.01), 2);
        assert_eq!(step_digits(10.0), 0);
    }
}
