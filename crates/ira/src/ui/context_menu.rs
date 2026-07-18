use std::collections::HashSet;

use gtk4::prelude::*;
use adw::prelude::{AlertDialogExt, AdwDialogExt};
use ira_models::GameKind;
use ira_parser::{data_dir, ps4_data_dir, sgdb_data_dir, retro_data_dir};
use crate::Game;
use crate::AppMessage;
use crate::strings as S;
use super::state::SharedState;
use super::helpers::open_folder;
use super::helpers::open_file_location;
use super::edit_game_dialog::show_edit_game_dialog;

fn show_collection_name_dialog(
    window: adw::ApplicationWindow,
    state: SharedState,
    add_games: impl Fn(&ira_db::DbConn, i64) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some("New Collection"), Some("Enter a name for the collection:"));
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Collection name"));
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry_clone = entry.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "create" {
            return;
        }
        let name = entry_clone.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let db = state.borrow().db.clone();
        match ira_db::create_group(&db, &name) {
            Ok(group_id) => {
                add_games(&db, group_id);
                let groups = ira_db::get_all_groups(&db).unwrap_or_default();
                state.borrow_mut().groups = groups;
                super::sidebar::rebuild_sidebar(&state);
            }
            Err(e) => {
                eprintln!("Failed to create group: {}", e);
            }
        }
    });
    dialog.present(Some(&window));
}

fn setup_and_show_popover(
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    at_x: f64,
    at_y: f64,
) {
    let popover = gtk4::PopoverMenu::from_model(Some(menu));
    popover.set_halign(gtk4::Align::Start);
    popover.set_has_arrow(false);
    parent.insert_action_group("game", Some(actions));
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk4::Rectangle::new(at_x as i32, at_y as i32, 1, 1)));
    popover.popup();
}

fn build_collections_submenu(
    groups: &[ira_models::Group],
    is_checked: impl Fn(&ira_models::Group) -> bool,
) -> gio::Menu {
    let collections_menu = gio::Menu::new();
    for g in groups {
        let label = if is_checked(g) {
            format!("✓ {}", g.name)
        } else {
            g.name.clone()
        };
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("game.toggle_group"), Some(&g.id.to_variant()));
        collections_menu.append_item(&item);
    }
    if !groups.is_empty() {
        collections_menu.append_section(None, &gio::Menu::new());
    }
    collections_menu.append(Some("Add to new collection…"), Some("game.new_collection"));
    collections_menu
}

