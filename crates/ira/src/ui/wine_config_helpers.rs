use std::cell::RefCell;
use std::rc::Rc;

use super::css::*;
use adw::prelude::*;
use glib::clone::Downgrade;

pub(super) type OverrideList = Rc<RefCell<Vec<String>>>;

pub(super) fn build_combo_row(
    title: &str,
    options: &[(&str, &str)],
) -> (adw::ComboRow, gtk4::StringList) {
    let model = gtk4::StringList::new(&options.iter().map(|(l, _)| *l).collect::<Vec<_>>());
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_model(Some(&model));
    (row, model)
}

pub(super) fn build_switch_row(title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_active(active);
    row
}

pub(super) fn make_section(title: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    g
}

pub(super) fn make_page() -> gtk4::ScrolledWindow {
    let sw = gtk4::ScrolledWindow::new();
    sw.set_vexpand(true);
    sw.set_hexpand(true);
    sw
}

pub(super) fn page_with_content(content: gtk4::Box) -> gtk4::ScrolledWindow {
    let sw = make_page();
    sw.set_child(Some(&content));
    sw
}

pub(super) fn make_revert_btn() -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
    btn.add_css_class(CSS_FLAT);
    btn.set_valign(gtk4::Align::Center);
    btn.set_tooltip_text(Some(&crate::tr!("Revert to app default")));
    btn
}

/// Generates a `track_*` helper: the row's change signal marks the field
/// overridden and reveals the revert button; clicking the button applies
/// the default inside a `reverting` guard (so the change signal ignores
/// it), clears the override, and hides the button again.
macro_rules! track_impl {
    ($fn_name:ident, ($($w:ident: $wt:ty),+), $signal_w:ident.$signal:ident, $suffix_w:ident, $apply:ident, $val_ty:ty) => {
        pub(super) fn $fn_name(
            $($w: &$wt),+,
            field: &str,
            default_val: $val_ty,
            overridden: &OverrideList,
        ) {
            let revert_btn = make_revert_btn();
            revert_btn.set_visible(overridden.borrow().contains(&field.to_string()));
            $suffix_w.add_suffix(&revert_btn);
            let reverting = Rc::new(RefCell::new(false));

            let field_s = field.to_string();
            let ov = overridden.clone();
            let btn = revert_btn.clone();
            let rev = reverting.clone();
            $signal_w.$signal(move |_| {
                if *rev.borrow() {
                    return;
                }
                if !ov.borrow().contains(&field_s) {
                    ov.borrow_mut().push(field_s.clone());
                }
                btn.set_visible(true);
            });

            let field_s = field.to_string();
            let ov = overridden.clone();
            let $signal_w = Downgrade::downgrade($signal_w);
            let btn = Downgrade::downgrade(&revert_btn);
            revert_btn.connect_clicked(move |_| {
                let Some($signal_w) = $signal_w.upgrade() else {
                    return;
                };
                let Some(btn) = btn.upgrade() else {
                    return;
                };
                *reverting.borrow_mut() = true;
                $apply(&$signal_w, default_val);
                *reverting.borrow_mut() = false;
                ov.borrow_mut().retain(|f| f != &field_s);
                btn.set_visible(false);
            });
        }
    };
}

fn apply_switch(row: &adw::SwitchRow, v: bool) {
    row.set_active(v);
}

fn apply_spin_row_value(row: &adw::SpinRow, v: i32) {
    row.set_value(v as f64);
}

fn apply_combo_selected(row: &adw::ComboRow, v: u32) {
    row.set_selected(v);
}

fn apply_spin_button_value(spin: &gtk4::SpinButton, v: i32) {
    spin.set_value(v as f64);
}

track_impl!(
    track_switch,
    (row: adw::SwitchRow),
    row.connect_active_notify,
    row,
    apply_switch,
    bool
);
track_impl!(
    track_spin_row,
    (row: adw::SpinRow),
    row.connect_value_notify,
    row,
    apply_spin_row_value,
    i32
);
track_impl!(
    track_combo,
    (row: adw::ComboRow),
    row.connect_selected_notify,
    row,
    apply_combo_selected,
    u32
);
track_impl!(
    track_spin,
    (spin: gtk4::SpinButton, row: adw::ActionRow),
    spin.connect_value_changed,
    row,
    apply_spin_button_value,
    i32
);
