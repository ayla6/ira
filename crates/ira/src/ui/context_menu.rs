use std::collections::HashSet;

use super::context_menu_actions::{
    setup_controller_action, setup_delete_game_action, setup_edit_action, setup_hide_action,
    setup_multi_new_collection_action, setup_multi_toggle_group_action,
    setup_multi_toggle_hide_action, setup_new_collection_action,
    setup_open_game_folder_action, setup_open_gog_status_action, setup_open_images_action,
    setup_open_save_location_action, setup_open_steam_status_action, setup_open_wine_prefix_action,
    setup_play_action, setup_play_history_action, setup_run_manual_script_action,
    setup_toggle_group_action,
};
use super::state::SharedState;
use crate::Game;
use gtk4::prelude::*;

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
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk4::Rectangle::new(at_x as i32, at_y as i32, 1, 1)));
    parent.insert_action_group("game", Some(actions));
    let popover_clone = popover.clone();
    popover.connect_closed(move |_| {
        let p = popover_clone.clone();
        glib::idle_add_local_once(move || {
            p.unparent();
        });
    });
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
    collections_menu.append(
        Some(&crate::tr!("Add to new collection…")),
        Some("game.new_collection"),
    );
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

    let play_item = gio::MenuItem::new(Some(&crate::tr!("Play")), Some("game.play"));
    menu.prepend_item(&play_item);

    menu.append(Some(&crate::tr!("Edit game settings")), Some("game.edit"));
    menu.append(
        Some(&crate::tr!("Controller settings")),
        Some("game.controller"),
    );
    menu.append(
        Some(&crate::tr!("View play history")),
        Some("game.play_history"),
    );

    let folders_menu = gio::Menu::new();

    let (game_folder, game_file, wine_prefix, manual_script) = {
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
        let resolve_rom = |p: &str| -> String {
            if !matches!(
                game.kind,
                ira_models::GameKind::Retro | ira_models::GameKind::Switch
            ) || p.is_empty()
            {
                return p.to_string();
            }
            s.cfg
                .resolve_rom_path(&game.platform_id, p)
                .map(|resolved| resolved.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        };
        let resolved_game_path = resolve_rom(&game.game_path);
        let game_dir = if !launch.working_dir.is_empty() {
            Some(launch.working_dir)
        } else if !launch.exe.is_empty() {
            std::path::Path::new(&launch.exe)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
        } else if !resolved_game_path.is_empty() {
            std::path::Path::new(&resolved_game_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        let game_file =
            if !resolved_game_path.is_empty() && game.kind != ira_models::GameKind::Steam {
                Some(resolved_game_path)
            } else {
                None
            };
        let show_wine = game.kind == ira_models::GameKind::Wine && wine.enabled;
        (
            game_dir,
            game_file,
            if show_wine {
                Some(ira_launcher::wine_launch::wine_prefix(&wine))
            } else {
                None
            },
            launch.manual_script.clone(),
        )
    };

    if game_file.is_some() || game_folder.is_some() {
        folders_menu.append(
            Some(&crate::tr!("Game location")),
            Some("game.open_game_folder"),
        );
    }
    if wine_prefix.is_some() {
        folders_menu.append(
            Some(&crate::tr!("Wine prefix")),
            Some("game.open_wine_prefix"),
        );
    }
    folders_menu.append(Some(&crate::tr!("Data location")), Some("game.open_images"));
    let save_dir = state.borrow().save_dir.clone();
    let save_location = super::context_menu_actions::centralized_save_path(&save_dir, game);
    if save_location.is_some() {
        folders_menu.append(
            Some(&crate::tr!("Save location")),
            Some("game.open_save_location"),
        );
    }
    if game.trophy_source == ira_models::TrophySource::Gse {
        folders_menu.append(
            Some(&crate::tr!("Achievement status")),
            Some("game.open_steam_status"),
        );
    } else if game.trophy_source == ira_models::TrophySource::Nge {
        folders_menu.append(
            Some(&crate::tr!("Achievement status")),
            Some("game.open_gog_status"),
        );
    }
    if folders_menu.n_items() > 0 {
        menu.append_submenu(Some(&crate::tr!("Open folder")), &folders_menu);
    }

    let groups = state.borrow().groups.clone();
    let game_groups = {
        let db = state.borrow().db.clone();
        ira_db::get_groups_for_game(&db, game.db_id).unwrap_or_default()
    };
    let collections_menu =
        build_collections_submenu(&groups, |g| game_groups.iter().any(|gg| gg.id == g.id));
    menu.append_submenu(Some(&crate::tr!("Collections")), &collections_menu);

    if !manual_script.is_empty() {
        menu.append(
            Some(&crate::tr!("Run manual script")),
            Some("game.run_manual_script"),
        );
    }

    let hide_label = if current_hidden {
        crate::tr!("Unhide game")
    } else {
        crate::tr!("Hide game")
    };
    menu.append(Some(&hide_label), Some("game.hide"));

    let is_deletable = matches!(
        game.kind,
        ira_models::GameKind::Wine | ira_models::GameKind::Linux
    );
    if is_deletable {
        let remove_section = gio::Menu::new();
        remove_section.append(Some(&crate::tr!("Remove game")), Some("game.delete_game"));
        menu.append_section(None, &remove_section);
    }

    let actions = gio::SimpleActionGroup::new();

    setup_play_action(&actions, state.clone(), game.clone());
    setup_edit_action(&actions, state.clone(), game.db_id);
    setup_controller_action(&actions, state.clone(), game.clone());
    setup_play_history_action(&actions, state.clone(), game.db_id, game.variant_id);
    setup_hide_action(&actions, state.clone(), game.db_id, current_hidden);
    if !manual_script.is_empty() {
        setup_run_manual_script_action(
            &actions,
            game.clone(),
            manual_script,
            game_folder.clone(),
        );
    }
    if is_deletable {
        setup_delete_game_action(&actions, state.clone(), game.clone());
    }
    if game_file.is_some() || game_folder.is_some() {
        setup_open_game_folder_action(&actions, game_file, game_folder);
    }
    setup_open_wine_prefix_action(&actions, wine_prefix);
    setup_open_images_action(&actions, state.clone(), game.clone());
    if let Some(save_location) = save_location {
        setup_open_save_location_action(&actions, save_location);
    }
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
            game_group_map
                .get(&db_id)
                .is_some_and(|ids| ids.contains(&g.id))
        })
    });
    menu.append_submenu(Some(&crate::tr!("Collections")), &collections_menu);

    let all_hidden = db_ids.iter().all(|&db_id| {
        state
            .borrow()
            .games
            .iter()
            .find(|g| g.db_id == db_id)
            .is_some_and(|g| g.hidden)
    });
    let hide_label = if all_hidden {
        crate::tr!("Unhide game")
    } else {
        crate::tr!("Hide game")
    };
    let hide_section = gio::Menu::new();
    hide_section.append(Some(&hide_label), Some("game.toggle_hide"));
    menu.append_section(None, &hide_section);

    let ids: Vec<i64> = db_ids.iter().copied().collect();
    let actions = gio::SimpleActionGroup::new();

    setup_multi_toggle_group_action(&actions, state.clone(), ids.clone());
    setup_multi_new_collection_action(&actions, state.clone(), ids.clone());
    setup_multi_toggle_hide_action(&actions, state.clone(), ids, all_hidden);

    setup_and_show_popover(&menu, &actions, parent, at_x, at_y);
}


