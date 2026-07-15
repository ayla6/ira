use adw::prelude::*;
use ira_models::GameLaunchConfig;
use super::add_game_dialog::build_env_var_row;
use super::helpers;
use super::settings_dialog;

pub(super) struct LaunchConfigWidgets {
    pub(super) exe_entry: adw::EntryRow,
    pub(super) args_entry: adw::EntryRow,
    pub(super) wd_entry: adw::EntryRow,
    pub(super) env_vars_box: gtk4::ListBox,
    pub(super) ld_preload_entry: adw::EntryRow,
    pub(super) ld_path_entry: adw::EntryRow,
}

pub(super) fn build_launch_config_page(
    launch: &GameLaunchConfig,
    win: &adw::Window,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    show_advanced: bool,
) -> Option<LaunchConfigWidgets> {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let lc_group = adw::PreferencesGroup::new();
    lc_group.set_title("Executable");

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable path");
    exe_entry.set_text(&launch.exe);

    let exe_browse = helpers::make_browse_button(
        Some(win),
        "Select executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&exe_browse);
    lc_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    args_entry.set_text(&launch.args);
    lc_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");
    wd_entry.set_text(&launch.working_dir);

    let wd_browse = helpers::make_browse_button(
        Some(win),
        "Select working directory",
        true,
        None,
        {
            let entry = wd_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    wd_entry.add_suffix(&wd_browse);
    lc_group.add(&wd_entry);

    page.append(&lc_group);
    sidebar.append(&settings_dialog::settings_sidebar_row("preferences-other-symbolic", "Launch Config"));
    stack.add_named(&page, Some("launch"));

    let env_vars_box;
    let ld_preload_entry;
    let ld_path_entry;

    if show_advanced {
        let native_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

        let env_group = adw::PreferencesGroup::new();
        env_group.set_title("Environment Variables");
        env_vars_box = gtk4::ListBox::new();
        env_vars_box.add_css_class("boxed-list");
        for (k, v) in &launch.env_vars {
            let row = build_env_var_row(k, v);
            env_vars_box.append(&row);
        }
        env_group.add(&env_vars_box);

        let add_env_btn = gtk4::Button::with_label("Add variable");
        add_env_btn.add_css_class("flat");
        let env_box_c = env_vars_box.clone();
        add_env_btn.connect_clicked(move |_| {
            env_box_c.append(&build_env_var_row("", ""));
        });
        env_group.add(&add_env_btn);
        native_page.append(&env_group);

        let expander = adw::ExpanderRow::new();
        expander.set_title("Advanced");
        expander.set_expanded(!launch.ld_preload.is_empty() || !launch.ld_library_path.is_empty());

        ld_preload_entry = adw::EntryRow::new();
        ld_preload_entry.set_title("LD_PRELOAD");
        ld_preload_entry.set_text(&launch.ld_preload);
        expander.add_row(&ld_preload_entry);

        ld_path_entry = adw::EntryRow::new();
        ld_path_entry.set_title("LD_LIBRARY_PATH");
        ld_path_entry.set_text(&launch.ld_library_path);
        expander.add_row(&ld_path_entry);

        let ld_group = adw::PreferencesGroup::new();
        ld_group.add(&expander);
        native_page.append(&ld_group);

        sidebar.append(&settings_dialog::settings_sidebar_row("preferences-other-symbolic", "Advanced"));
        stack.add_named(&native_page, Some("advanced"));
    } else {
        env_vars_box = gtk4::ListBox::new();
        ld_preload_entry = adw::EntryRow::new();
        ld_path_entry = adw::EntryRow::new();
    }

    Some(LaunchConfigWidgets {
        exe_entry,
        args_entry,
        wd_entry,
        env_vars_box,
        ld_preload_entry,
        ld_path_entry,
    })
}
