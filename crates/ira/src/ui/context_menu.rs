use std::collections::HashSet;

use gtk4::prelude::*;
use adw::prelude::{AlertDialogExt, AdwDialogExt};
use crate::Game;
use crate::strings as S;
use super::state::SharedState;
use super::context_menu_actions::*;

pub(super) fn show_collection_name_dialog(
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

    let actions = gio::SimpleActionGroup::new();

    setup_play_action(&actions, state.clone(), game.clone());
    setup_edit_action(&actions, state.clone(), game.db_id);
    setup_play_history_action(&actions, state.clone(), game.db_id, game.variant_id);
    setup_hide_action(&actions, state.clone(), game.db_id, current_hidden);
    if is_deletable {
        setup_delete_game_action(&actions, state.clone(), game.clone());
    }
    if game_file.is_some() || game_folder.is_some() {
        setup_open_game_folder_action(&actions, game_file.clone(), game_folder.clone());
    }
    setup_open_wine_prefix_action(&actions, wine_prefix.clone());
    setup_open_images_action(&actions, state.clone(), game.clone());
    if game.trophy_source == ira_models::TrophySource::Gse {
        setup_open_steam_status_action(&actions, state.clone(), game.clone());
    }
    if game.trophy_source == ira_models::TrophySource::Nge {
        setup_open_gog_status_action(&actions, state.clone(), game.clone());
    }
    setup_toggle_group_action(&actions, state.clone(), game.clone());
    setup_new_collection_action(&actions, state.clone(), game.clone());

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

    let ids: Vec<i64> = db_ids.iter().copied().collect();
    let actions = gio::SimpleActionGroup::new();

    setup_multi_toggle_group_action(&actions, state.clone(), ids.clone());
    setup_multi_new_collection_action(&actions, state.clone(), ids.clone());
    setup_multi_toggle_hide_action(&actions, state.clone(), ids.clone(), all_hidden);

    setup_and_show_popover(&menu, &actions, parent, at_x, at_y);
}
