use super::add_game_db::{add_game_to_db, AddGameToDbParams};
use super::add_game_env::build_env_page;
use super::add_game_general::build_general_page;
use super::css::*;
use super::state::SharedState;
use super::wine_config_widget::WineConfigWidgets;
use crate::AppMessage;
use adw::prelude::*;
use ira_models::{GameLaunchConfig, WineConfig};

pub fn show_add_game_dialog(state: &SharedState) {
    let (window, db, sender, steam, save_dir, ra_username, ra_web_api_key) = {
        let s = state.borrow();
        (
            s.window.clone(),
            s.db.clone(),
            s.sender.clone(),
            s.steam.clone(),
            s.save_dir.clone(),
            s.cfg.ra_username.clone(),
            s.cfg.ra_web_api_key.clone(),
        )
    };

    let layout = super::helpers::dialog_layout();
    layout.window.set_title(&crate::tr!("Add game"));
    layout
        .header
        .set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!("Add game")))));
    layout.stack.set_vexpand(true);

    let win = layout.window;
    let sidebar = layout.sidebar;
    let stack = layout.stack;
    let content_area = layout.content_area;

    let profiles = ira_db::get_all_profiles(&db).unwrap_or_default();
    let (
        general_page,
        name_entry,
        kind_row,
        folder_entry,
        exe_entry,
        args_entry,
        wd_entry,
        detect_btn,
        profile_row,
        steam_id_entry,
        gog_id_entry,
    ) = build_general_page(&win, &profiles, state);
    sidebar.append(&super::settings_dialog::settings_sidebar_row(
        "emblem-system-symbolic",
        &crate::tr!("General"),
        "general",
    ));
    stack.add_named(&general_page, Some("general"));

    let (wine_pages, wine_widgets) = {
        let dft = state.borrow().cfg.default_wine_config.clone();
        let cfg = if dft.enabled {
            dft.clone()
        } else {
            WineConfig {
                enabled: true,
                ..dft.clone()
            }
        };
        super::wine_config_widget::build_wine_config_pages(&cfg, Some(&dft), &save_dir)
    };

    let sep1 = super::settings_dialog::sidebar_separator();
    sidebar.append(&sep1);

    let mut wine_sidebar_rows: Vec<gtk4::ListBoxRow> = Vec::new();
    for wp in &wine_pages {
        let row = super::settings_dialog::settings_sidebar_row(wp.icon, &wp.label, wp.page_id);
        sidebar.append(&row);
        stack.add_named(&wp.page, Some(wp.page_id));
        wine_sidebar_rows.push(row);
    }

    let sep2 = super::settings_dialog::sidebar_separator();
    sidebar.append(&sep2);

    setup_wine_sidebar_visibility(&kind_row, &wine_sidebar_rows, &sep1, &sep2, &profile_row);

    let (env_page, env_vars_box, ld_preload_entry, ld_library_entry) = build_env_page();
    sidebar.append(&super::settings_dialog::settings_sidebar_row(
        "preferences-other-symbolic",
        &crate::tr!("Environment"),
        "env",
    ));
    stack.add_named(&env_page, Some("env"));

    let detect_group = adw::PreferencesGroup::new();
    detect_group.set_title(&crate::tr!("Quick detect"));
    let detect_row = adw::ActionRow::new();
    detect_row.set_title(&crate::tr!("Select game folder to auto-detect"));
    detect_row.add_suffix(&detect_btn);
    detect_group.add(&detect_row);
    general_page.append(&detect_group);

    connect_sidebar_selection(&sidebar, &stack);

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    let (cancel_btn, add_btn) = build_dialog_button_row(&content_area);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| {
        win_c.close();
    });

    // Natural height outgrew what a normal window offers as pages landed;
    // libadwaita warns and clips floating sheets that ask for more than
    // their presenter has.
    super::helpers::fit_dialog_height(&win, &window, 720);
    win.present(Some(&window));

    connect_add_handler(
        &add_btn,
        AddGameWidgets {
            name_entry: &name_entry,
            kind_row: &kind_row,
            folder_entry: &folder_entry,
            exe_entry: &exe_entry,
            args_entry: &args_entry,
            wd_entry: &wd_entry,
            profile_row: &profile_row,
            steam_id_entry: &steam_id_entry,
            gog_id_entry: &gog_id_entry,
            env_vars_box: &env_vars_box,
            ld_preload_entry: &ld_preload_entry,
            ld_library_entry: &ld_library_entry,
            wine_widgets: &wine_widgets,
            db: &db,
            sender: &sender,
            steam: &steam,
            save_dir: &save_dir,
            ra_username: &ra_username,
            ra_web_api_key: &ra_web_api_key,
            win: &win,
            state,
        },
    );
}

