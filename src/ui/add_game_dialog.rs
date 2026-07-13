use crate::models::{GameLaunchConfig, WineConfig};
use crate::AppMessage;
use gtk4::prelude::*;
use adw::prelude::*;
use super::state::SharedState;

pub fn show_add_game_dialog(state: &SharedState) {
    let (window, db, sender, steam, watcher, save_dir) = {
        let s = state.borrow();
        (s.window.clone(), s.db.clone(), s.sender.clone(), s.steam.clone(), s.watcher.clone(), s.save_dir.clone())
    };

    let win = adw::Window::new();
    win.set_title(Some("Add Game"));
    win.set_default_size(720, 540);
    win.set_transient_for(Some(&window));
    win.set_modal(true);

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
    header.set_title_widget(Some(&gtk4::Label::new(Some("Add Game"))));
    content_area.append(&header);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_margin_start(16);
    stack.set_margin_end(16);
    stack.set_margin_top(16);
    stack.set_margin_bottom(16);
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let profiles = crate::db::get_all_profiles(&db).unwrap_or_default();
    let (general_page, name_entry, kind_row, exe_entry, args_entry, wd_entry, detect_btn, profile_row, steam_id_entry, gog_id_entry) =
        build_general_page(&win, &profiles, state);
    sidebar.append(&super::dialogs::settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let (wine_pages, wine_widgets) = {
        let dft = state.borrow().cfg.default_wine_config.clone();
        let cfg = if dft.enabled { dft.clone() } else { WineConfig { enabled: true, ..dft.clone() } };
        super::wine_config_widget::build_wine_config_pages(&cfg, Some(&dft))
    };

    let sep1 = super::dialogs::sidebar_separator();
    sidebar.append(&sep1);

    let mut wine_sidebar_rows: Vec<gtk4::ListBoxRow> = Vec::new();
    for wp in &wine_pages {
        let row = super::dialogs::settings_sidebar_row(wp.icon, wp.label);
        sidebar.append(&row);
        stack.add_named(&wp.page, Some(wp.label));
        wine_sidebar_rows.push(row);
    }

    let sep2 = super::dialogs::sidebar_separator();
    sidebar.append(&sep2);

    {
        let rows = wine_sidebar_rows.clone();
        let sep1_c = sep1.clone();
        let sep2_c = sep2.clone();
        let profile_row_c = profile_row.clone();
        kind_row.connect_selected_notify(move |row| {
            let visible = row.selected() == 1;
            for r in &rows {
                r.set_visible(visible);
            }
            sep1_c.set_visible(visible);
            sep2_c.set_visible(visible);
            profile_row_c.set_visible(visible);
        });
        let visible = kind_row.selected() == 1;
        for r in &wine_sidebar_rows {
            r.set_visible(visible);
        }
        sep1.set_visible(visible);
        sep2.set_visible(visible);
        profile_row.set_visible(visible);
    }

    let (env_page, env_vars_box, ld_preload_entry, ld_library_entry) = build_env_page();
    sidebar.append(&super::dialogs::settings_sidebar_row("preferences-other-symbolic", "Environment"));
    stack.add_named(&env_page, Some("env"));

    let detect_group = adw::PreferencesGroup::new();
    detect_group.set_title("Quick detect");
    let detect_row = adw::ActionRow::new();
    detect_row.set_title("Select game folder to auto-detect");
    detect_row.add_suffix(&detect_btn);
    detect_group.add(&detect_row);
    general_page.append(&detect_group);

    {
        let sc = state.clone();
        let n_detect = name_entry.clone();
        let exe_detect = exe_entry.clone();
        let sid_detect = steam_id_entry.clone();
        let gid_detect = gog_id_entry.clone();
        detect_btn.connect_clicked(move |_| {
            let file_dialog = gtk4::FileDialog::new();
            file_dialog.set_title("Select game folder");
            let sc_c = sc.clone();
            let n = n_detect.clone();
            let exe = exe_detect.clone();
            let sid = sid_detect.clone();
            let gid = gid_detect.clone();
            file_dialog.select_folder(Some(&sc_c.borrow().window), None::<&gio::Cancellable>, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let folder = path.to_string_lossy().into_owned();
                if let Some(app_id) = crate::platforms::steam::detect_app_id(&folder) {
                    sid.set_text(&app_id);
                }
                if crate::platforms::gog::is_gog_game(&folder) {
                    if let Some((_info_dir, product_id, game_name)) = crate::platforms::gog::find_gog_info(&folder) {
                        gid.set_text(&product_id);
                        if n.text().is_empty() {
                            n.set_text(&game_name);
                        }
                    }
                }
                if exe.text().is_empty() {
                    if let Ok(entries) = std::fs::read_dir(&folder) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if let Some(ext) = p.extension() {
                                if ext == "exe" || ext == "x86_64" || ext == "AppRun" || ext == "sh" {
                                    exe.set_text(&p.to_string_lossy());
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        });
    }

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "Performance" => "Performance",
                                "Graphics" => "Graphics",
                                "Wine Advanced" => "Wine Advanced",
                                "Environment" => "env",
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

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let add_btn = gtk4::Button::with_label("Add Game");
    add_btn.add_css_class("suggested-action");

    btn_row.append(&cancel_btn);
    btn_row.append(&add_btn);
    content_area.append(&btn_row);

    outer.append(&content_area);
    win.set_content(Some(&outer));
    win.present();

    let state_c = state.clone();
    add_btn.connect_clicked(move |_| {
        name_entry.remove_css_class("error");
        exe_entry.remove_css_class("error");

        let name = name_entry.text().to_string();
        if name.is_empty() {
            name_entry.add_css_class("error");
            return;
        }
        let exe_path = exe_entry.text().to_string();
        if exe_path.is_empty() {
            exe_entry.add_css_class("error");
            return;
        }
        if !std::path::Path::new(&exe_path).is_file() {
            exe_entry.add_css_class("error");
            return;
        }
        let is_wine = kind_row.selected() == 1;
        let args = args_entry.text().to_string();
        let wd = wd_entry.text().to_string();
        let steam_app_id = steam_id_entry.text().to_string();
        let gog_product_id = gog_id_entry.text().to_string();

        let trophy_source = if !steam_app_id.is_empty() { "gse" } else if !gog_product_id.is_empty() { "nge" } else { "" };
        let kind = if is_wine { "wine" } else { "linux" };
        let platform_id = if !steam_app_id.is_empty() { steam_app_id.clone() } else if !gog_product_id.is_empty() { gog_product_id.clone() } else { format!("manual_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()) };

        let selected_profile_id = if is_wine && profile_row.selected() > 0 {
            profiles.get((profile_row.selected() - 1) as usize).map(|p| p.id)
        } else {
            None
        };

        let launch_config = GameLaunchConfig {
            exe: exe_path,
            args,
            working_dir: wd,
            env_vars: collect_env_vars(&env_vars_box),
            ld_preload: ld_preload_entry.text().to_string(),
            ld_library_path: ld_library_entry.text().to_string(),
        };
        let wine_config = if is_wine { wine_widgets.to_wine_config() } else { WineConfig::default() };

        let db_c = db.clone();
        let sender_c = sender.clone();
        let name_c = name.clone();
        let app_id_c = platform_id.clone();
        let kind_c = kind.to_string();
        let ts_c = trophy_source.to_string();
        let steam_c = steam.clone();
        let watcher_c = watcher.clone();
        let save_dir_c = save_dir.clone();

        std::thread::spawn(move || {
            match add_game_to_db(&db_c, &name_c, &kind_c, &ts_c, &app_id_c, &platform_id, &launch_config, &wine_config, selected_profile_id, &steam_c, &save_dir_c) {
                Ok(_game_id) => {
                    let entry = crate::db::find_by_steam_id(&db_c, &app_id_c).ok().flatten();
                    if let Some(entry) = entry {
                        if let Ok(mut game) = crate::parser::load_game(&entry, &save_dir_c) {
                            game.name = name_c.clone();
                            let _ = crate::db::update_game_title(&db_c, game.db_id, &name_c);
                            if let Some(ref w) = watcher_c {
                                w.watch(&entry, &game.achievements);
                            }
                            let _ = sender_c.send(AppMessage::NewGame(game.clone()));
                            let g_name = game.name.clone();
                            crate::ui::enrichment::enrich_game_async(
                                game.app_id.clone(), game.trophy_source.clone(), game.platform_id.clone(),
                                game.db_id, game.lutris_id, g_name,
                                steam_c, watcher_c, sender_c, save_dir_c, db_c,
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to add game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        });

        let win_c2 = win.clone();
        let sc2 = state_c.clone();
        win_c2.close();
        let _ = sc2;
    });
}

fn build_general_page(win: &adw::Window, profiles: &[crate::models::WineProfile], state: &super::state::SharedState) -> (gtk4::Box, adw::EntryRow, adw::ComboRow, adw::EntryRow, adw::EntryRow, adw::EntryRow, gtk4::Button, adw::ComboRow, adw::EntryRow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title("Game Info");

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Name");
    info_group.add(&name_entry);

    let kind_model = gtk4::StringList::new(&["Native Linux", "Wine (Windows)"]);
    let kind_row = adw::ComboRow::new();
    kind_row.set_title("Kind");
    kind_row.set_model(Some(&kind_model));
    kind_row.set_selected(1);
    info_group.add(&kind_row);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable");

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
        filter.add_mime_type("application/x-msdos-program");
        filter.add_pattern("*.exe");
        filter.add_pattern("*.msi");
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
    info_group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    info_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");

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
    info_group.add(&wd_entry);

    let profile_labels: Vec<String> = std::iter::once("Custom (per-game)".to_string())
        .chain(profiles.iter().map(|p| p.name.clone()))
        .collect();
    let str_refs: Vec<&str> = profile_labels.iter().map(|s| s.as_str()).collect();
    let profile_model = gtk4::StringList::new(&str_refs);
    let profile_row = adw::ComboRow::new();
    profile_row.set_title("Wine Profile");
    profile_row.set_subtitle("Links wine version + prefix together");
    profile_row.set_model(Some(&profile_model));
    info_group.add(&profile_row);

    page.append(&info_group);

    let ids_group = adw::PreferencesGroup::new();
    ids_group.set_title("Service IDs");
    let steam_id_entry = adw::EntryRow::new();
    steam_id_entry.set_title("Steam App ID");
    let steam_search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    steam_search_btn.set_valign(gtk4::Align::Center);
    steam_search_btn.set_tooltip_text(Some("Search Steam Store"));
    steam_search_btn.add_css_class("flat");
    {
        let sc = state.clone();
        let win_c = win.clone();
        let row_c = steam_id_entry.clone();
        let name_c = String::new();
        steam_search_btn.connect_clicked(move |_| {
            super::dialogs::show_steam_id_search_popup_add(&sc, &name_c, &win_c, &row_c);
        });
    }
    steam_id_entry.add_suffix(&steam_search_btn);
    ids_group.add(&steam_id_entry);
    let gog_id_entry = adw::EntryRow::new();
    gog_id_entry.set_title("GOG Product ID");
    let gog_browse_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    gog_browse_btn.set_valign(gtk4::Align::Center);
    gog_browse_btn.set_tooltip_text(Some("Detect from game folder"));
    gog_browse_btn.add_css_class("flat");
    {
        let win_c = win.clone();
        let row_c = gog_id_entry.clone();
        let name_row = name_entry.clone();
        gog_browse_btn.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select game folder");
            let row = row_c.clone();
            let name_row = name_row.clone();
            dialog.select_folder(Some(&win_c), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        if let Some((_, product_id, name)) = crate::platforms::gog::find_gog_info(&path.to_string_lossy()) {
                            row.set_text(&product_id);
                            if name_row.text().is_empty() {
                                name_row.set_text(&name);
                            }
                        }
                    }
                }
            });
        });
    }
    gog_id_entry.add_suffix(&gog_browse_btn);
    ids_group.add(&gog_id_entry);
    page.append(&ids_group);

    let detect_btn = gtk4::Button::with_label("Browse");
    detect_btn.add_css_class("flat");

    (page, name_entry, kind_row, exe_entry, args_entry, wd_entry, detect_btn, profile_row, steam_id_entry, gog_id_entry)
}

fn build_env_page() -> (gtk4::Box, gtk4::ListBox, adw::EntryRow, adw::EntryRow) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let env_group = adw::PreferencesGroup::new();
    env_group.set_title("Environment Variables");

    let env_vars_box = gtk4::ListBox::new();
    env_vars_box.add_css_class("boxed-list");
    env_group.add(&env_vars_box);

    let add_env_btn = gtk4::Button::with_label("Add variable");
    add_env_btn.add_css_class("flat");
    let env_box_clone = env_vars_box.clone();
    add_env_btn.connect_clicked(move |_| {
        env_box_clone.append(&build_env_var_row("", ""));
    });
    env_group.add(&add_env_btn);
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

fn add_game_to_db(
    db: &crate::db::DbConn,
    name: &str,
    kind: &str,
    trophy_source: &str,
    app_id: &str,
    platform_id: &str,
    launch_config: &GameLaunchConfig,
    wine_config: &WineConfig,
    profile_id: Option<i64>,
    steam: &crate::api::SteamClient,
    save_dir: &str,
) -> Result<i64, String> {
    let game_id = crate::db::add_game(db, kind, trophy_source, app_id, platform_id, name)?;
    crate::db::save_game_config(db, game_id, launch_config, wine_config, profile_id)?;
    if !app_id.is_empty() && app_id.parse::<i64>().is_ok() {
        let folder = std::path::Path::new(&launch_config.exe).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        if !folder.is_empty() {
            let _ = crate::platforms::steam::add_game_from_folder(&folder, app_id, kind, steam, db, save_dir);
        }
    }
    Ok(game_id)
}

pub(super) fn build_env_var_row(key: &str, value: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let key_entry = gtk4::Entry::new();
    key_entry.set_placeholder_text(Some("Variable name (e.g. FOO)"));
    key_entry.set_text(key);
    key_entry.set_hexpand(true);
    hbox.append(&key_entry);

    let val_entry = gtk4::Entry::new();
    val_entry.set_placeholder_text(Some("Value"));
    val_entry.set_text(value);
    val_entry.set_hexpand(true);
    hbox.append(&val_entry);

    let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove_btn.add_css_class("flat");
    remove_btn.add_css_class("circular");
    let row_clone = row.clone();
    remove_btn.connect_clicked(move |_| {
        row_clone.parent().and_then(|p| p.downcast::<gtk4::ListBox>().ok()).map(|list| {
            row_clone.unparent();
            list.remove(&row_clone);
        });
    });
    hbox.append(&remove_btn);

    row.set_child(Some(&hbox));
    row
}

pub(super) fn collect_env_vars(box_: &gtk4::ListBox) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut child = box_.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<gtk4::ListBoxRow>() {
            if let Some(hbox) = row.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                let children: Vec<gtk4::Widget> = {
                    let mut v = Vec::new();
                    let mut ch = hbox.first_child();
                    while let Some(c) = ch.clone() {
                        v.push(c.clone());
                        ch = c.next_sibling();
                    }
                    v
                };
                if children.len() >= 2 {
                    let key_w = &children[0];
                    let val_w = &children[1];
                    if let Some(key) = key_w.downcast_ref::<gtk4::Entry>() {
                        if let Some(val) = val_w.downcast_ref::<gtk4::Entry>() {
                            let k = key.text().to_string();
                            if !k.is_empty() {
                                result.push((k, val.text().to_string()));
                            }
                        }
                    }
                }
            }
        }
        child = w.next_sibling();
    }
    result
}
