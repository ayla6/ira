use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use adw::prelude::*;
use crate::models::{GameLaunchConfig, WineConfig};
use crate::AppMessage;
use super::state::SharedState;
use super::add_game_dialog::{build_env_var_row, collect_env_vars};

pub fn show_edit_game_dialog(state: &SharedState, db_id: i64) {
    let (game, config, app_default_wine) = {
        let s = state.borrow();
        let game = s.games.iter().find(|g| g.db_id == db_id).cloned();
        let config = crate::db::get_game_config(&s.db, db_id).ok().flatten();
        let app_default_wine = s.cfg.default_wine_config.clone();
        (game, config, app_default_wine)
    };
    let Some(game) = game else { return };
    let is_native_platform = game.kind == "steam" || game.kind == "ps4";
    let has_config = config.is_some();
    let (saved_launch, mut saved_wine, saved_profile_id) = config.clone().unwrap_or_default();

    if !has_config {
        saved_wine = WineConfig::default();
    } else {
        saved_wine = saved_wine.merge_with_default(&app_default_wine);
    }

    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_default_width(720);
    win.set_default_height(540);
    win.set_transient_for(Some(&parent));
    win.set_modal(true);
    win.set_deletable(false);
    let save_dir = state.borrow().save_dir.clone();
    let app_details = crate::parser::read_app_details(&save_dir, &game.app_id);

    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let sidebar_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_area.add_css_class("settings-sidebar");
    sidebar_area.set_size_request(200, -1);
    sidebar_area.set_vexpand(true);
    let sidebar = gtk4::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_margin_top(6);
    sidebar.set_margin_bottom(6);
    sidebar_area.append(&sidebar);

    let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    outer.append(&sidebar_area);
    outer.append(&sep);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.set_hexpand(true);

    let header = adw::HeaderBar::new();
    header.add_css_class("settings-header");
    header.set_title_widget(Some(&gtk4::Label::new(Some(&game.name))));
    content_area.append(&header);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_margin_start(16);
    stack.set_margin_end(16);
    stack.set_margin_top(16);
    stack.set_margin_bottom(16);
    stack.set_hexpand(true);

    // --- General page ---
    let languages = app_details.as_ref().map(|d| d.languages.clone()).unwrap_or_default();
    let (general_page, title_entry, sort_entry, pending_version, app_id_entry, language_row) =
        super::dialogs::build_game_general_page(state, &game, &win, &languages);
    sidebar.append(&super::dialogs::settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let is_lutris_unmanaged = !game.lutris_name.is_empty() && !has_config;
    if is_lutris_unmanaged {
        let convert_group = adw::PreferencesGroup::new();
        let convert_btn = gtk4::Button::with_label("Convert to managed game");
        convert_btn.add_css_class("suggested-action");
        convert_group.add(&convert_btn);
        general_page.append(&convert_group);

        let state_c = state.clone();
        let db_id_c = db_id;
        let lutris_id_c = game.lutris_id;
        let game_name_c = game.name.clone();
        let win_c = win.clone();
        convert_btn.connect_clicked(move |_| {
            let alert = adw::AlertDialog::new(
                Some("Convert to managed game"),
                Some("This will read the game's Lutris configuration and create a managed game config."),
            );
            alert.add_response("cancel", "Cancel");
            alert.add_response("convert", "Convert");
            alert.set_response_appearance("convert", adw::ResponseAppearance::Suggested);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");

            let sc = state_c.clone();
            let db_id = db_id_c;
            let lutris_id = lutris_id_c;
            let w_close = win_c.clone();
            let game_name = game_name_c.clone();
            alert.connect_response(None, move |_, response| {
                if response == "convert" {
                    let db = sc.borrow().db.clone();
                    let sender = sc.borrow().sender.clone();
                    let gn = game_name.clone();
                    w_close.close();
                    std::thread::spawn(move || {
                        match crate::platforms::lutris_config::read_lutris_game_config(lutris_id) {
                            Ok((_runner, _directory, config)) => {
                                let launch = GameLaunchConfig {
                                    exe: config.game.exe.clone(),
                                    args: config.game.args.clone(),
                                    working_dir: config.game.working_dir.clone(),
                                    env_vars: config.system.env.iter().map(|(k,v)| (k.clone(), v.clone())).collect(),
                                    ..Default::default()
                                };
                                let wine = WineConfig {
                                    enabled: true,
                                    prefix: config.game.prefix.clone(),
                                    version: if config.wine.version.is_empty() { "system".to_string() } else { config.wine.version.clone() },
                                    arch: if config.game.arch.is_empty() { "auto".to_string() } else { config.game.arch.clone() },
                                    esync: config.wine.esync,
                                    fsync: config.wine.fsync,
                                    dxvk: config.wine.dxvk,
                                    vkd3d: config.wine.vkd3d,
                                    d3d_extras: config.wine.d3d_extras,
                                    dxvk_nvapi: config.wine.dxvk_nvapi,
                                    fsr: config.wine.fsr,
                                    battleye: config.wine.battleye,
                                    eac: config.wine.eac,
                                    show_debug: if config.wine.show_debug.is_empty() { "-all".to_string() } else { config.wine.show_debug.clone() },
                                    dll_overrides: config.wine.overrides.iter().map(|(k,v)| (k.clone(), v.clone())).collect(),
                                    gamemode: config.system.gamemode,
                                    mangohud: config.system.mangohud,
                                    gamescope: config.system.gamescope,
                                    gamescope_flags: config.system.gamescope_flags.clone(),
                                    ..Default::default()
                                };
                                let profile_id = {
                                    let profiles = crate::db::get_all_profiles(&db).unwrap_or_default();
                                    let existing = profiles.iter().find(|p| p.prefix == wine.prefix && p.wine_version == wine.version);
                                    if let Some(p) = existing {
                                        Some(p.id)
                                    } else {
                                        let profile_name = format!("{} ({})", gn, wine.version);
                                        crate::db::add_profile(&db, &profile_name, &wine.version, &wine.custom_wine_path, &wine.prefix, &wine.arch).ok()
                                    }
                                };
                                let _ = crate::db::save_game_config(&db, db_id, &launch, &wine, profile_id);
                            }
                            Err(e) => {
                                let _ = sender.send(AppMessage::AddGameError(e));
                            }
                        }
                    });
                }
            });
            alert.present(Some(&win_c));
        });
    }

    // --- Profile dropdown (only when wine config exists and wine is enabled) ---
    let profiles = crate::db::get_all_profiles(&state.borrow().db).unwrap_or_default();
    let profile_row: Option<adw::ComboRow> = if has_config && saved_wine.enabled {
        let profile_labels: Vec<String> = std::iter::once("Custom (per-game)".to_string())
            .chain(profiles.iter().map(|p| p.name.clone()))
            .collect();
        let str_refs: Vec<&str> = profile_labels.iter().map(|s| s.as_str()).collect();
        let profile_model = gtk4::StringList::new(&str_refs);
        let pr = adw::ComboRow::new();
        pr.set_title("Wine Profile");
        pr.set_subtitle("Links wine version + prefix together");
        pr.set_model(Some(&profile_model));
        if let Some(pid) = saved_profile_id {
            for (i, p) in profiles.iter().enumerate() {
                if p.id == pid {
                    pr.set_selected((i + 1) as u32);
                    break;
                }
            }
        }
        let profile_group = adw::PreferencesGroup::new();
        profile_group.add(&pr);
        general_page.prepend(&profile_group);
        Some(pr)
    } else {
        None
    };

    // --- Launch Config + Wine Config (not for steam/ps4; lutris only if it has a saved config) ---
    let show_launch_config = !is_native_platform && (game.kind != "lutris" || has_config);
    let launch_config_widgets = if show_launch_config {
        build_launch_config_page(&saved_launch, &win, &sidebar, &stack, !saved_wine.enabled)
    } else {
        None
    };

    if saved_wine.enabled && show_launch_config {
        sidebar.append(&super::dialogs::sidebar_separator());
    }

    let wine_widgets_opt = if saved_wine.enabled && show_launch_config {
        let (wine_pages, ww) = crate::ui::wine_config_widget::build_wine_config_pages(&saved_wine, Some(&app_default_wine));
        for wp in &wine_pages {
            sidebar.append(&super::dialogs::settings_sidebar_row(wp.icon, wp.label));
            stack.add_named(&wp.page, Some(wp.label));
        }
        Some(ww)
    } else {
        None
    };

    if saved_wine.enabled && show_launch_config {
        sidebar.append(&super::dialogs::sidebar_separator());
    }

    // --- Images page ---
    let pending_copies: Rc<RefCell<HashMap<String, String>>> = Default::default();
    if !game.app_id.is_empty() {
        let images_page = super::dialogs::build_image_manager_content_with_drafts(
            state, &game, &win, Some(pending_copies.clone()),
        );
        sidebar.append(&super::dialogs::settings_sidebar_row("image-x-generic-symbolic", "Images"));
        stack.add_named(&images_page, Some("images"));
    }

    // --- Logo page ---
    let logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)> =
        if let Some((logo_page, selected_pos, size_adj)) = super::dialogs::build_game_logo_page(&game) {
            sidebar.append(&super::dialogs::settings_sidebar_row("preferences-desktop-wallpaper-symbolic", "Logo"));
            stack.add_named(&logo_page, Some("logo"));
            Some((selected_pos, size_adj))
        } else {
            None
        };

    // --- DLC page ---
    let dlc_switches: Vec<adw::SwitchRow> = if let Some(ref details) = app_details {
        if !details.dlcs.is_empty() {
            let dlc_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            let dlc_group = adw::PreferencesGroup::new();
            dlc_group.set_title(&format!("DLCs  ·  {}", details.dlcs.len()));

            let mut dlc_list: Vec<(String, crate::api::types::DlcInfo)> = details.dlcs.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            dlc_list.sort_by_key(|(_, d)| d.app_id);

            let mut switches: Vec<adw::SwitchRow> = Vec::new();
            for (_, dlc) in &dlc_list {
                let row = adw::SwitchRow::new();
                row.set_title(&super::helpers::esc(&dlc.name));
                row.set_subtitle(&format!("App ID: {}", dlc.app_id));
                row.set_active(dlc.enabled);
                dlc_group.add(&row);
                switches.push(row);
            }
            dlc_page.append(&dlc_group);

            let dlc_scroll = gtk4::ScrolledWindow::new();
            dlc_scroll.set_child(Some(&dlc_page));
            dlc_scroll.set_vexpand(true);
            dlc_scroll.set_hexpand(true);

            sidebar.append(&super::dialogs::settings_sidebar_row("package-x-generic-symbolic", "DLC"));
            stack.add_named(&dlc_scroll, Some("dlc"));
            switches
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // --- API Emulator page ---
    let emu_exe = saved_launch.exe.clone();
    let emu_trophy_source = game.trophy_source.clone();
    let emu_app_id = game.app_id.clone();
    let emu_save_dir = state.borrow().save_dir.clone();
    if (emu_trophy_source == crate::models::GSE || emu_trophy_source == crate::models::NGE) && !emu_exe.is_empty() {
        let emu_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        emu_page.set_margin_start(12);
        emu_page.set_margin_end(12);
        emu_page.set_margin_top(12);
        emu_page.set_margin_bottom(12);
        let status_group = adw::PreferencesGroup::new();
        status_group.set_title("Status");

        let status_row = adw::ActionRow::new();
        let is_installed = if emu_trophy_source == crate::models::GSE {
            crate::platforms::api_emulators::is_gse_installed(&emu_exe)
        } else {
            crate::platforms::api_emulators::is_nge_installed(&emu_exe)
        };
        status_row.set_title(if is_installed { "API emulator installed" } else { "API emulator not installed" });
        status_row.set_sensitive(false);
        status_group.add(&status_row);

        emu_page.append(&status_group);

        let action_group = adw::PreferencesGroup::new();
        action_group.set_title("Actions");

        if is_installed {
            let uninstall_btn = gtk4::Button::with_label("Uninstall API emulator");
            uninstall_btn.add_css_class("destructive-action");
            let exe_c = emu_exe.clone();
            let ts_c = emu_trophy_source.clone();
            let status_c = status_row.clone();
            uninstall_btn.connect_clicked(move |_| {
                let result = if ts_c == crate::models::GSE {
                    crate::platforms::api_emulators::uninstall_gse(&exe_c)
                } else {
                    crate::platforms::api_emulators::uninstall_nge(&exe_c)
                };
                match result {
                    Ok(()) => {
                        status_c.set_title("API emulator not installed");
                    }
                    Err(e) => eprintln!("Uninstall failed: {}", e),
                }
            });
            action_group.add(&uninstall_btn);
        } else {
            let versions = if emu_trophy_source == crate::models::GSE {
                crate::platforms::api_emulators::list_gse_versions(&emu_save_dir)
            } else {
                crate::platforms::api_emulators::list_gog_versions(&emu_save_dir)
            };
            let has_dlls = if emu_trophy_source == crate::models::GSE {
                crate::platforms::api_emulators::has_original_steam_dlls(&emu_exe)
            } else {
                crate::platforms::api_emulators::has_original_gog_dlls(&emu_exe)
            };

            if !has_dlls {
                let missing_row = adw::ActionRow::new();
                missing_row.set_title("No original Steam/GOG DLLs detected in game folder");
                missing_row.set_subtitle("Install the game first and make sure it has the original API DLLs");
                missing_row.set_sensitive(false);
                action_group.add(&missing_row);
            }

            let version_row = if !versions.is_empty() {
                let vr = adw::ComboRow::new();
                vr.set_title("Emulator version");
                vr.set_subtitle("Version directory to use for installation");
                let strings: Vec<&str> = versions.iter().map(|s| s.as_str()).collect();
                let model = gtk4::StringList::new(&strings);
                vr.set_model(Some(&model));
                let default_ver = &state.borrow().cfg.default_api_emu_version;
                if !default_ver.is_empty() {
                    if let Some(idx) = versions.iter().position(|v| v == default_ver) {
                        vr.set_selected(idx as u32);
                    }
                }
                action_group.add(&vr);
                Some(vr)
            } else {
                let no_ver_row = adw::ActionRow::new();
                no_ver_row.set_title("No emulator versions available");
                no_ver_row.set_subtitle("Place version directories in api_emulators/");
                no_ver_row.set_sensitive(false);
                action_group.add(&no_ver_row);
                None
            };

            let install_btn = gtk4::Button::with_label("Install API emulator");
            install_btn.add_css_class("suggested-action");
            install_btn.set_sensitive(has_dlls);
            let exe_c = emu_exe.clone();
            let ts_c = emu_trophy_source.clone();
            let app_id_c = emu_app_id.clone();
            let save_dir_c = emu_save_dir.clone();
            let status_c = status_row.clone();
            let langs_c = languages.clone();
            install_btn.connect_clicked(move |_| {
                let ver = version_row.as_ref().map(|vr| {
                    let idx = vr.selected() as usize;
                    if idx < versions.len() { versions[idx].clone() } else { String::new() }
                }).unwrap_or_default();
                let result = if ts_c == crate::models::GSE {
                    crate::platforms::api_emulators::install_gse(&save_dir_c, &exe_c, &app_id_c, &langs_c, &ver)
                } else {
                    crate::platforms::api_emulators::install_nge(&save_dir_c, &exe_c, &app_id_c, &ver)
                };
                match result {
                    Ok(()) => {
                        status_c.set_title("API emulator installed");
                    }
                    Err(e) => eprintln!("Install failed: {}", e),
                }
            });
            action_group.add(&install_btn);
        }

        if emu_trophy_source == crate::models::GSE && is_installed {
            let gen_btn = gtk4::Button::with_label("Generate steam_interfaces.txt");
            gen_btn.add_css_class("flat");
            let exe_c = emu_exe.clone();
            gen_btn.connect_clicked(move |_| {
                let game_dir = std::path::Path::new(&exe_c).parent();
                if let Some(dir) = game_dir {
                    let settings_dir = dir.join("steam_settings");
                    let gen_path = settings_dir.join("generate_interfaces");
                    if gen_path.is_file() {
                        let _ = std::process::Command::new(&gen_path)
                            .current_dir(&settings_dir)
                            .status();
                    } else {
                        eprintln!("generate_interfaces not found in steam_settings folder");
                    }
                }
            });
            action_group.add(&gen_btn);
        }

        emu_page.append(&action_group);

        let emu_scroll = gtk4::ScrolledWindow::new();
        emu_scroll.set_child(Some(&emu_page));
        emu_scroll.set_vexpand(true);
        emu_scroll.set_hexpand(true);
        sidebar.append(&super::dialogs::sidebar_separator());
        sidebar.append(&super::dialogs::settings_sidebar_row("applications-engineering-symbolic", "API Emulator"));
        stack.add_named(&emu_scroll, Some("api_emulator"));
    }

    // --- Variants page ---
    let variants: Vec<crate::models::GameVariant> = crate::db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let variant_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let variant_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    variant_container.set_margin_start(12);
    variant_container.set_margin_end(12);
    variant_page.append(&variant_container);

    struct VarW {
        _name: adw::EntryRow,
        _exe: adw::EntryRow,
        _wd: adw::EntryRow,
        _args: adw::EntryRow,
        _group: adw::PreferencesGroup,
    }

    let var_widgets: Rc<RefCell<Vec<VarW>>> = Rc::new(RefCell::new(Vec::new()));

    let add_variant_fn = {
        let var_widgets = var_widgets.clone();
        let container = variant_container.clone();
        move |v: crate::models::GameVariant| {
            let group = adw::PreferencesGroup::new();
            let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            del_btn.add_css_class("flat");
            del_btn.add_css_class("error");
            del_btn.set_valign(gtk4::Align::Center);
            let container_c = container.clone();
            let group_c = group.clone();
            del_btn.connect_clicked(move |_| {
                container_c.remove(&group_c);
            });
            group.set_header_suffix(Some(&del_btn));

            let name_entry = adw::EntryRow::new();
            name_entry.set_title("Variant name");
            name_entry.set_text(&v.name);
            group.add(&name_entry);

            let exe_entry = adw::EntryRow::new();
            exe_entry.set_title("Executable");
            exe_entry.set_text(&v.exe);
            let browse = gtk4::Button::from_icon_name("folder-open-symbolic");
            browse.add_css_class("flat");
            browse.set_valign(gtk4::Align::Center);
            let exe_c = exe_entry.clone();
            browse.connect_clicked(move |_| {
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Select variant executable");
                let filter = gtk4::FileFilter::new();
                filter.add_mime_type("application/x-executable");
                filter.add_pattern("*");
                dialog.set_default_filter(Some(&filter));
                let entry = exe_c.clone();
                dialog.open(None::<&adw::Window>, None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                        }
                    }
                });
            });
            exe_entry.add_suffix(&browse);
            group.add(&exe_entry);

            let args_entry = adw::EntryRow::new();
            args_entry.set_title("Arguments");
            args_entry.set_text(&v.args);
            group.add(&args_entry);

            let wd_entry = adw::EntryRow::new();
            wd_entry.set_title("Working directory");
            wd_entry.set_text(&v.working_dir);
            let wd_browse = gtk4::Button::from_icon_name("folder-open-symbolic");
            wd_browse.add_css_class("flat");
            wd_browse.set_valign(gtk4::Align::Center);
            let wd_c = wd_entry.clone();
            wd_browse.connect_clicked(move |_| {
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Select working directory");
                dialog.set_default_filter(Some(&gtk4::FileFilter::new()));
                let entry = wd_c.clone();
                dialog.select_folder(None::<&adw::Window>, None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                        }
                    }
                });
            });
            wd_entry.add_suffix(&wd_browse);
            group.add(&wd_entry);

            container.append(&group);

            var_widgets.borrow_mut().push(VarW {
                _name: name_entry,
                _exe: exe_entry,
                _wd: wd_entry,
                _args: args_entry,
                _group: group,
            });
        }
    };

    for v in &variants {
        add_variant_fn(v.clone());
    }

    let add_btn = gtk4::Button::with_label("Add variant");
    add_btn.add_css_class("suggested-action");
    add_btn.set_margin_top(8);
    {
        let add_variant_fn = add_variant_fn;
        let new_v = crate::models::GameVariant { game_id: db_id, ..Default::default() };
        add_btn.connect_clicked(move |_| add_variant_fn(new_v.clone()));
    }
    variant_page.append(&add_btn);

    let variant_scroll = gtk4::ScrolledWindow::new();
    variant_scroll.set_child(Some(&variant_page));
    variant_scroll.set_vexpand(true);
    variant_scroll.set_hexpand(true);
    if !is_native_platform && (game.kind != "lutris" || has_config || !variants.is_empty()) {
        sidebar.append(&super::dialogs::sidebar_separator());
        sidebar.append(&super::dialogs::settings_sidebar_row("application-x-executable-symbolic", "Variants"));
        stack.add_named(&variant_scroll, Some("variants"));
    }

    // --- Sidebar navigation ---
    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "Launch Config" => "launch",
                                "Performance" => "Performance",
                                "Graphics" => "Graphics",
                                "Wine Advanced" => "Wine Advanced",
                                "Images" => "images",
                                "Logo" => "logo",
                                "DLC" => "dlc",
                                "API Emulator" => "api_emulator",
                                "Variants" => "variants",
                                _ => "general",
                            };
                            stack_clone.set_visible_child_name(page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    content_area.append(&stack);

    // --- Save / Cancel ---
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    // --- Save handler ---
    let state_clone = state.clone();
    let win_s = win.clone();
    let db_id_s = db_id;
    let lutris_id = game.lutris_id;
    let app_id = game.app_id.clone();
    let trophy_source = game.trophy_source.clone();
    let var_widgets_save = var_widgets.clone();
    let save_dir_c = save_dir.clone();
    let logo_controls_c = logo_controls.clone();
    let dlc_switches_c = dlc_switches.clone();
    let pending_copies_c = pending_copies.clone();
    let old_wine = saved_wine.clone();
    let app_default_wine_c = app_default_wine.clone();
    let game_exe = saved_launch.exe.clone();
    let language_row_c = language_row.clone();
    let languages_c = languages.clone();

    save_btn.connect_clicked(move |_| {
        let title = title_entry.text().to_string();
        let sort_title = sort_entry.text().to_string();

        let db = state_clone.borrow().db.clone();

        if let Err(e) = crate::db::update_game_title(&db, db_id_s, &title) {
            eprintln!("Failed to update game: {}", e);
        }
        if let Err(e) = crate::db::update_sort_title(&db, db_id_s, &sort_title) {
            eprintln!("Failed to update sort title: {}", e);
        }

        let mut app_id_changed = false;
        let mut new_app_id_val = String::new();
        if let Some(ref app_id_row) = app_id_entry {
            let new_id = app_id_row.text().to_string();
            if new_id != app_id {
                app_id_changed = true;
                new_app_id_val = new_id.clone();
                let ts = if new_id.is_empty() { "" } else { &trophy_source };
                let pid = if new_id.is_empty() { "" } else { &new_id };
                if let Err(e) = crate::db::update_game_ids(&db, db_id_s, &new_id, ts, pid) {
                    eprintln!("Failed to update app ID: {}", e);
                }
            }
        }

        if let Some(ver) = pending_version.borrow().as_ref() {
            let _ = crate::db::set_shadps4_version(&db, db_id_s, ver);
        }

        // Save launch config + wine config
        if let Some(ref lc) = launch_config_widgets {
            let launch = GameLaunchConfig {
                exe: lc.exe_entry.text().to_string(),
                args: lc.args_entry.text().to_string(),
                working_dir: lc.wd_entry.text().to_string(),
                env_vars: collect_env_vars(&lc.env_vars_box),
                ld_preload: lc.ld_preload_entry.text().to_string(),
                ld_library_path: lc.ld_path_entry.text().to_string(),
            };
            let mut wine = wine_widgets_opt.as_ref().map_or(WineConfig::default(), |ww| ww.to_wine_config());

            if wine.dll_overrides != app_default_wine_c.dll_overrides {
                if !wine.overridden_fields.contains(&"dll_overrides".to_string()) {
                    wine.overridden_fields.push("dll_overrides".to_string());
                }
            } else {
                wine.overridden_fields.retain(|f| f != "dll_overrides");
            }
            if wine.wine_env_vars != app_default_wine_c.wine_env_vars {
                if !wine.overridden_fields.contains(&"wine_env_vars".to_string()) {
                    wine.overridden_fields.push("wine_env_vars".to_string());
                }
            } else {
                wine.overridden_fields.retain(|f| f != "wine_env_vars");
            }
            let new_profile_id = if let Some(ref profile_row) = profile_row {
                if profile_row.selected() > 0 {
                    profiles.get((profile_row.selected() - 1) as usize).map(|p| p.id)
                } else {
                    None
                }
            } else {
                saved_profile_id
            };
            let _ = crate::db::save_game_config(&db, db_id_s, &launch, &wine, new_profile_id);

            if wine.enabled {
                let reg_changed = wine.mouse_warp_override != old_wine.mouse_warp_override
                    || wine.virtual_desktop != old_wine.virtual_desktop
                    || wine.virtual_desktop_res != old_wine.virtual_desktop_res
                    || wine.dpi_enabled != old_wine.dpi_enabled
                    || wine.dpi != old_wine.dpi
                    || wine.show_crash_dialogs != old_wine.show_crash_dialogs
                    || wine.audio != old_wine.audio;

                if reg_changed {
                    let pfx = crate::launcher::wine_launch::wine_prefix(&wine);
                    let prefix_ready = std::path::Path::new(&pfx).join("system.reg").is_file();
                    if prefix_ready {
                        let wine_exe = crate::launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path);
                        if let Ok(wine_exe) = wine_exe {
                            let reg_cmds = crate::launcher::wine_launch::build_wine_reg_commands(&wine, &wine_exe);
                            let env = crate::launcher::wine_launch::build_wine_env(&wine, &wine_exe);
                            std::thread::spawn(move || {
                                for reg_cmd in reg_cmds {
                                    let mut child = std::process::Command::new(&reg_cmd[0]);
                                    for arg in &reg_cmd[1..] {
                                        child.arg(arg);
                                    }
                                    child.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                                    match child.status() {
                                        Ok(s) if !s.success() && s.code() != Some(1) => {
                                            eprintln!("Wine reg command failed (exit {:?}): {:?}", s.code(), reg_cmd);
                                        }
                                        Err(e) => eprintln!("Failed to run wine reg command: {}", e),
                                        _ => {}
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }

        // Pending image copies
        {
            let pc = pending_copies_c.borrow();
            for (asset, src_path) in pc.iter() {
                let cloud_dir = if !app_id.is_empty() {
                    crate::parser::data_dir(&save_dir_c, &app_id)
                } else {
                    continue;
                };
                let file_name = match asset.as_str() {
                    "icon" => "icon.png",
                    "hero" => "library_hero.jpg",
                    "grid" => "library_600x900.jpg",
                    "header" => "header.jpg",
                    "logo" => "logo.png",
                    _ => continue,
                };
                let dest = cloud_dir.join(file_name);
                let is_ico = src_path.ends_with(".ico");
                if is_ico {
                    let ico_dest = dest.with_extension("ico");
                    if std::fs::copy(&src_path, &ico_dest).is_ok() {
                        let _ = crate::parser::convert_ico_to_png(&ico_dest);
                    }
                } else if let Err(e) = std::fs::copy(&src_path, &dest) {
                    eprintln!("Failed to copy {}: {}", asset, e);
                }
            }
        }

        if pending_copies_c.borrow().contains_key("__unmatch__") {
            let _ = crate::db::set_sgdb_id(&db, db_id_s, "");
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.sgdb_id.clear();
            }
            pending_copies_c.borrow_mut().remove("__unmatch__");
        }

        if let Some((ref selected_pos, ref size_adj)) = logo_controls_c {
            let pos = selected_pos.borrow().clone();
            let size = size_adj.value() as i32;
            if db_id_s != 0 {
                if let Err(e) = crate::db::set_logo_settings(&db, db_id_s, &pos, size) {
                    eprintln!("Failed to update logo settings: {}", e);
                }
            }
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.logo_position = pos;
                g.logo_size = size;
            }
        }

        // DLC state
        {
            let details = crate::parser::read_app_details(&save_dir_c, &app_id);
            if let Some(ref details) = details {
                if !dlc_switches_c.is_empty() {
                    let mut details = details.clone();
                    let dlcs_vec: Vec<_> = details.dlcs.iter_mut().collect();
                    for (i, (_, dlc)) in dlcs_vec.into_iter().enumerate() {
                        if i < dlc_switches_c.len() {
                            dlc.enabled = dlc_switches_c[i].is_active();
                        }
                    }
                    let path = crate::parser::data_dir(&save_dir_c, &app_id).join("appdetails.json");
                    if let Ok(b) = serde_json::to_vec(&details) {
                        let _ = std::fs::write(&path, b);
                    }
                    crate::platforms::api_emulators::write_dlc_configs(
                        &trophy_source, &game_exe, &save_dir_c, &app_id, &details,
                    );
                }
            }
        }

        // Game language
        if let Some(ref lang_row) = language_row_c {
            let idx = lang_row.selected() as usize;
            if let Some(lang) = languages_c.get(idx) {
                crate::platforms::api_emulators::write_language_configs(
                    &trophy_source, &game_exe, &save_dir_c, &app_id, lang,
                );
            }
        }

        // Update in-memory state
        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.name = title.clone();
            g.sort_title = sort_title.clone();
            if let Some(ver) = pending_version.borrow().as_ref() {
                g.shadps4_version = ver.clone();
            }
            if app_id_changed {
                if new_app_id_val.is_empty() {
                    g.app_id.clear();
                    g.trophy_source.clear();
                    g.platform_id.clear();
                    g.achievements.clear();
                    g.earned_count = 0;
                    g.total_count = 0;
                    g.manual_unmatch = true;
                    if !g.lutris_name.is_empty() && g.name == format!("App ID: {}", app_id) {
                        g.name = g.lutris_name.clone();
                    }
                } else {
                    state_clone.borrow().game_names.lock().unwrap().remove(&app_id);
                    g.app_id = new_app_id_val.clone();
                    g.platform_id = new_app_id_val.clone();
                    g.manual_unmatch = false;
                }
            }
        }
        if !new_app_id_val.is_empty() {
            state_clone.borrow().game_names.lock().unwrap().insert(new_app_id_val.clone(), title);
        } else if app_id_changed {
            state_clone.borrow().game_names.lock().unwrap().remove(&app_id);
        }

        super::sidebar::rebuild_sidebar(&state_clone);

        let selected = state_clone.borrow().selected_id.clone();
        let game_after_save = if selected == lutris_id.to_string() {
            state_clone.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned()
        } else {
            None
        };
        if let Some(g) = game_after_save {
            crate::ui::game_display::display_game(&g, &state_clone);
        }

        // Save variants
        {
            let _ = crate::db::delete_all_variants(&db, db_id_s);
            for vw in var_widgets_save.borrow().iter() {
                if vw._group.parent().is_none() { continue; }
                let name = vw._name.text().to_string();
                if name.is_empty() { continue; }
                let variant = crate::models::GameVariant {
                    id: 0,
                    game_id: db_id_s,
                    name,
                    exe: vw._exe.text().to_string(),
                    working_dir: vw._wd.text().to_string(),
                    args: vw._args.text().to_string(),
                    env_vars: Vec::new(),
                };
                let _ = crate::db::add_variant(&db, &variant);
            }
        }

        win_s.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);
    outer.append(&content_area);

    win.set_content(Some(&outer));
    win.present();
}

struct LaunchConfigWidgets {
    exe_entry: adw::EntryRow,
    args_entry: adw::EntryRow,
    wd_entry: adw::EntryRow,
    env_vars_box: gtk4::ListBox,
    ld_preload_entry: adw::EntryRow,
    ld_path_entry: adw::EntryRow,
}

fn build_launch_config_page(
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

    let exe_browse = gtk4::Button::from_icon_name("folder-open-symbolic");
    exe_browse.add_css_class("flat");
    exe_browse.set_valign(gtk4::Align::Center);
    let exe_entry_b = exe_entry.clone();
    let win_b = win.clone();
    exe_browse.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select executable");
        let filter = gtk4::FileFilter::new();
        filter.add_mime_type("application/x-executable");
        filter.add_pattern("*");
        dialog.set_default_filter(Some(&filter));
        let entry = exe_entry_b.clone();
        dialog.open(Some(&win_b), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    exe_entry.add_suffix(&exe_browse);
    lc_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    args_entry.set_text(&launch.args);
    lc_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");
    wd_entry.set_text(&launch.working_dir);

    let wd_browse = gtk4::Button::from_icon_name("folder-open-symbolic");
    wd_browse.add_css_class("flat");
    wd_browse.set_valign(gtk4::Align::Center);
    let wd_entry_b = wd_entry.clone();
    let win_wd = win.clone();
    wd_browse.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select working directory");
        let entry = wd_entry_b.clone();
        dialog.select_folder(Some(&win_wd), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    wd_entry.add_suffix(&wd_browse);
    lc_group.add(&wd_entry);

    page.append(&lc_group);
    sidebar.append(&super::dialogs::settings_sidebar_row("preferences-other-symbolic", "Launch Config"));
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

        sidebar.append(&super::dialogs::settings_sidebar_row("preferences-other-symbolic", "Advanced"));
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
