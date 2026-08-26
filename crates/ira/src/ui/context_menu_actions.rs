use super::edit_game_dialog::show_edit_game_dialog;
use super::helpers::{open_file_location, open_folder};
use super::state::SharedState;
use crate::AppMessage;
use crate::Game;
use adw::prelude::*;

pub(super) fn setup_play_action(actions: &gio::SimpleActionGroup, state: SharedState, game: Game) {
    let play_action = gio::SimpleAction::new("play", None);
    play_action.connect_activate(move |_, _| {
        let db_id = game.db_id;
        let is_running = state
            .borrow()
            .running_games
            .lock()
            .unwrap()
            .contains_key(&db_id);
        if is_running {
            super::play_button::stop_game(&state, db_id);
        } else {
            match super::play_button::launch_game(&state, db_id, game.variant_id) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Failed to launch game: {}", e);
                    let _ = state.borrow().sender.send(AppMessage::AddGameError(e));
                }
            }
        }
    });
    actions.add_action(&play_action);
}

pub(super) fn setup_edit_action(actions: &gio::SimpleActionGroup, state: SharedState, db_id: i64) {
    let edit_action = gio::SimpleAction::new("edit", None);
    edit_action.connect_activate(move |_, _| {
        show_edit_game_dialog(&state, db_id);
    });
    actions.add_action(&edit_action);
}

pub(super) fn setup_controller_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let controller_action = gio::SimpleAction::new("controller", None);
    controller_action.connect_activate(move |_, _| {
        super::edit_game_controller::open_controller_settings(&state, &game);
    });
    actions.add_action(&controller_action);
}

pub(super) fn setup_play_history_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    db_id: i64,
    variant_id: Option<i64>,
) {
    let play_hist_action = gio::SimpleAction::new("play_history", None);
    play_hist_action.connect_activate(move |_, _| {
        super::play_history::show_play_history_dialog(&state, db_id, variant_id);
    });
    actions.add_action(&play_hist_action);
}

pub(super) fn setup_hide_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    db_id: i64,
    current_hidden: bool,
) {
    let hide_action = gio::SimpleAction::new("hide", None);
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        {
            let s = state.borrow();
            if let Some(g) = s.games.iter().find(|g| g.db_id == db_id) {
                if let Err(e) = ira_db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                    eprintln!("Failed to set hidden: {}", e);
                }
            }
        }
        if let Some(g) = state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == db_id)
        {
            g.hidden = new_hidden;
        }
        super::sidebar::rebuild_sidebar(&state);
        super::grid_view::refresh_grid_store(&state);
    });
    actions.add_action(&hide_action);
}

pub(super) fn setup_delete_game_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let delete_game_action = gio::SimpleAction::new("delete_game", None);
    delete_game_action.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let dialog = adw::AlertDialog::new(
            Some(&crate::tr!("Remove game?")),
            Some(&crate::tr!("Remove \"{}\"?").replacen("{}", &game.name, 1)),
        );
        dialog.add_response("cancel", &crate::tr!("Cancel"));
        dialog.add_response("delete", &crate::tr!("Remove"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let sc = state.clone();
        dialog.connect_response(None, move |_, resp| {
            if resp != "delete" {
                return;
            }
            let db = sc.borrow().db.clone();
            let db_id = game.db_id;
            if let Err(e) = ira_db::delete_game_config(&db, db_id) {
                eprintln!("Failed to delete game config: {}", e);
            }
            if let Err(e) = ira_db::remove_game(&db, db_id) {
                eprintln!("Failed to remove game: {}", e);
                return;
            }
            let mut s = sc.borrow_mut();
            s.games.retain(|g| g.db_id != db_id);
            let was_selected = ira_models::parse_db_id(&s.selected_id) == db_id;
            if was_selected {
                s.selected_id = String::new();
                s.selected_group = ira_models::GroupSelection::AllGames;
                drop(s);
                super::sidebar::rebuild_sidebar(&sc);
                super::message_helpers::clear_content(&sc);
            } else {
                drop(s);
                super::sidebar::rebuild_sidebar(&sc);
            }
        });
        dialog.present(Some(&window));
    });
    actions.add_action(&delete_game_action);
}