/// Shift+right-click on grid covers: a small menu to reset (delete) any of
/// the game's stored art files, so the fallback — or a fresh auto
/// download — takes over.
pub fn show_image_reset_menu(
    state: &SharedState,
    game: &Game,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    at_x: f64,
    at_y: f64,
) {
    let (db, save_dir) = {
        let s = state.borrow();
        (s.db.clone(), s.save_dir.clone())
    };
    let Ok(Some(entry)) = ira_db::find_by_db_id(&db, game.db_id) else {
        return;
    };
    let image_dir = ira_parser::entry_data_dir(&save_dir, &entry);

    let candidates: [(String, &str); 6] = [
        (crate::tr!("Icon"), "icon"),
        (crate::tr!("Capsule"), "vertical"),
        (crate::tr!("Square"), "square"),
        (crate::tr!("Header"), "header"),
        (crate::tr!("Logo"), "logo"),
        (crate::tr!("Hero"), "hero"),
    ];
    let existing: Vec<(String, &str)> = candidates
        .iter()
        .filter(|(_, base)| ira_parser::find_image_file(&image_dir, base).is_some())
        .map(|(label, base)| (label.clone(), *base))
        .collect();
    if existing.is_empty() {
        return;
    }

    // Standard menu styling, same as the game context menu.
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    for (label, base) in &existing {
        section.append(Some(label), Some(&format!("image.reset('{base}')")));
    }
    menu.append_section(Some(&crate::tr!("Reset image")), &section);

    let actions = gio::SimpleActionGroup::new();
    let reset = gio::SimpleAction::new("reset", Some(glib::VariantTy::STRING));
    let state_c = state.clone();
    let game_c = game.clone();
    reset.connect_activate(move |_, parameter| {
        if let Some(base) = parameter.and_then(|v| v.str()) {
            reset_game_image(&state_c, &game_c, base);
        }
    });
    actions.add_action(&reset);

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_halign(gtk4::Align::Start);
    popover.set_has_arrow(false);
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk4::Rectangle::new(
        at_x as i32,
        at_y as i32,
        1,
        1,
    )));
    parent.insert_action_group("image", Some(&actions));
    let popover_clone = popover.clone();
    popover.connect_closed(move |_| {
        let p = popover_clone.clone();
        glib::idle_add_local_once(move || {
            p.unparent();
        });
    });
    popover.popup();
}

