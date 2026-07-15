use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::Game;
use crate::strings as S;
use std::collections::{HashMap, HashSet};
use super::state::SharedState;
use super::sidebar::{select_row_silently, rebuild_sidebar, find_game_index, update_sidebar_game, set_sidebar_playing};
use super::grid_view::show_grid_view;
use super::game_display::display_game;
use super::helpers::{merge_game_enrichment, clear_children};
use super::enrichment::enrich_game_async;
use super::image_manager::build_image_manager_content;

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
        AppMessage::GameStopped(db_id) => {
            state.borrow().running_games.lock().unwrap().remove(&db_id);
            set_sidebar_playing(state, db_id, false);
            let is_steam = state.borrow().games.iter()
                .find(|g| g.db_id == db_id)
                .map(|g| g.kind == ira_models::GameKind::Steam)
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
            set_sidebar_playing(state, db_id, true);
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
            let play_times = ira_platforms::ps4::read_play_times();
            let mut updated_ids = Vec::new();
            for g in state.borrow_mut().games.iter_mut() {
                if g.kind == ira_models::GameKind::Ps4 {
                    let serial = &g.platform_id;
                    if let Some(time_str) = play_times.get(serial) {
                        let new_playtime = ira_platforms::ps4::parse_playtime(time_str);
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
    let Ok(all) = ira_platforms::lutris::load_lutris_playtime() else { return };
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
            .filter(|g| id_set.contains(&g.db_id) && g.kind == ira_models::GameKind::Steam)
            .map(|g| (g.db_id, g.app_id.clone()))
            .collect()
    };
    if app_ids.is_empty() {
        return;
    }

    let all_playtimes = ira_platforms::steam::read_all_playtimes();
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
                if g.playtime != playtime || g.last_played != lastplayed {
                    g.playtime = playtime;
                    g.last_played = lastplayed;
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

    select_row_silently(state, Some(0));
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
        if g.trophy_source.has_steam_enrichment() {
            if let Some(ref watcher) = watcher {
                let mut entry = GameEntry::for_reload(g.db_id, g.kind, g.trophy_source, &g.app_id, "", &g.platform_id);
                entry.sort_title = g.sort_title.clone();
                watcher.watch(&entry, &g.achievements);
            }
        }

        if g.kind == ira_models::GameKind::Ps4 {
            continue;
        }

        if g.kind == ira_models::GameKind::Retro {
            continue;
        }

        enrich_game_async(crate::ui::enrichment::EnrichGameParams {
            app_id: g.app_id.clone(),
            trophy_source: g.trophy_source,
            platform_id: g.platform_id.clone(),
            db_id: g.db_id,
            title: g.name.clone(),
            steam: steam.clone(),
            watcher: watcher.clone(),
            sender: sender.clone(),
            save_dir: state.borrow().save_dir.clone(),
            db: db.clone(),
            ra_username: ra_username.clone(),
            ra_token: ra_token.clone(),
            ra_password: ra_password.clone(),
        });
    }
}

pub(crate) fn apply_game_update(state: &SharedState, mut updated: Game) {
    let app_id = updated.app_id.clone();

    let (game_for_display, needs_grid_refresh, sidebar_update) = {
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
            if let Err(e) = ira_db::update_game_title(&db, updated.db_id, &updated.name) {
                eprintln!("Failed to persist game title: {}", e);
            }
        }

        s.game_names.lock().unwrap().insert(app_id.clone(), updated.name.clone());

        let icon_path = if updated.icon_path.is_empty() {
            String::new()
        } else {
            updated.icon_path.clone()
        };
        let sidebar_update = (updated.db_id, updated.name.clone(), icon_path);

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
        (game, needs_grid_refresh, sidebar_update)
    };

    let (db_id, name, icon_path) = sidebar_update;
    update_sidebar_game(state, db_id, &name, &icon_path);

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
            let sort_mode = s.cfg.sort_mode;
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
                    select_row_silently(&state_clone, Some(0));
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
}

pub fn switch_to_game(state: &SharedState, db_id: i64) {
    state.borrow_mut().selected_id = db_id.to_string();

    if let Some(index) = find_game_index(state, db_id) {
        select_row_silently(state, Some(index));
    }

    let game = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);

        if game.kind == ira_models::GameKind::Retro && game.trophy_source != ira_models::TrophySource::Empty && game.total_count == 0 && !game.app_id.is_empty() {
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
            enrich_game_async(crate::ui::enrichment::EnrichGameParams {
                app_id: game.app_id.clone(),
                trophy_source: game.trophy_source,
                platform_id: game.platform_id.clone(),
                db_id: game.db_id,
                title: game.name.clone(),
                steam,
                watcher,
                sender,
                save_dir,
                db,
                ra_username,
                ra_token,
                ra_password,
            });
        }
    }
}

pub(crate) fn clear_content(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let grid_header = state.borrow().grid_header.clone();
    clear_children(&content_box);
    clear_children(&grid_header);
}
