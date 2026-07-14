use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::Game;
use crate::images;
use crate::strings as S;
use std::collections::{HashMap, HashSet};
use super::state::SharedState;
use super::sidebar::{select_row_silently, rebuild_sidebar, apply_selected_highlight};
use super::grid_view::show_grid_view;
use super::game_display::display_game;
use super::helpers::{merge_game_enrichment, clear_children};
use super::enrichment::enrich_game_async;
use super::dialogs::build_image_manager_content;

pub fn handle_app_message(state: &SharedState, msg: AppMessage) {
    match msg {
        AppMessage::EnrichedGame(game) | AppMessage::WatcherGameUpdated(game) => {
            apply_game_update(state, game);
        }
        AppMessage::NewGame(game) => {
            insert_or_update_game(state, game);
        }
        AppMessage::AddGameError(e) => {
            let window = state.borrow().window.clone();
            let dialog = adw::AlertDialog::new(
                Some(S::COULDNT_ADD_GAME),
                Some(&e),
            );
            dialog.add_response("ok", S::OK);
            dialog.set_default_response(Some("ok"));
            dialog.set_close_response("ok");
            dialog.present(Some(&window));
        }
        AppMessage::GameRemoved { app_id } => {
            let mut s = state.borrow_mut();
            s.games.retain(|g| g.app_id != app_id);
            s.game_names.lock().unwrap().remove(&app_id);
            drop(s);
            rebuild_sidebar(state);
        }
        AppMessage::GameStopped(db_id) => {
            state.borrow().running_games.lock().unwrap().remove(&db_id);
            {
                let rows = state.borrow().rows.get(&db_id).cloned().unwrap_or_default();
                for rw in &rows {
                    rw.row.remove_css_class("playing-game");
                }
            }
            let is_steam = state.borrow().games.iter()
                .find(|g| g.db_id == db_id)
                .map(|g| g.kind == "steam")
                .unwrap_or(false);
            if is_steam {
                refresh_steam_playtimes_for(state, &[db_id]);
            } else {
                refresh_playtime_for(state, &[db_id]);
            }
            let selected_id = state.borrow().selected_id.clone();
            let game = state.borrow().games.iter()
                .find(|g| g.db_id == db_id)
                .cloned();
            if selected_id == db_id.to_string() {
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::GameStarted(db_id) => {
            {
                let rows = state.borrow().rows.get(&db_id).cloned().unwrap_or_default();
                for rw in &rows {
                    rw.row.add_css_class("playing-game");
                }
            }
            let selected_id = state.borrow().selected_id.clone();
            if selected_id == db_id.to_string() {
                let game = state.borrow().games.iter()
                    .find(|g| g.db_id == db_id)
                    .cloned();
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::LutrisDataChanged(data) => {
            handle_lutris_data_changed(state, data);
        }
        AppMessage::SessionRecorded { game_id, duration_seconds, .. } => {
            let mut s = state.borrow_mut();
            if let Some(g) = s.games.iter_mut().find(|g| g.db_id == game_id) {
                g.playtime += (duration_seconds as f64) / 3600.0;
            }
            drop(s);
        }
        AppMessage::ShadPS4PlaytimeChanged => {
            let play_times = crate::platforms::ps4::read_play_times();
            let mut updated_ids = Vec::new();
            for g in state.borrow_mut().games.iter_mut() {
                if g.kind == "ps4" {
                    let serial = &g.platform_id;
                    if let Some(time_str) = play_times.get(serial) {
                        let new_playtime = crate::platforms::ps4::parse_playtime(time_str);
                        if (g.playtime - new_playtime).abs() > 0.001 {
                            g.playtime = new_playtime;
                            updated_ids.push(g.db_id);
                        }
                    }
                }
            }
            if !updated_ids.is_empty() {
                rebuild_sidebar(state);
                let selected_id = state.borrow().selected_id.clone();
                if let Some(id) = updated_ids.first() {
                    if selected_id == id.to_string() {
                        let game = state.borrow().games.iter()
                            .find(|g| g.db_id == *id)
                            .cloned();
                        if let Some(game) = game {
                            display_game(&game, state);
                        }
                    }
                }
            }
        }
        AppMessage::GamesLoaded(games) => {
            handle_games_loaded(state, games);
        }
        AppMessage::ReloadGames => {
            let db = state.borrow().db.clone();
            let save_dir = state.borrow().save_dir.clone();
            let sender = state.borrow().sender.clone();
            let cfg = state.borrow().cfg.clone();
            let shadps4_enabled = cfg.shadps4_enabled;
            let steam_enabled = cfg.steam_enabled;
            let lutris_enabled = cfg.lutris_enabled;
            let ra_config = cfg.ra_config();
            let sort_mode = crate::models::SortMode::from_str(&cfg.sort_mode);
            let sort_descending = cfg.sort_descending;
            std::thread::spawn(move || {
                let games = crate::game_list::build_game_list(
                    &db, &save_dir, lutris_enabled, shadps4_enabled, steam_enabled, &ra_config, sort_mode, sort_descending,
                );
                let _ = sender.send(AppMessage::GamesLoaded(games));
                let ra_any_console = ra_config.psx_enabled || ra_config.ps2_enabled || ra_config.psp_enabled;
                if ra_any_console {
                    crate::game_list::build_ra_games_threaded(&db, &save_dir, &ra_config, &sender);
                }
            });
        }
        AppMessage::SgdbAssetsDownloaded { db_id, sgdb_id, icon, hero, grid, logo, header } => {
            if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                g.sgdb_id = sgdb_id;
                if !icon.is_empty() { g.icon_path = icon; }
                if !hero.is_empty() { g.hero_image_path = hero; }
                if !grid.is_empty() { g.grid_path = grid; }
                if !logo.is_empty() { g.logo_path = logo; }
                if !header.is_empty() { g.header_path = header; }
            }
            super::helpers::refresh_settings_images_page(state, db_id, |s, game, win| {
                build_image_manager_content(s, game, win).upcast()
            });
            let selected_id = state.borrow().selected_id.clone();
            if selected_id == db_id.to_string() {
                let game = state.borrow().games.iter()
                    .find(|g| g.db_id == db_id)
                    .cloned();
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
    }
}

fn refresh_playtime_for(state: &SharedState, db_ids: &[i64]) {
    let lutris_ids: Vec<i64> = {
        let s = state.borrow();
        db_ids.iter()
            .filter_map(|&db_id| s.games.iter().find(|g| g.db_id == db_id).map(|g| g.lutris_id))
            .filter(|&id| id != 0)
            .collect()
    };
    let Ok(all) = crate::platforms::lutris::load_lutris_playtime() else { return };
    let id_set: HashSet<i64> = lutris_ids.iter().copied().collect();
    let lutris_map: HashMap<i64, (f64, i64)> = all
        .into_iter()
        .filter(|(id, _, _)| id_set.contains(id))
        .map(|(id, pt, lp)| (id, (pt, lp)))
        .collect();
    let db_map: HashMap<i64, (f64, i64)> = {
        let s = state.borrow();
        s.games.iter()
            .filter(|g| id_set.contains(&g.lutris_id))
            .filter_map(|g| lutris_map.get(&g.lutris_id).map(|&v| (g.db_id, v)))
            .collect()
    };
    apply_playtime_updates_db(state, &db_map);
}

fn refresh_steam_playtimes_for(state: &SharedState, db_ids: &[i64]) {
    let id_set: HashSet<i64> = db_ids.iter().copied().collect();
    let app_ids: Vec<(i64, String)> = {
        let s = state.borrow();
        s.games.iter()
            .filter(|g| id_set.contains(&g.db_id) && g.kind == "steam")
            .map(|g| (g.db_id, g.app_id.clone()))
            .collect()
    };
    if app_ids.is_empty() {
        return;
    }

    let all_playtimes = crate::platforms::steam::read_all_playtimes();
    let map: HashMap<i64, (f64, i64)> = app_ids.iter()
        .filter_map(|(db_id, app_id)| {
            all_playtimes.get(app_id).map(|&(pt, lp)| (*db_id, (pt, lp)))
        })
        .collect();
    apply_playtime_updates_db(state, &map);
}

fn apply_playtime_updates_db(state: &SharedState, updates: &HashMap<i64, (f64, i64)>) {
    let mut changed_db_ids: Vec<i64> = Vec::new();
    let selected_db_id: i64;

    {
        let mut s = state.borrow_mut();
        selected_db_id = s.selected_id.parse().unwrap_or(0);

        for g in &mut s.games {
            if let Some(&(playtime, lastplayed)) = updates.get(&g.db_id) {
                if g.playtime != playtime || g.lastplayed != lastplayed {
                    g.playtime = playtime;
                    g.lastplayed = lastplayed;
                    changed_db_ids.push(g.db_id);
                }
            }
        }
    }

    if changed_db_ids.is_empty() {
        return;
    }

    if changed_db_ids.contains(&selected_db_id) {
        let game = state
            .borrow()
            .games
            .iter()
            .find(|g| g.db_id == selected_db_id)
            .cloned();
        if let Some(game) = game {
            display_game(&game, state);
        }
    }

    let is_grid_showing = state.borrow().selected_id.is_empty() && !state.borrow().content_unloaded;
    if is_grid_showing {
        show_grid_view(state);
    }
}

fn handle_lutris_data_changed(state: &SharedState, data: Vec<(i64, f64, i64)>) {
    let lutris_map: HashMap<i64, (f64, i64)> =
        data.into_iter().map(|(id, pt, lp)| (id, (pt, lp))).collect();
    let db_map: HashMap<i64, (f64, i64)> = {
        let s = state.borrow();
        s.games.iter()
            .filter(|g| g.lutris_id != 0)
            .filter_map(|g| lutris_map.get(&g.lutris_id).map(|&v| (g.db_id, v)))
            .collect()
    };
    apply_playtime_updates_db(state, &db_map);
}

fn handle_games_loaded(state: &SharedState, games: Vec<Game>) {
    {
        let mut s = state.borrow_mut();
        s.games = games;
        let mut names = s.game_names.lock().unwrap();
        for g in &s.games {
            if !g.app_id.is_empty() {
                names.insert(g.app_id.clone(), g.name.clone());
            }
        }
    }

    rebuild_sidebar(state);

    let row = state.borrow().game_list.row_at_index(0);
    select_row_silently(state, row.as_ref());
    show_grid_view(state);

    let (steam, watcher, sender) = {
        let s = state.borrow();
        (s.steam.clone(), s.watcher.clone(), s.sender.clone())
    };

    let (ra_username, ra_token, ra_password) = {
        let s = state.borrow();
        (s.cfg.ra_username.clone(), s.cfg.ra_token.clone(), s.cfg.ra_password.clone())
    };

    let db = state.borrow().db.clone();
    let s = state.borrow();
    for g in &s.games {
        if g.app_id.is_empty() {
            continue;
        }
        if crate::models::has_steam_enrichment(&g.trophy_source) {
            if let Some(ref watcher) = watcher {
                let mut entry = GameEntry::for_reload(g.db_id, &g.kind, &g.trophy_source, &g.app_id, &g.platform_id);
                entry.sort_title = g.sort_title.clone();
                watcher.watch(&entry, &g.achievements);
            }
        }

        if g.kind == "ps4" {
            continue;
        }

        if g.kind == "retro" && g.trophy_source.is_empty() {
            continue;
        }

        enrich_game_async(
            g.app_id.clone(),
            g.trophy_source.clone(),
            g.platform_id.clone(),
            g.db_id,
            g.lutris_id,
            g.name.clone(),
            steam.clone(),
            watcher.clone(),
            sender.clone(),
            state.borrow().save_dir.clone(),
            db.clone(),
            ra_username.clone(),
            ra_token.clone(),
            ra_password.clone(),
        );
    }
}

pub(crate) fn apply_game_update(state: &SharedState, mut updated: Game) {
    let app_id = updated.app_id.clone();

    let (game_for_display, needs_grid_refresh) = {
        let mut s = state.borrow_mut();
        let Some(i) = s.games.iter().position(|g| g.app_id == app_id) else {
            return;
        };

        let was_placeholder =
            s.games[i].name.is_empty() || s.games[i].name.starts_with("App ID:");

        let old_grid_path = s.games[i].grid_path.clone();
        let old_header_path = s.games[i].header_path.clone();

        merge_game_enrichment(&s.games[i], &mut updated);

        if was_placeholder && !updated.name.is_empty() && !updated.name.starts_with("App ID:") {
            let db = s.db.clone();
            if let Err(e) = crate::db::update_game_title(&db, updated.db_id, &updated.name) {
                eprintln!("Failed to persist game title: {}", e);
            }
        }

        s.game_names.lock().unwrap().insert(app_id.clone(), updated.name.clone());

        let row_widgets: Vec<super::sidebar::SidebarRowWidgets> = s.rows.get(&updated.db_id).cloned().unwrap_or_default();
        for rw in &row_widgets {
            rw.title.set_text(&updated.name);
            rw.title.set_tooltip_text(Some(&format!("{} ({})", updated.name, updated.app_id)));
            if !updated.icon_path.is_empty() {
                images::set_image(&rw.icon, &updated.icon_path);
            }
        }

        let needs_rebuild = s.selected_id == updated.db_id.to_string() && !s.content_unloaded;
        let visual_changed = updated.grid_path != old_grid_path
            || updated.header_path != old_header_path;
        let needs_grid_refresh = visual_changed
            && s.selected_id.is_empty()
            && !s.content_unloaded
            && !s.grid_refresh_pending;
        if needs_grid_refresh {
            s.grid_refresh_pending = true;
        }
        let game = if needs_rebuild { Some(updated.clone()) } else { None };
        s.games[i] = updated;
        (game, needs_grid_refresh)
    };

    if let Some(game) = game_for_display {
        display_game(&game, state);
    }
    if needs_grid_refresh {
        let state_clone = state.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            let mut s = state_clone.borrow_mut();
            s.grid_refresh_pending = false;
            let should_refresh = s.selected_id.is_empty() && !s.content_unloaded;
            drop(s);
            if should_refresh {
                show_grid_view(&state_clone);
            }
        });
    }
}

fn insert_or_update_game(state: &SharedState, game: Game) {
    let app_id = game.app_id.clone();
    let needs_enrichment = game.kind == "retro" && !game.trophy_source.is_empty() && !game.app_id.is_empty();

    {
        let mut s = state.borrow_mut();
        if !app_id.is_empty() {
            s.game_names.lock().unwrap().insert(app_id.clone(), game.name.clone());
        }

        let found = s.games.iter().position(|g| g.db_id == game.db_id);

        if let Some(i) = found {
            let mut g = game;
            g.hidden = s.games[i].hidden;
            g.lutris_name = s.games[i].lutris_name.clone();
            g.manual_unmatch = s.games[i].manual_unmatch;
            s.games[i] = g;
        } else {
            s.games.push(game);
            let sort_mode = crate::models::SortMode::from_str(&s.cfg.sort_mode);
            let sort_descending = s.cfg.sort_descending;
            s.games.sort_by(|a, b| {
                let ord = sort_mode.compare(a, b);
                if sort_descending { ord.reverse() } else { ord }
            });
        }
    }

    if !state.borrow().content_unloaded {
        let needs_rebuild = !state.borrow().sidebar_rebuild_pending;
        if needs_rebuild {
            state.borrow_mut().sidebar_rebuild_pending = true;
            let state_clone = state.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                state_clone.borrow_mut().sidebar_rebuild_pending = false;
                rebuild_sidebar(&state_clone);
                let selected = state_clone.borrow().selected_id.clone();
                if selected.is_empty() {
                    let row = state_clone.borrow().game_list.row_at_index(0);
                    select_row_silently(&state_clone, row.as_ref());
                }
                let needs_refresh = !state_clone.borrow().grid_refresh_pending;
                if needs_refresh {
                    state_clone.borrow_mut().grid_refresh_pending = true;
                    let sc = state_clone.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                        let mut s = sc.borrow_mut();
                        s.grid_refresh_pending = false;
                        let should_refresh = s.selected_id.is_empty() && !s.content_unloaded;
                        drop(s);
                        if should_refresh {
                            show_grid_view(&sc);
                        }
                    });
                }
            });
        }
    }

    if needs_enrichment {
        let (ra_username, ra_token, ra_password, steam, watcher, sender, save_dir, db) = {
            let s = state.borrow();
            (
                s.cfg.ra_username.clone(),
                s.cfg.ra_token.clone(),
                s.cfg.ra_password.clone(),
                s.steam.clone(),
                s.watcher.clone(),
                s.sender.clone(),
                s.save_dir.clone(),
                s.db.clone(),
            )
        };
        let g = state.borrow().games.iter().find(|g| g.app_id == app_id).cloned();
        if let Some(g) = g {
            enrich_game_async(
                g.app_id.clone(),
                g.trophy_source.clone(),
                g.platform_id.clone(),
                g.db_id,
                g.lutris_id,
                g.name.clone(),
                steam,
                watcher,
                sender,
                save_dir,
                db,
                ra_username,
                ra_token,
                ra_password,
            );
        }
    }
}

pub fn switch_to_game(state: &SharedState, db_id: i64) {
    state.borrow_mut().selected_id = db_id.to_string();

    let row = state.borrow().rows.get(&db_id).and_then(|v| v.first()).map(|rw| rw.row.clone());
    if let Some(row) = row {
        select_row_silently(state, Some(&row));
    }

    apply_selected_highlight(state);

    let game = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);
    }
}

pub(crate) fn clear_content(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    clear_children(&content_box);
}