pub(super) fn setup_open_game_folder_action(
    actions: &gio::SimpleActionGroup,
    game_file: Option<String>,
    game_folder: Option<String>,
) {
    let open_game_folder = gio::SimpleAction::new("open_game_folder", None);
    open_game_folder.connect_activate(move |_, _| {
        if let Some(ref file) = game_file {
            open_file_location(file);
        } else if let Some(ref folder) = game_folder {
            open_folder(folder);
        }
    });
    actions.add_action(&open_game_folder);
}

pub(super) fn setup_open_wine_prefix_action(
    actions: &gio::SimpleActionGroup,
    wine_prefix: Option<String>,
) {
    if let Some(pfx) = wine_prefix {
        let open_wine_prefix = gio::SimpleAction::new("open_wine_prefix", None);
        open_wine_prefix.connect_activate(move |_, _| {
            open_folder(&pfx);
        });
        actions.add_action(&open_wine_prefix);
    }
}

pub(super) fn setup_open_images_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_images = gio::SimpleAction::new("open_images", None);
    let save_dir = state.borrow().save_dir.clone();
    open_images.connect_activate(move |_, _| {
        let path = ira_parser::game_data_dir(&save_dir, &game);
        open_folder(&path.to_string_lossy());
    });
    actions.add_action(&open_images);
}

pub(super) fn setup_open_save_location_action(actions: &gio::SimpleActionGroup, path: String) {
    let open_save = gio::SimpleAction::new("open_save_location", None);
    open_save.connect_activate(move |_, _| {
        open_folder(&path);
    });
    actions.add_action(&open_save);
}

/// The game's centralized save folder (`saves/<app_id>`), or `None` when the
/// game has no centralized save data yet. Emulator save folders (Gse/Nge) are
/// intentionally not covered here — their per-game folders are reachable via
/// the "Achievement status" menu items instead.
pub(super) fn centralized_save_path(save_dir: &str, game: &Game) -> Option<String> {
    let dir = ira_launcher::game_saves::centralized_save_dir(save_dir, &game.app_id);
    if dir.is_dir() && ira_launcher::game_saves::dir_has_save_data(&dir) {
        Some(dir.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub(super) fn setup_open_steam_status_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_status = gio::SimpleAction::new("open_steam_status", None);
    let save_dir = state.borrow().save_dir.clone();
    open_status.connect_activate(move |_, _| {
        let path = format!("{}/emulator_saves/gbe/{}", save_dir, game.app_id);
        open_folder(&path);
    });
    actions.add_action(&open_status);
}

pub(super) fn setup_open_gog_status_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let open_gog = gio::SimpleAction::new("open_gog_status", None);
    let save_dir = state.borrow().save_dir.clone();
    open_gog.connect_activate(move |_, _| {
        let path = format!(
            "{}/emulator_saves/nge/{}/{}",
            save_dir,
            ira_parser::GALAXY_ID,
            game.platform_id
        );
        open_folder(&path);
    });
    actions.add_action(&open_gog);
}

pub(super) fn setup_toggle_group_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = state.borrow().db.clone();
        let existing = ira_db::get_groups_for_game(&db, game.db_id).unwrap_or_default();
        if existing.iter().any(|g| g.id == group_id) {
            if let Err(e) = ira_db::remove_game_from_group(&db, game.db_id, group_id) {
                eprintln!("Failed to remove game from group: {}", e);
            }
            if let Some(members) = state.borrow_mut().group_members.get_mut(&group_id) {
                members.remove(&game.db_id);
            }
        } else {
            if let Err(e) = ira_db::add_game_to_group(&db, game.db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            }
            state
                .borrow_mut()
                .group_members
                .entry(group_id)
                .or_default()
                .insert(game.db_id);
        }
        super::sidebar::rebuild_sidebar(&state);
    });
    actions.add_action(&toggle_group);
}

