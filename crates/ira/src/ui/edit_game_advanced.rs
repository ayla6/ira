use adw::prelude::*;
use ira_models::GameLaunchConfig;
use super::add_game_dialog::build_env_var_row;
use super::settings_dialog;
use super::wine_config_env_dll::build_dll_override_row;
use super::wine_config_widget::WineConfigWidgets;
use super::css::*;

#[derive(Clone)]
pub(super) struct AdvancedWidgets {
    pub env_vars_box: gtk4::ListBox,
    pub dll_overrides_box: Option<gtk4::ListBox>,
    pub ld_preload_entry: adw::EntryRow,
    pub ld_library_path_entry: adw::EntryRow,
}

pub(super) fn build_advanced_page(
    launch: &GameLaunchConfig,
    wine_data: Option<&[(String, String)]>,       // wine env vars
    wine_dll_data: Option<&[(String, String)]>,    // dll overrides
    ww_opt: Option<&WineConfigWidgets>,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) -> AdvancedWidgets {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    if let Some(ww) = ww_opt {
        let ac_group = adw::PreferencesGroup::new();
        ac_group.set_title("Anti-Cheat");
        ac_group.add(&ww.battleye);
        ac_group.add(&ww.eac);
        ac_group.add(&ww.desktop_integration);
        page.append(&ac_group);

        let dbg_group = adw::PreferencesGroup::new();
        dbg_group.set_title("Debugging");
        dbg_group.add(&ww.show_debug);
        dbg_group.add(&ww.show_crash_dialogs);
        page.append(&dbg_group);
    }

    // Environment Variables section
    let env_group = adw::PreferencesGroup::new();
    env_group.set_title("Environment variables");
    let add_env_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_env_btn.set_tooltip_text(Some("Add variable"));
    add_env_btn.set_valign(gtk4::Align::Center);
    add_env_btn.add_css_class(CSS_FLAT);
    env_group.set_header_suffix(Some(&add_env_btn));

    let env_vars_box = gtk4::ListBox::new();
    env_vars_box.add_css_class(CSS_BOXED_LIST);

    if let Some(data) = wine_data {
        for (name, value) in data {
            env_vars_box.append(&build_env_var_row(name, value));
        }
    }
    for (name, value) in &launch.env_vars {
        env_vars_box.append(&build_env_var_row(name, value));
    }

    let env_box_clone = env_vars_box.clone();
    add_env_btn.connect_clicked(move |_| {
        env_box_clone.append(&build_env_var_row("", ""));
    });

    env_group.add(&env_vars_box);
    page.append(&env_group);

    // DLL Overrides section (wine only)
    let dll_overrides_box = if let Some(data) = wine_dll_data {
        let dll_group = adw::PreferencesGroup::new();
        dll_group.set_title("DLL overrides");
        let add_dll_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        add_dll_btn.set_tooltip_text(Some("Add override"));
        add_dll_btn.set_valign(gtk4::Align::Center);
        add_dll_btn.add_css_class(CSS_FLAT);
        dll_group.set_header_suffix(Some(&add_dll_btn));

        let dob = gtk4::ListBox::new();
        dob.add_css_class(CSS_BOXED_LIST);
        for (name, value) in data {
            dob.append(&build_dll_override_row(name, value));
        }

        let box_clone = dob.clone();
        add_dll_btn.connect_clicked(move |_| {
            box_clone.append(&build_dll_override_row("", "native,builtin"));
        });

        dll_group.add(&dob);
        page.append(&dll_group);
        Some(dob)
    } else {
        None
    };

    // LD_PRELOAD and LD_LIBRARY_PATH (flat, no expander)
    let ld_group = adw::PreferencesGroup::new();
    let ld_preload_entry = adw::EntryRow::new();
    ld_preload_entry.set_title("LD_PRELOAD");
    ld_preload_entry.set_text(&launch.ld_preload);
    ld_group.add(&ld_preload_entry);
    let ld_library_path_entry = adw::EntryRow::new();
    ld_library_path_entry.set_title("LD_LIBRARY_PATH");
    ld_library_path_entry.set_text(&launch.ld_library_path);
    ld_group.add(&ld_library_path_entry);
    page.append(&ld_group);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    sidebar.append(&settings_dialog::sidebar_separator());
    sidebar.append(&settings_dialog::settings_sidebar_row("preferences-other-symbolic", "Advanced", "advanced"));
    stack.add_named(&scroll, Some("advanced"));

    AdvancedWidgets {
        env_vars_box,
        dll_overrides_box,
        ld_preload_entry,
        ld_library_path_entry,
    }
}