pub fn show_game_context_menu(
    state: &SharedState,
    game: &Game,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    at_x: f64,
    at_y: f64,
    _sidebar_row: Option<&gtk4::ListBoxRow>,
) {
    let current_hidden = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == game.db_id)
        .map(|g| g.hidden)
        .unwrap_or(game.hidden);

    let menu = gio::Menu::new();

    let play_item = gio::MenuItem::new(Some("Play"), Some("game.play"));
    menu.prepend_item(&play_item);

    menu.append(Some(S::EDIT_GAME_SETTINGS), Some("game.edit"));
    menu.append(Some(S::VIEW_PLAY_HISTORY), Some("game.play_history"));

    let folders_menu = gio::Menu::new();

    let (game_folder, game_file, wine_prefix) = {
        let s = state.borrow();
        let config = ira_db::get_game_config(&s.db, game.db_id).ok().flatten();
        let app_default = s.cfg.default_wine_config.clone();
        let (launch, mut wine, profile_id) = config.unwrap_or_default();
        if let Some(pid) = profile_id {
            if let Ok(Some(profile)) = ira_db::get_profile(&s.db, pid) {
                wine.version = profile.wine_version;
                wine.custom_wine_path = profile.custom_wine_path;
                wine.prefix = profile.prefix;
                wine.arch = profile.arch;
            }
        }
        wine = wine.merge_with_default(&app_default);
        let game_dir = if !launch.working_dir.is_empty() {
            Some(launch.working_dir)
        } else if !launch.exe.is_empty() {
            std::path::Path::new(&launch.exe).parent().map(|p| p.to_string_lossy().to_string())
        } else if !game.game_path.is_empty() {
            std::path::Path::new(&game.game_path).parent().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        let game_file = if !game.game_path.is_empty() && game.kind != ira_models::GameKind::Steam {
            Some(game.game_path.clone())
        } else {
            None
        };
        let show_wine = game.kind == ira_models::GameKind::Wine && wine.enabled;
        (game_dir, game_file, if show_wine { Some(ira_launcher::wine_launch::wine_prefix(&wine)) } else { None })
    };

    if game_file.is_some() || game_folder.is_some() {
        folders_menu.append(Some("Game location"), Some("game.open_game_folder"));
    }
    if wine_prefix.is_some() {
        folders_menu.append(Some("Wine prefix"), Some("game.open_wine_prefix"));
    }
    folders_menu.append(Some("Image data"), Some("game.open_images"));
    if game.trophy_source == ira_models::TrophySource::Gse {
        folders_menu.append(Some("Achievement status"), Some("game.open_steam_status"));
    } else if game.trophy_source == ira_models::TrophySource::Nge {
        folders_menu.append(Some("Achievement status"), Some("game.open_gog_status"));
    }
    if folders_menu.n_items() > 0 {
        menu.append_submenu(Some("Open folder"), &folders_menu);
    }

    let groups = state.borrow().groups.clone();
    let game_groups = {
        let db = state.borrow().db.clone();
        ira_db::get_groups_for_game(&db, game.db_id).unwrap_or_default()
    };
    let collections_menu = build_collections_submenu(&groups, |g| {
        game_groups.iter().any(|gg| gg.id == g.id)
    });
    menu.append_submenu(Some("Collections"), &collections_menu);

    menu.append(Some(if current_hidden { S::UNHIDE_GAME } else { S::HIDE_GAME }), Some("game.hide"));

    let is_deletable = matches!(game.kind, ira_models::GameKind::Wine | ira_models::GameKind::Linux);
    if is_deletable {
        let remove_section = gio::Menu::new();
        remove_section.append(Some(S::REMOVE_GAME), Some("game.delete_game"));
        menu.append_section(None, &remove_section);
    }

    let state_clone = state.clone();
    let game_clone = game.clone();
    let actions = gio::SimpleActionGroup::new();

    let play_action = gio::SimpleAction::new("play", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    play_action.connect_activate(move |_, _| {
        let db_id = gc.db_id;
        let is_running = sc.borrow().running_games.lock().unwrap().contains_key(&db_id);
        if is_running {
            super::play_button::stop_game(&sc, db_id);
        } else {
            match super::play_button::launch_game(&sc, db_id, None) {
                Ok(()) => {
                    let _ = sc.borrow().sender.send(AppMessage::GameStarted(db_id));
                }
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = sc.borrow().sender.send(AppMessage::AddGameError(e));
                }
            }
        }
    });
    actions.add_action(&play_action);

    let edit_action = gio::SimpleAction::new("edit", None);
    let sc = state_clone.clone();
    let db_id_for_edit = game_clone.db_id;
    edit_action.connect_activate(move |_, _| {
        show_edit_game_dialog(&sc, db_id_for_edit);
    });
    actions.add_action(&edit_action);

    let play_hist_action = gio::SimpleAction::new("play_history", None);
    let sc = state_clone.clone();
    let db_id_for_hist = game_clone.db_id;
    play_hist_action.connect_activate(move |_, _| {
        super::play_history::show_play_history_dialog(&sc, db_id_for_hist);
    });
    actions.add_action(&play_hist_action);

    let hide_action = gio::SimpleAction::new("hide", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        let db_id = gc.db_id;
        {
            let s = sc.borrow();
            if let Some(g) = s.games.iter().find(|g| g.db_id == db_id) {
                if let Err(e) = ira_db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                    eprintln!("Failed to set hidden: {}", e);
                }
            }
        }
        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
            g.hidden = new_hidden;
        }
        super::sidebar::rebuild_sidebar(&sc);
    });
    actions.add_action(&hide_action);

    if is_deletable {
        let delete_game_action = gio::SimpleAction::new("delete_game", None);
        let sc = state_clone.clone();
        let gc = game_clone.clone();
        delete_game_action.connect_activate(move |_, _| {
            let window = sc.borrow().window.clone();
            let dialog = adw::AlertDialog::new(
                Some(S::REMOVE_GAME_QUESTION),
                Some(&format!("Remove \"{}\"?", gc.name)),
            );
            dialog.add_response("cancel", S::CANCEL);
            dialog.add_response("delete", "Remove");
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let sc = sc.clone();
            let gc = gc.clone();
            dialog.connect_response(None, move |_, resp| {
                if resp != "delete" {
                    return;
                }
                let db = sc.borrow().db.clone();
                let db_id = gc.db_id;
                if let Err(e) = ira_db::delete_game_config(&db, db_id) {
                    eprintln!("Failed to delete game config: {}", e);
                }
                if let Err(e) = ira_db::remove_game(&db, db_id) {
                    eprintln!("Failed to remove game: {}", e);
                    return;
                }
                let mut s = sc.borrow_mut();
                s.games.retain(|g| g.db_id != db_id);
                let was_selected = s.selected_id == db_id.to_string();
                if was_selected {
                    s.selected_id = String::new();
                    s.selected_group = ira_models::GroupSelection::AllGames;
                    drop(s);
                    super::sidebar::rebuild_sidebar(&sc);
                    super::message_handler::clear_content(&sc);
                } else {
                    drop(s);
                    super::sidebar::rebuild_sidebar(&sc);
                }
            });
            dialog.present(Some(&window));
        });
        actions.add_action(&delete_game_action);
    }

    if game_file.is_some() || game_folder.is_some() {
        let open_game_folder = gio::SimpleAction::new("open_game_folder", None);
        let file_path = game_file.clone();
        let folder_path = game_folder.clone();
        open_game_folder.connect_activate(move |_, _| {
            if let Some(ref file) = file_path {
                open_file_location(file);
            } else if let Some(ref folder) = folder_path {
                open_folder(folder);
            }
        });
        actions.add_action(&open_game_folder);
    }

    if let Some(ref pfx) = wine_prefix {
        let open_wine_prefix = gio::SimpleAction::new("open_wine_prefix", None);
        let path = pfx.clone();
        open_wine_prefix.connect_activate(move |_, _| {
            open_folder(&path);
        });
        actions.add_action(&open_wine_prefix);
    }

    let open_images = gio::SimpleAction::new("open_images", None);
    let gc = game_clone.clone();
    let save_dir = state_clone.borrow().save_dir.clone();
    open_images.connect_activate(move |_, _| {
        let path = if gc.kind == GameKind::Retro {
            retro_data_dir(&save_dir, gc.db_id)
        } else if gc.kind == GameKind::Ps4 {
            ps4_data_dir(&save_dir, &gc.app_id)
        } else if gc.trophy_source.has_steam_enrichment() {
            data_dir(&save_dir, &gc.app_id)
        } else if !gc.sgdb_id.is_empty() {
            sgdb_data_dir(&save_dir, &gc.sgdb_id)
        } else {
            data_dir(&save_dir, &gc.app_id)
        };
        open_folder(&path.to_string_lossy());
    });
    actions.add_action(&open_images);

    if game_clone.trophy_source == ira_models::TrophySource::Gse {
        let open_status = gio::SimpleAction::new("open_steam_status", None);
        let gc = game_clone.clone();
        let save_dir = state_clone.borrow().save_dir.clone();
        open_status.connect_activate(move |_, _| {
            let path = format!("{}/steam/{}", save_dir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_status);
    }

    if game_clone.trophy_source == ira_models::TrophySource::Nge {
        let open_gog = gio::SimpleAction::new("open_gog_status", None);
        let gc = game_clone.clone();
        let save_dir = state_clone.borrow().save_dir.clone();
        open_gog.connect_activate(move |_, _| {
            let path = format!("{}/gog/{}/{}", save_dir, ira_parser::GALAXY_ID, gc.platform_id);
            open_folder(&path);
        });
        actions.add_action(&open_gog);
    }

    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = sc.borrow().db.clone();
        let existing = ira_db::get_groups_for_game(&db, gc.db_id).unwrap_or_default();
        if existing.iter().any(|g| g.id == group_id) {
            if let Err(e) = ira_db::remove_game_from_group(&db, gc.db_id, group_id) {
                eprintln!("Failed to remove game from group: {}", e);
            }
        } else {
            if let Err(e) = ira_db::add_game_to_group(&db, gc.db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            }
        }
        super::sidebar::rebuild_sidebar(&sc);
    });
    actions.add_action(&toggle_group);

    let new_collection = gio::SimpleAction::new("new_collection", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    new_collection.connect_activate(move |_, _| {
        let window = sc.borrow().window.clone();
        let sc2 = sc.clone();
        show_collection_name_dialog(window, sc2, move |db, group_id| {
            if let Err(e) = ira_db::add_game_to_group(db, gc.db_id, group_id) {
                eprintln!("Failed to add game to new group: {}", e);
            }
        });
    });
    actions.add_action(&new_collection);

    setup_and_show_popover(&menu, &actions, parent, at_x, at_y);
}

