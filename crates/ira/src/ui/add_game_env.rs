use gtk4::prelude::*;
use adw::prelude::*;
use super::css::*;

pub(super) fn build_env_page() -> (gtk4::Box, gtk4::ListBox, adw::EntryRow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let env_group = adw::PreferencesGroup::new();
    env_group.set_title("Environment Variables");

    let env_vars_box = gtk4::ListBox::new();
    env_vars_box.add_css_class(CSS_BOXED_LIST);

    let add_env_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_env_btn.set_tooltip_text(Some("Add variable"));
    add_env_btn.set_valign(gtk4::Align::Center);
    add_env_btn.add_css_class(CSS_FLAT);
    let env_box_clone = env_vars_box.clone();
    add_env_btn.connect_clicked(move |_| {
        env_box_clone.append(&super::add_game_dialog::build_env_var_row("", ""));
    });
    env_group.set_header_suffix(Some(&add_env_btn));
    env_group.add(&env_vars_box);
    page.append(&env_group);

    let expander = adw::ExpanderRow::new();
    expander.set_title("Advanced");
    expander.set_expanded(false);

    let ld_preload_entry = adw::EntryRow::new();
    ld_preload_entry.set_title("LD_PRELOAD");
    expander.add_row(&ld_preload_entry);

    let ld_library_entry = adw::EntryRow::new();
    ld_library_entry.set_title("LD_LIBRARY_PATH");
    expander.add_row(&ld_library_entry);

    let ld_group = adw::PreferencesGroup::new();
    ld_group.add(&expander);
    page.append(&ld_group);

    (page, env_vars_box, ld_preload_entry, ld_library_entry)
}
