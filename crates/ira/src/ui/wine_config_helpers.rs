use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use super::css::*;

pub(super) type OverrideList = Rc<RefCell<Vec<String>>>;

pub(super) fn build_combo_row(title: &str, options: &[(&str, &str)]) -> (adw::ComboRow, gtk4::StringList) {
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
    btn.set_tooltip_text(Some("Revert to app default"));
    btn
}

pub(super) fn track_switch(row: &adw::SwitchRow, field: &str, default_val: bool, overridden: &OverrideList) {
    let is_overridden = overridden.borrow().contains(&field.to_string());
    let revert_btn = make_revert_btn();
    revert_btn.set_visible(is_overridden);
    row.add_suffix(&revert_btn);

    let reverting = Rc::new(RefCell::new(false));

    let field_s = field.to_string();
    let ov = overridden.clone();
    let btn = revert_btn.clone();
    let rev = reverting.clone();
    row.connect_active_notify(move |_| {
        if *rev.borrow() { return; }
        if !ov.borrow().contains(&field_s) {
            ov.borrow_mut().push(field_s.clone());
        }
        btn.set_visible(true);
    });

    let field_s2 = field.to_string();
    let ov2 = overridden.clone();
    let row2 = row.clone();
    let btn2 = revert_btn.clone();
    let rev2 = reverting.clone();
    revert_btn.connect_clicked(move |_| {
        *rev2.borrow_mut() = true;
        row2.set_active(default_val);
        *rev2.borrow_mut() = false;
        ov2.borrow_mut().retain(|f| f != &field_s2);
        btn2.set_visible(false);
    });
}

pub(super) fn track_spin(spin: &gtk4::SpinButton, row: &adw::ActionRow, field: &str, default_val: i32, overridden: &OverrideList) {
    let is_overridden = overridden.borrow().contains(&field.to_string());
    let revert_btn = make_revert_btn();
    revert_btn.set_visible(is_overridden);
    row.add_suffix(&revert_btn);

    let reverting = Rc::new(RefCell::new(false));

    let field_s = field.to_string();
    let ov = overridden.clone();
    let btn = revert_btn.clone();
    let rev = reverting.clone();
    spin.connect_value_changed(move |_| {
        if *rev.borrow() { return; }
        if !ov.borrow().contains(&field_s) {
            ov.borrow_mut().push(field_s.clone());
        }
        btn.set_visible(true);
    });

    let field_s2 = field.to_string();
    let ov2 = overridden.clone();
    let spin2 = spin.clone();
    let btn2 = revert_btn.clone();
    let rev2 = reverting.clone();
    revert_btn.connect_clicked(move |_| {
        *rev2.borrow_mut() = true;
        spin2.set_value(default_val as f64);
        *rev2.borrow_mut() = false;
        ov2.borrow_mut().retain(|f| f != &field_s2);
        btn2.set_visible(false);
    });
}