/// Delete one stored art file from the game's data dir and update the
/// shared state so every view falls back immediately.
fn reset_game_image(state: &SharedState, game: &Game, base: &str) {
    let (db, save_dir) = {
        let s = state.borrow();
        (s.db.clone(), s.save_dir.clone())
    };
    let Ok(Some(entry)) = ira_db::find_by_db_id(&db, game.db_id) else {
        return;
    };
    let dir = ira_parser::entry_data_dir(&save_dir, &entry);
    ira_parser::remove_image_variants(&dir, base);
    ira_parser::remove_image_variants(&dir, &format!("{base}_small"));
    if let Some(path) = ira_parser::find_image_file(&dir, base) {
        ira_images::invalidate_texture(&path.to_string_lossy());
    }

    let mut updated = game.clone();
    match base {
        "icon" => updated.icon_path.clear(),
        "hero" => updated.hero_image_path.clear(),
        "vertical" => updated.grid_path.clear(),
        "square" => updated.square_path.clear(),
        "header" => updated.header_path.clear(),
        "logo" => updated.logo_path.clear(),
        _ => {}
    }

    {
        let mut s = state.borrow_mut();
        for g in s
            .games
            .iter_mut()
            .filter(|g| g.db_id == game.db_id && g.variant_id == game.variant_id)
        {
            match base {
                "icon" => g.icon_path.clear(),
                "hero" => g.hero_image_path.clear(),
                "vertical" => g.grid_path.clear(),
                "square" => g.square_path.clear(),
                "header" => g.header_path.clear(),
                "logo" => g.logo_path.clear(),
                _ => {}
            }
        }
    }
    super::helpers::replace_grid_game(state, &updated);
    super::big_picture_view::refresh(state);
}

