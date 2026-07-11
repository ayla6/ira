use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::Game;
use crate::images;
use crate::strings as S;
use std::collections::{HashMap, HashSet};
use super::state::SharedState;
use super::sidebar::{select_row_silently, rebuild_sidebar};
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
        AppMessage::GameStopped(lutris_id) => {
            state.borrow().running_games.lock().unwrap().remove(&lutris_id);
            refresh_playtime_for(state, &[lutris_id]);
            let selected_id = state.borrow().selected_id.clone();
            let game = state.borrow().games.iter()
                .find(|g| g.lutris_id == lutris_id)
                .cloned();
            if selected_id == lutris_id.to_string() {
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::LutrisDataChanged(data) => {
            handle_lutris_data_changed(state, data);
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
                            updated_ids.push(g.lutris_id);
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
                            .find(|g| g.lutris_id == *id)
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
            let lutris_id = state.borrow().games.iter()
                .find(|g| g.db_id == db_id)
                .map(|g| g.lutris_id);
            if let Some(lid) = lutris_id {
                if selected_id == lid.to_string() {
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
}

fn refresh_playtime_for(state: &SharedState, lutris_ids: &[i64]) {
    let Ok(all) = crate::platforms::lutris::load_lutris_playtime() else { return };
    let id_set: HashSet<i64> = lutris_ids.iter().copied().collect();
    let map: HashMap<i64, (f64, i64)> = all
        .into_iter()
        .filter(|(id, _, _)| id_set.contains(id))
        .map(|(id, pt, lp)| (id, (pt, lp)))
        .collect();
    apply_playtime_updates(state, &map);
}

fn apply_playtime_updates(state: &SharedState, updates: &HashMap<i64, (f64, i64)>) {
    let mut changed_ids: Vec<i64> = Vec::new();
    let selected_lutris_id: i64;

    {
        let mut s = state.borrow_mut();
        selected_lutris_id = s.selected_id.parse().unwrap_or(0);

        for g in &mut s.games {
            if let Some(&(playtime, lastplayed)) = updates.get(&g.lutris_id) {
                if g.playtime != playtime || g.lastplayed != lastplayed {
                    g.playtime = playtime;
                    g.lastplayed = lastplayed;
                    changed_ids.push(g.lutris_id);
                }
            }
        }
    }

    if changed_ids.is_empty() {
        return;
    }

    if changed_ids.contains(&selected_lutris_id) {
        let game = state
            .borrow()
            .games
            .iter()
            .find(|g| g.lutris_id == selected_lutris_id)
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
    let map: HashMap<i64, (f64, i64)> =
        data.into_iter().map(|(id, pt, lp)| (id, (pt, lp))).collect();
    apply_playtime_updates(state, &map);
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

    let s = state.borrow();
    for g in &s.games {
        if g.app_id.is_empty() {
            continue;
        }
        if g.kind != "sgdb" && g.kind != "ps4" {
            if let Some(ref watcher) = watcher {
                let mut entry = GameEntry::for_reload(g.db_id, &g.kind, &g.app_id, &g.platform_id, g.lutris_id);
                entry.sort_title = g.sort_title.clone();
                watcher.watch(&entry, &g.achievements);
            }
        }

        if g.kind == "ps4" {
            continue;
        }

        enrich_game_async(
            g.app_id.clone(),
            g.kind.clone(),
            g.platform_id.clone(),
            g.db_id,
            g.lutris_id,
            g.name.clone(),
            steam.clone(),
            watcher.clone(),
            sender.clone(),
            state.borrow().save_dir.clone(),
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

        if i < s.rows.len() {
            let rw = &s.rows[i];
            rw.title.set_text(&updated.name);
            rw.title.set_tooltip_text(Some(&format!("{} ({})", updated.name, updated.app_id)));
            if !updated.icon_path.is_empty() {
                images::set_image(&rw.icon, &updated.icon_path);
            }
        }

        let needs_rebuild = s.selected_id == updated.lutris_id.to_string() && !s.content_unloaded;
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
    let lutris_id = game.lutris_id;

    {
        let mut s = state.borrow_mut();
        if !app_id.is_empty() {
            s.game_names.lock().unwrap().insert(app_id.clone(), game.name.clone());
        }

        let found = if lutris_id != 0 {
            s.games.iter().position(|g| g.lutris_id == lutris_id)
        } else {
            None
        };
        let found = found.or_else(|| {
            if !app_id.is_empty() {
                s.games.iter().position(|g| g.app_id == app_id)
            } else {
                None
            }
        });

        if let Some(i) = found {
            let mut g = game;
            g.hidden = s.games[i].hidden;
            g.lutris_name = s.games[i].lutris_name.clone();
            g.manual_unmatch = s.games[i].manual_unmatch;
            s.games[i] = g;
        } else {
            s.games.push(game);
            s.games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
        }
    }

    if !state.borrow().content_unloaded {
        rebuild_sidebar(state);
        let selected = state.borrow().selected_id.clone();
        if selected.is_empty() {
            let row = state.borrow().game_list.row_at_index(0);
            select_row_silently(state, row.as_ref());
            let needs_refresh = !state.borrow().grid_refresh_pending;
            if needs_refresh {
                state.borrow_mut().grid_refresh_pending = true;
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
    }
}

pub fn switch_to_game(state: &SharedState, lutris_id: i64) {
    state.borrow_mut().selected_id = lutris_id.to_string();

    let game_list = state.borrow().game_list.clone();
    let idx = state.borrow().games.iter().position(|g| g.lutris_id == lutris_id);
    if let Some(idx) = idx {
        let row = game_list.row_at_index((idx + 1) as i32);
        select_row_silently(state, row.as_ref());
    }

    let game = state.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);
    }
}

pub(crate) fn clear_content(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    clear_children(&content_box);
}