pub(super) fn setup_new_collection_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    game: Game,
) {
    let new_collection = gio::SimpleAction::new("new_collection", None);
    new_collection.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let sc = state.clone();
        show_collection_name_dialog(window, sc, move |db, group_id| {
            if let Err(e) = ira_db::add_game_to_group(db, game.db_id, group_id) {
                eprintln!("Failed to add game to new group: {}", e);
            }
        });
    });
    actions.add_action(&new_collection);
}

pub(super) fn setup_multi_toggle_group_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let toggle_group = gio::SimpleAction::new("toggle_group", Some(&i64::static_variant_type()));
    toggle_group.connect_activate(move |_, param| {
        let group_id = param.and_then(|p| p.get::<i64>()).unwrap_or(0);
        let db = state.borrow().db.clone();

        let all_in = ids.iter().all(|&db_id| {
            let game_groups = ira_db::get_groups_for_game(&db, db_id).unwrap_or_default();
            game_groups.iter().any(|g| g.id == group_id)
        });

        for &db_id in &ids {
            if all_in {
                if let Err(e) = ira_db::remove_game_from_group(&db, db_id, group_id) {
                    eprintln!("Failed to remove game from group: {}", e);
                }
                if let Some(members) = state.borrow_mut().group_members.get_mut(&group_id) {
                    members.remove(&db_id);
                }
            } else if let Err(e) = ira_db::add_game_to_group(&db, db_id, group_id) {
                eprintln!("Failed to add game to group: {}", e);
            } else {
                state
                    .borrow_mut()
                    .group_members
                    .entry(group_id)
                    .or_default()
                    .insert(db_id);
            }
        }
        super::sidebar::rebuild_sidebar(&state);
    });
    actions.add_action(&toggle_group);
}

pub(super) fn setup_multi_new_collection_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
) {
    let new_collection = gio::SimpleAction::new("new_collection", None);
    new_collection.connect_activate(move |_, _| {
        let window = state.borrow().window.clone();
        let sc = state.clone();
        let ids = ids.clone();
        show_collection_name_dialog(window, sc, move |db, group_id| {
            for &db_id in &ids {
                if let Err(e) = ira_db::add_game_to_group(db, db_id, group_id) {
                    eprintln!("Failed to add game to new group: {}", e);
                }
            }
        });
    });
    actions.add_action(&new_collection);
}

pub(super) fn setup_multi_toggle_hide_action(
    actions: &gio::SimpleActionGroup,
    state: SharedState,
    ids: Vec<i64>,
    all_hidden: bool,
) {
    let toggle_hide = gio::SimpleAction::new("toggle_hide", None);
    toggle_hide.connect_activate(move |_, _| {
        let new_hidden = !all_hidden;
        let db = state.borrow().db.clone();
        for &db_id in &ids {
            if let Err(e) = ira_db::set_game_hidden(&db, db_id, new_hidden) {
                eprintln!("Failed to set hidden: {}", e);
            }
        }
        {
            let mut s = state.borrow_mut();
            for g in s.games.iter_mut() {
                if ids.contains(&g.db_id) {
                    g.hidden = new_hidden;
                }
            }
        }
        super::sidebar::rebuild_sidebar(&state);
        super::grid_view::refresh_grid_store(&state);
    });
    actions.add_action(&toggle_hide);
}

pub(super) fn show_collection_name_dialog(
    window: adw::ApplicationWindow,
    state: SharedState,
    add_games: impl Fn(&ira_db::DbConn, i64) + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some(&crate::tr!("New collection")),
        Some(&crate::tr!("Enter a name for the collection:")),
    );
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Collection name")));
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("create", &crate::tr!("Create"));
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry_clone = entry;
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
                let members = ira_db::get_game_ids_in_group(&db, group_id).unwrap_or_default();
                state.borrow_mut().groups = groups;
                state
                    .borrow_mut()
                    .group_members
                    .insert(group_id, members.into_iter().collect());
                super::sidebar::rebuild_sidebar(&state);
            }
            Err(e) => {
                eprintln!("Failed to create group: {}", e);
            }
        }
    });
    dialog.present(Some(&window));
}
