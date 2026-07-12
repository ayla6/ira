use gtk4::prelude::*;
use crate::Game;
use crate::AppMessage;
use crate::strings as S;
use super::state::SharedState;
use super::helpers::open_folder;
use super::edit_game_dialog::show_edit_game_dialog;

pub fn show_game_context_menu(
    state: &SharedState,
    game: &Game,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    at_x: f64,
    at_y: f64,
    sidebar_row: Option<&gtk4::ListBoxRow>,
) {
    let current_hidden = state
        .borrow()
        .games
        .iter()
        .find(|g| g.lutris_id == game.lutris_id)
        .map(|g| g.hidden)
        .unwrap_or(game.hidden);

    let menu = gio::Menu::new();

    let play_item = gio::MenuItem::new(Some("Play"), Some("game.play"));
    menu.prepend_item(&play_item);

    menu.append(Some(S::EDIT_GAME_SETTINGS), Some("game.edit"));
    menu.append(Some(S::VIEW_PLAY_HISTORY), Some("game.play_history"));

    let folders_menu = gio::Menu::new();

    let (game_folder, wine_prefix) = {
        let s = state.borrow();
        let config = crate::db::get_game_config(&s.db, game.db_id).ok().flatten();
        let app_default = s.cfg.default_wine_config.clone();
        let (launch, mut wine, _) = config.unwrap_or_default();
        wine = wine.merge_with_default(&app_default);
        let game_dir = if !launch.working_dir.is_empty() {
            Some(launch.working_dir)
        } else if !launch.exe.is_empty() {
            std::path::Path::new(&launch.exe).parent().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        (game_dir, if wine.enabled { Some(crate::launcher::wine_launch::wine_prefix(&wine)) } else { None })
    };

    if game_folder.is_some() {
        folders_menu.append(Some("Game folder"), Some("game.open_game_folder"));
    }
    if wine_prefix.is_some() {
        folders_menu.append(Some("Wine prefix"), Some("game.open_wine_prefix"));
    }
    if crate::models::has_steam_enrichment(&game.trophy_source) || !game.sgdb_id.is_empty() {
        folders_menu.append(Some("Image data"), Some("game.open_images"));
    }
    if game.trophy_source == crate::models::GSE {
        folders_menu.append(Some("Achievement status"), Some("game.open_steam_status"));
    } else if game.trophy_source == crate::models::NGE {
        folders_menu.append(Some("Achievement status"), Some("game.open_gog_status"));
    }
    if folders_menu.n_items() > 0 {
        menu.append_submenu(Some("Open folder"), &folders_menu);
    }
    menu.append(Some(if current_hidden { S::UNHIDE_GAME } else { S::HIDE_GAME }), Some("game.hide"));

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_halign(gtk4::Align::Start);
    popover.set_has_arrow(false);

    let state_clone = state.clone();
    let game_clone = game.clone();
    let actions = gio::SimpleActionGroup::new();

    let play_action = gio::SimpleAction::new("play", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    play_action.connect_activate(move |_, _| {
        let lutris_id = gc.lutris_id;
        let is_running = sc.borrow().running_games.lock().unwrap().contains_key(&lutris_id);
        if is_running {
            super::play_button::stop_game(&sc, lutris_id);
        } else {
            match super::play_button::launch_game(&sc, lutris_id, None) {
                Ok(()) => {
                    let _ = sc.borrow().sender.send(AppMessage::GameStarted(lutris_id));
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
    let row = sidebar_row.map(|r| r.clone());
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        let lutris_id = gc.lutris_id;
        {
            let s = sc.borrow();
            if let Some(g) = s.games.iter().find(|g| g.lutris_id == lutris_id) {
                if g.db_id != 0 {
                    if let Err(e) = crate::db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                        eprintln!("Failed to set hidden: {}", e);
                    }
                } else if lutris_id != 0 {
                    if let Err(e) = crate::db::set_lutris_hidden(&s.db, lutris_id, new_hidden) {
                        eprintln!("Failed to set lutris hidden: {}", e);
                    }
                }
            }
        }
        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.hidden = new_hidden;
        }
        if let Some(ref row_clone) = row {
            let scroll = sc.borrow().sidebar_scroll.clone();
            let saved_scroll = scroll.vadjustment().value();
            if new_hidden {
                row_clone.add_css_class("hidden-game");
            } else {
                row_clone.remove_css_class("hidden-game");
            }
            let show_hidden = sc.borrow().cfg.show_hidden_games;
            row_clone.set_visible(!new_hidden || show_hidden);
            let adj = scroll.vadjustment();
            let max = (adj.upper() - adj.page_size()).max(0.0);
            adj.set_value(saved_scroll.min(max));
        }
    });
    actions.add_action(&hide_action);

    if let Some(ref gf) = game_folder {
        let open_game_folder = gio::SimpleAction::new("open_game_folder", None);
        let path = gf.clone();
        open_game_folder.connect_activate(move |_, _| {
            open_folder(&path);
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

    if crate::models::has_steam_enrichment(&game_clone.trophy_source) || !game_clone.sgdb_id.is_empty() {
        let open_images = gio::SimpleAction::new("open_images", None);
        let gc = game_clone.clone();
        let save_dir = state_clone.borrow().save_dir.clone();
        open_images.connect_activate(move |_, _| {
            let subdir = if !gc.sgdb_id.is_empty() { "steamgriddb" } else { "steam" };
            let path = format!("{}/data/{}/{}", save_dir, subdir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_images);
    }

    if game_clone.trophy_source == crate::models::GSE {
        let open_status = gio::SimpleAction::new("open_steam_status", None);
        let gc = game_clone.clone();
        let save_dir = state_clone.borrow().save_dir.clone();
        open_status.connect_activate(move |_, _| {
            let path = format!("{}/steam/{}", save_dir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_status);
    }

    if game_clone.trophy_source == crate::models::NGE {
        let open_gog = gio::SimpleAction::new("open_gog_status", None);
        let gc = game_clone.clone();
        let save_dir = state_clone.borrow().save_dir.clone();
        open_gog.connect_activate(move |_, _| {
            let path = format!("{}/gog/{}/{}", save_dir, crate::parser::GALAXY_ID, gc.platform_id);
            open_folder(&path);
        });
        actions.add_action(&open_gog);
    }

    parent.insert_action_group("game", Some(&actions));

    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk4::Rectangle::new(at_x as i32, at_y as i32, 1, 1)));
    popover.popup();
}
