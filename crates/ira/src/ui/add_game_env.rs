use adw::prelude::*;

pub(super) fn build_env_page() -> (gtk4::Box, gtk4::ListBox, adw::EntryRow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let (env_group, env_vars_box) = super::system_settings::build_env_vars_group(&[]);
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
