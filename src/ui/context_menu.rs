use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use super::state::{SharedState, SAVE_DIR};
use super::helpers::open_folder;
use super::dialogs::show_game_settings_dialog;

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
    let folders_menu = gio::Menu::new();
    if game.kind == "gbe_steam" || game.kind == "sgdb" {
        folders_menu.append(Some("Image data"), Some("game.open_images"));
    }
    if game.kind == "gbe_steam" {
        folders_menu.append(Some("Achievement status"), Some("game.open_steam_status"));
    } else if game.kind == "ne_gog" {
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
        if lutris_id != 0 {
            let uri = format!("lutris:rungameid/{}", lutris_id);
            let rg = sc.borrow().running_games.clone();
            let sender = sc.borrow().sender.clone();
            match std::process::Command::new("lutris").arg(&uri).spawn() {
                Ok(child) => {
                    super::helpers::monitor_running_game(sender.clone(), rg.clone(), lutris_id, child);
                }
                Err(e) => {
                    eprintln!("Failed to launch {}: {}", uri, e);
                }
            }
        }
    });
    actions.add_action(&play_action);

    let edit_action = gio::SimpleAction::new("edit", None);
    let sc = state_clone.clone();
    let lid_for_edit = game_clone.lutris_id;
    edit_action.connect_activate(move |_, _| {
        let game = sc.borrow().games.iter().find(|g| g.lutris_id == lid_for_edit).cloned();
        if let Some(game) = game {
            show_game_settings_dialog(&sc, &game);
        }
    });
    actions.add_action(&edit_action);

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

    if game_clone.kind == "gbe_steam" || game_clone.kind == "sgdb" {
        let open_images = gio::SimpleAction::new("open_images", None);
        let gc = game_clone.clone();
        open_images.connect_activate(move |_, _| {
            let subdir = if gc.kind == "sgdb" { "steamgriddb" } else { "steam" };
            let path = format!("{}/data/{}/{}", SAVE_DIR, subdir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_images);
    }

    if game_clone.kind == "gbe_steam" {
        let open_status = gio::SimpleAction::new("open_steam_status", None);
        let gc = game_clone.clone();
        open_status.connect_activate(move |_, _| {
            let path = format!("{}/steam/{}", SAVE_DIR, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_status);
    }

    if game_clone.kind == "ne_gog" {
        let open_gog = gio::SimpleAction::new("open_gog_status", None);
        let gc = game_clone.clone();
        open_gog.connect_activate(move |_, _| {
            let path = format!("{}/gog/{}/{}", SAVE_DIR, crate::parser::GALAXY_ID, gc.platform_id);
            open_folder(&path);
        });
        actions.add_action(&open_gog);
    }

    parent.insert_action_group("game", Some(&actions));

    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk4::Rectangle::new(at_x as i32, at_y as i32, 1, 1)));
    popover.popup();
}