pub fn show_multi_game_context_menu(
    state: &SharedState,
    db_ids: &HashSet<i64>,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    at_x: f64,
    at_y: f64,
) {
    let menu = gio::Menu::new();

    let groups = state.borrow().groups.clone();
    let db = state.borrow().db.clone();

    let game_group_map: std::collections::HashMap<i64, Vec<i64>> = db_ids
        .iter()
        .map(|&db_id| {
            let group_ids = ira_db::get_groups_for_game(&db, db_id)
                .unwrap_or_default()
                .iter()
                .map(|g| g.id)
                .collect();
            (db_id, group_ids)
        })
        .collect();

    let collections_menu = build_collections_submenu(&groups, |g| {
        db_ids.iter().all(|&db_id| {
            game_group_map.get(&db_id).is_some_and(|ids| ids.contains(&g.id))
        })
    });
    menu.append_submenu(Some("Collections"), &collections_menu);

    let all_hidden = db_ids.iter().all(|&db_id| {
        state.borrow().games.iter()
            .find(|g| g.db_id == db_id)
            .is_some_and(|g| g.hidden)
    });
    let hide_label = if all_hidden { S::UNHIDE_GAME } else { S::HIDE_GAME };
    let hide_section = gio::Menu::new();
    hide_section.append(Some(hide_label), Some("game.toggle_hide"));
    menu.append_section(None, &hide_section);

    let sc_orig = state.clone();
    let ids: Vec<i64> = db_ids.iter().copied().collect();
    let actions = gio::SimpleActionGroup::new();

    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    let sc = sc_orig.clone();
    let ids_clone = ids.clone();
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = sc.borrow().db.clone();

        let all_in = ids_clone.iter().all(|&db_id| {
            let game_groups = ira_db::get_groups_for_game(&db, db_id).unwrap_or_default();
            game_groups.iter().any(|g| g.id == group_id)
        });

        for &db_id in &ids_clone {
            if all_in {
                if let Err(e) = ira_db::remove_game_from_group(&db, db_id, group_id) {
                    eprintln!("Failed to remove game from group: {}", e);
                }
            } else if let Err(e) = ira_db::add_game_to_group(&db, db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            }
        }
        super::sidebar::rebuild_sidebar(&sc);
    });
    actions.add_action(&toggle_group);

    let new_collection = gio::SimpleAction::new("new_collection", None);
    let sc = sc_orig.clone();
    let ids_clone2 = ids.clone();
    new_collection.connect_activate(move |_, _| {
        let window = sc.borrow().window.clone();
        let sc2 = sc.clone();
        let ids3 = ids_clone2.clone();
        show_collection_name_dialog(window, sc2, move |db, group_id| {
            for &db_id in &ids3 {
                if let Err(e) = ira_db::add_game_to_group(db, db_id, group_id) {
                    eprintln!("Failed to add game to new group: {}", e);
                }
            }
        });
    });
    actions.add_action(&new_collection);

    let toggle_hide = gio::SimpleAction::new("toggle_hide", None);
    let sc = sc_orig.clone();
    let ids_clone4 = ids.clone();
    let all_hidden_clone = all_hidden;
    toggle_hide.connect_activate(move |_, _| {
        let new_hidden = !all_hidden_clone;
        let db = sc.borrow().db.clone();
        for &db_id in &ids_clone4 {
            if let Err(e) = ira_db::set_game_hidden(&db, db_id, new_hidden) {
                eprintln!("Failed to set hidden: {}", e);
            }
        }
        {
            let mut s = sc.borrow_mut();
            for g in s.games.iter_mut() {
                if ids_clone4.contains(&g.db_id) {
                    g.hidden = new_hidden;
                }
            }
        }
        super::sidebar::rebuild_sidebar(&sc);
    });
    actions.add_action(&toggle_hide);

    setup_and_show_popover(&menu, &actions, parent, at_x, at_y);
}