fn setup_wine_sidebar_visibility(
    kind_row: &adw::ComboRow,
    wine_sidebar_rows: &[gtk4::ListBoxRow],
    sep1: &gtk4::ListBoxRow,
    sep2: &gtk4::ListBoxRow,
    profile_row: &adw::ComboRow,
) {
    let rows = wine_sidebar_rows.to_vec();
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
    for r in wine_sidebar_rows {
        r.set_visible(visible);
    }
    sep1.set_visible(visible);
    sep2.set_visible(visible);
    profile_row.set_visible(visible);
}

fn connect_sidebar_selection(sidebar: &gtk4::ListBox, stack: &gtk4::Stack) {
    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let page_id = row.widget_name().to_string();
            stack_clone.set_visible_child_name(&page_id);
        }
    });
}
fn build_dialog_button_row(content_area: &gtk4::Box) -> (gtk4::Button, gtk4::Button) {
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(&crate::tr!("Cancel"));
    let add_btn = gtk4::Button::with_label(&crate::tr!("Add game"));
    add_btn.add_css_class(CSS_SUGGESTED_ACTION);

    btn_row.append(&cancel_btn);
    btn_row.append(&add_btn);
    content_area.append(&btn_row);

    (cancel_btn, add_btn)
}

struct AddGameWidgets<'a> {
    name_entry: &'a adw::EntryRow,
    kind_row: &'a adw::ComboRow,
    folder_entry: &'a adw::EntryRow,
    exe_entry: &'a adw::EntryRow,
    args_entry: &'a adw::EntryRow,
    wd_entry: &'a adw::EntryRow,
    profile_row: &'a adw::ComboRow,
    steam_id_entry: &'a adw::EntryRow,
    gog_id_entry: &'a adw::EntryRow,
    env_vars_box: &'a gtk4::ListBox,
    ld_preload_entry: &'a adw::EntryRow,
    ld_library_entry: &'a adw::EntryRow,
    wine_widgets: &'a WineConfigWidgets,
    db: &'a ira_db::DbConn,
    sender: &'a crate::AppSender,
    steam: &'a std::sync::Arc<ira_api::SteamDataClient>,
    save_dir: &'a str,
    ra_username: &'a str,
    ra_web_api_key: &'a str,
    win: &'a adw::Dialog,
    state: &'a SharedState,
}

fn connect_add_handler(add_btn: &gtk4::Button, widgets: AddGameWidgets<'_>) {
    let AddGameWidgets {
        name_entry,
        kind_row,
        folder_entry,
        exe_entry,
        args_entry,
        wd_entry,
        profile_row,
        steam_id_entry,
        gog_id_entry,
        env_vars_box,
        ld_preload_entry,
        ld_library_entry,
        wine_widgets,
        db,
        sender,
        steam,
        save_dir,
        ra_username,
        ra_web_api_key,
        win,
        state,
    } = widgets;

    let name_entry = name_entry.clone();
    let kind_row = kind_row.clone();
    let folder_entry = folder_entry.clone();
    let exe_entry = exe_entry.clone();
    let args_entry = args_entry.clone();
    let wd_entry = wd_entry.clone();
    let profile_row = profile_row.clone();
    let steam_id_entry = steam_id_entry.clone();
    let gog_id_entry = gog_id_entry.clone();
    let env_vars_box = env_vars_box.clone();
    let ld_preload_entry = ld_preload_entry.clone();
    let ld_library_entry = ld_library_entry.clone();
    let wine_widgets = wine_widgets.clone();
    let db = db.clone();
    let sender = sender.clone();
    let steam = steam.clone();
    let save_dir = save_dir.to_string();
    let ra_username = ra_username.to_string();
    let ra_web_api_key = ra_web_api_key.to_string();
    let win = win.clone();
    let state_c = state.clone();

    add_btn.connect_clicked(move |_| {
        name_entry.remove_css_class(CSS_ERROR);

        let name = name_entry.text().to_string();
        if name.is_empty() {
            name_entry.add_css_class(CSS_ERROR);
            return;
        }
        let exe_path = exe_entry.text().to_string();
        let game_folder = folder_entry.text().to_string();
        let is_wine = kind_row.selected() == 1;
        let args = args_entry.text().to_string();
        let wd = {
            let wd_text = wd_entry.text().to_string();
            if wd_text.is_empty() && !game_folder.is_empty() {
                game_folder.clone()
            } else {
                wd_text
            }
        };
        let steam_app_id = steam_id_entry.text().to_string();
        let gog_product_id = gog_id_entry.text().to_string();

        let trophy_source = if !steam_app_id.is_empty() {
            ira_models::TrophySource::Gse
        } else if !gog_product_id.is_empty() {
            ira_models::TrophySource::Nge
        } else {
            ira_models::TrophySource::Empty
        };
        let kind = if is_wine {
            ira_models::GameKind::Wine
        } else {
            ira_models::GameKind::Linux
        };
        let platform_id = if !steam_app_id.is_empty() {
            steam_app_id
        } else if !gog_product_id.is_empty() {
            gog_product_id
        } else {
            format!(
                "manual_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            )
        };

        let selected_profile_id = if is_wine {
            super::wine_profile_picker::selected_profile_id(&profile_row, &db)
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
            pre_launch: String::new(),
            overlay_enabled: None,
            ..Default::default()
        };
        let wine_config = if is_wine {
            wine_widgets.to_wine_config()
        } else {
            WineConfig::default()
        };

        let db_c = db.clone();
        let sender_c = sender.clone();
        let name_c = name;
        let app_id_c = platform_id.clone();
        let game_folder_c = game_folder;
        let kind_c = kind;
        let ts_c = trophy_source;
        let steam_c = steam.clone();
        let save_dir_c = save_dir.clone();
        let ra_username_c = ra_username.clone();
        let ra_web_api_key_c = ra_web_api_key.clone();

        std::thread::spawn(move || {
            match add_game_to_db(AddGameToDbParams {
                db: &db_c,
                name: &name_c,
                kind: kind_c,
                trophy_source: ts_c,
                app_id: &app_id_c,
                platform_id: &platform_id,
                game_folder: &game_folder_c,
                launch_config: &launch_config,
                wine_config: &wine_config,
                profile_id: selected_profile_id,
                steam: &steam_c,
                save_dir: &save_dir_c,
            }) {
                Ok(game_id) => {
                    let entry = ira_db::find_by_db_id(&db_c, game_id).ok().flatten();
                    if let Some(entry) = entry {
                        if let Ok(mut game) = crate::game_loader::load_game(&entry, &save_dir_c) {
                            game.set_name(&name_c);
                            game.game_path = launch_config.exe.clone();
                            if game.game_folder.is_empty() {
                                game.game_folder = game_folder_c.clone();
                            }
                            if let Err(e) = ira_db::update_game_title(&db_c, game.db_id, &name_c) {
                                eprintln!("Failed to update game title: {}", e);
                            }

                            // One-time migration of existing emulator saves to centralized path
                            let wine_prefix = if kind_c == ira_models::GameKind::Wine {
                                Some(ira_launcher::wine_launch::wine_prefix(&wine_config))
                            } else {
                                None
                            };
                            ira_platforms::api_emulators::migrate_emulator_saves(
                                &save_dir_c,
                                ts_c,
                                &app_id_c,
                                wine_prefix.as_deref(),
                            );

                            // Centralize game saves if UFS data is available
                            if let Some(details) =
                                crate::game_loader::read_app_details(&save_dir_c, &app_id_c)
                            {
                                if !details.ufs_savefiles.is_empty() {
                                    ira_launcher::game_saves::setup_game_saves(
                                        &details.ufs_savefiles,
                                        &details.ufs_rootoverrides,
                                        &app_id_c,
                                        &save_dir_c,
                                        wine_prefix.as_deref(),
                                    );
                                }
                            }

                            let _ = sender_c.send(AppMessage::NewGame(game.clone()));
                            let g_name = game.name.clone();
                            crate::ui::enrichment::enrich_game_async(
                                crate::ui::enrichment::EnrichGameParams {
                                    app_id: game.app_id.clone(),
                                    trophy_source: game.trophy_source,
                                    platform_id: game.platform_id.clone(),
                                    db_id: game.db_id,
                                    title: g_name,
                                    steam: steam_c,
                                    sender: sender_c,
                                    save_dir: save_dir_c,
                                    db: db_c,
                                    ra_username: ra_username_c,
                                    ra_web_api_key: ra_web_api_key_c,
                                    game: None,
                                },
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

pub(super) fn build_env_var_row(key: &str, value: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let key_entry = gtk4::Entry::new();
    key_entry.set_placeholder_text(Some(&crate::tr!("Variable name (e.g. FOO)")));
    key_entry.set_text(key);
    key_entry.set_hexpand(true);
    hbox.append(&key_entry);

    let val_entry = gtk4::Entry::new();
    val_entry.set_placeholder_text(Some(&crate::tr!("Value")));
    val_entry.set_text(value);
    val_entry.set_hexpand(true);
    hbox.append(&val_entry);

    let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove_btn.add_css_class(CSS_FLAT);
    remove_btn.add_css_class(CSS_CIRCULAR);
    let row_clone = row.clone();
    remove_btn.connect_clicked(move |_| {
        if let Some(list) = row_clone
            .parent()
            .and_then(|p| p.downcast::<gtk4::ListBox>().ok())
        {
            row_clone.unparent();
            list.remove(&row_clone);
        }
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
