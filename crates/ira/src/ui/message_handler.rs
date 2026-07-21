use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::Game;
use crate::strings as S;
use std::collections::{HashMap, HashSet};
use super::state::SharedState;
use super::sidebar::{select_row_silently, rebuild_sidebar, rebuild_sidebar_and_show_grid, find_game_index, update_sidebar_game, set_sidebar_playing};
use super::grid_view::show_grid_view;
use super::game_item::GameItem;
use super::game_display::display_game;
use super::helpers::{merge_game_enrichment, clear_children};
use super::enrichment::enrich_game_async;
use super::image_manager::build_image_manager_content_with_drafts;
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
        AppMessage::GameStopped(db_id, _variant_id) => {
            state.borrow().running_games.lock().unwrap().remove(&db_id);
            set_sidebar_playing(state, db_id, false);
            refresh_steam_playtimes_for(state, &[db_id]);
            let selected_id = state.borrow().selected_id.clone();
            if ira_models::parse_db_id(&selected_id) == db_id {
                let game = state.borrow().games.iter()
                    .find(|g| g.grid_id() == selected_id)
                    .cloned();
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::GameStarted(db_id, _variant_id) => {
            set_sidebar_playing(state, db_id, true);
            let selected_id = state.borrow().selected_id.clone();
            if ira_models::parse_db_id(&selected_id) == db_id {
                let game = state.borrow().games.iter()
                    .find(|g| g.grid_id() == selected_id)
                    .cloned();
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::SessionRecorded { game_id, variant_id, duration_seconds, .. } => {
            let hours = (duration_seconds as f64) / 3600.0;
            let (db, new_base_playtime, new_variant_playtime) = {
                let mut s = state.borrow_mut();
                let db = s.db.clone();
                let mut base_pt = 0.0;
                let mut var_pt: Option<(i64, f64)> = None;
                for g in &mut s.games {
                    if g.db_id == game_id && g.variant_id.is_none() {
                        g.playtime += hours;
                        base_pt = g.playtime;
                    } else if g.db_id == game_id && g.variant_id == variant_id && variant_id.is_some() {
                        g.playtime += hours;
                        var_pt = Some((variant_id.unwrap(), g.playtime));
                    }
                }
                (db, base_pt, var_pt)
            };
            if new_base_playtime > 0.0 {
                if let Err(e) = ira_db::update_field(&db, game_id, "playtime", &new_base_playtime) {
                    eprintln!("Failed to update playtime: {}", e);
                }
            }
            if let Some((vid, vpt)) = new_variant_playtime {
                if let Err(e) = ira_db::update_variant_playtime(&db, vid, vpt) {
                    eprintln!("Failed to update variant playtime: {}", e);
                }
            }
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
                    if ira_models::parse_db_id(&selected_id) == *id {
                        let game = state.borrow().games.iter()
                            .find(|g| g.grid_id() == selected_id)
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
            let _span = tracing::info_span!("SgdbAssetsDownloaded", db_id).entered();
            {
                let db = state.borrow().db.clone();
                if let Err(e) = ira_db::set_sgdb_id(&db, db_id, &sgdb_id) {
                    eprintln!("Failed to set SGDB ID: {}", e);
                }
            }
            if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                g.sgdb_id = sgdb_id;
                if !icon.is_empty() { g.icon_path = icon; }
                if !hero.is_empty() { g.hero_image_path = hero; }
                if !grid.is_empty() { g.grid_path = grid; }
                if !logo.is_empty() { g.logo_path = logo; }
                if !header.is_empty() { g.header_path = header; }
            }
            super::helpers::refresh_settings_images_page(state, db_id, |s, game, win, pc| {
                build_image_manager_content_with_drafts(s, game, win, pc).upcast()
            });
            let selected_id = state.borrow().selected_id.clone();
            if ira_models::parse_db_id(&selected_id) == db_id {
                let game = state.borrow().games.iter()
                    .find(|g| g.grid_id() == selected_id)
                    .cloned();
                if let Some(game) = game {
                    display_game(&game, state);
                }
            }
        }
        AppMessage::VariantSelected(db_id, variant_id) => {
            // For show_as_entry variants, navigate to the variant's grid entry
            if let Some(vid) = variant_id {
                let is_show_as_entry = ira_db::get_variants(&state.borrow().db, db_id)
                    .unwrap_or_default()
                    .iter()
                    .find(|v| v.id == vid)
                    .is_some_and(|v| v.show_as_entry);
                if is_show_as_entry {
                    switch_to_game(state, db_id, Some(vid));
                    return;
                }
            }

            // If the selected game is a variant entry and the user selected
            // "Base game" or a non-show_as_entry variant, navigate to the base game
            let selected_id = state.borrow().selected_id.clone();
            if ira_models::parse_db_id(&selected_id) == db_id && selected_id != db_id.to_string() {
                switch_to_game(state, db_id, None);
            }

            let (db, save_dir, sender) = {
                let s = state.borrow();
                (s.db.clone(), s.save_dir.clone(), s.sender.clone())
            };
            std::thread::spawn(move || {
                let Some(entry) = ira_db::find_by_db_id(&db, db_id).ok().flatten() else { return };
                let Ok(mut game) = crate::game_loader::load_game(&entry, &save_dir) else { return };
                if let Some(vid) = variant_id {
                    crate::game_loader::apply_variant_images_for(&db, &save_dir, &entry, &mut game, vid);
                }
                let _ = sender.send(crate::AppMessage::EnrichedGame(game));
            });
        }
        AppMessage::VariantsChanged(db_id) => {
            let (db, save_dir) = {
                let s = state.borrow();
                (s.db.clone(), s.save_dir.clone())
            };
            {
                let mut s = state.borrow_mut();
                s.games.retain(|g| g.db_id != db_id || g.variant_id.is_none());
            }
            if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, db_id) {
                if let Ok(game) = crate::game_loader::load_game_fast(&entry, &save_dir) {
                    let variant_entries = crate::game_loader::build_variant_entries(&db, &save_dir, &game);
                    let mut s = state.borrow_mut();
                    if let Some(idx) = s.games.iter().position(|g| g.db_id == db_id && g.variant_id.is_none()) {
                        s.games[idx] = game;
                    }
                    s.games.extend(variant_entries);
                    let sort_mode = s.cfg.sort_mode;
                    let sort_descending = s.cfg.sort_descending;
                    s.games.sort_by(|a, b| {
                        let ord = sort_mode.compare(a, b);
                        if sort_descending { ord.reverse() } else { ord }
                    });
                }
            }
            super::sidebar::rebuild_sidebar(state);
        }
    }
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
        selected_db_id = ira_models::parse_db_id(&s.selected_id);

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
}

fn handle_games_loaded(state: &SharedState, games: Vec<Game>) {
    {
        let mut s = state.borrow_mut();
        s.games = games;

        // Validate default variants: reset if no longer eligible
        let db = s.db.clone();
        let mut db_ids_to_check: Vec<i64> = s.games.iter()
            .filter(|g| g.variant_id.is_none())
            .map(|g| g.db_id)
            .collect();
        db_ids_to_check.dedup();
        for db_id in &db_ids_to_check {
            if let Some(default_vid) = ira_db::get_default_variant(&db, *db_id) {
                let eligible = ira_db::get_variants(&db, *db_id)
                    .unwrap_or_default()
                    .iter()
                    .find(|v| v.id == default_vid)
                    .is_some_and(|v| v.count_playtime && !v.show_as_entry);
                if !eligible {
                    ira_db::set_default_variant(&db, *db_id, None);
                }
            }
        }

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

    start_background_enrichment(state);
}

fn start_background_enrichment(state: &SharedState) {
    let (steam, watcher, sender, db, save_dir, ra_username, ra_token, ra_password) = {
        let s = state.borrow();
        (s.steam.clone(), s.watcher.clone(), s.sender.clone(), s.db.clone(), s.save_dir.clone(), s.cfg.ra_username.clone(), s.cfg.ra_token.clone(), s.cfg.ra_password.clone())
    };

    let s = state.borrow();
    for g in &s.games {
        if g.app_id.is_empty() || g.variant_id.is_some() {
            continue;
        }
        if g.trophy_source.has_steam_enrichment() {
            if let Some(ref watcher) = watcher {
                let mut entry = GameEntry::for_reload(g.db_id, g.kind, g.trophy_source, &g.app_id, "", &g.platform_id);
                entry.sort_title = g.sort_title.clone();
                watcher.watch(&entry, &g.achievements);
            }
        }

        if g.kind == ira_models::GameKind::Ps4 || g.kind == ira_models::GameKind::Retro {
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
            game: Some(g.clone()),
        });
    }

    let ra_games: Vec<(i64, String, String)> = {
        let s = state.borrow();
        s.games.iter()
            .filter(|g| g.kind == ira_models::GameKind::Retro
                && g.trophy_source == ira_models::TrophySource::Ra
                && !g.app_id.is_empty())
            .map(|g| (g.db_id, g.app_id.clone(), g.platform_id.clone()))
            .collect()
    };
    if !ra_games.is_empty() {
        let db = db.clone();
        let sender = sender.clone();
        let save_dir = save_dir.clone();
        std::thread::spawn(move || {
            let _s = tracing::info_span!("background_load_ra", count = ra_games.len()).entered();
            let n_threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(ra_games.len())
                .max(1);
            let chunk_size = ra_games.len().div_ceil(n_threads).max(1);
            std::thread::scope(|s| {
                let handles: Vec<_> = ra_games.chunks(chunk_size).map(|chunk| {
                    let db = &db;
                    let save_dir = &save_dir;
                    let sender = &sender;
                    s.spawn(move || {
                        for (db_id, app_id, _platform_id) in chunk {
                            let Some(entry) = ira_db::find_by_db_id(db, *db_id).ok().flatten() else {
                                continue;
                            };
                            let current_mtime = crate::game_loader::ra_achievement_mtime(save_dir, app_id);
                            if current_mtime > 0 && current_mtime == entry.cached_achievement_mtime {
                                continue;
                            }
                            if let Ok(updated) = crate::game_loader::load_game(&entry, save_dir) {
                                if let Err(e) = ira_db::update_achievement_counts(db, updated.db_id, updated.earned_count as i64, updated.total_count as i64, current_mtime) {
                                    eprintln!("Failed to update achievement counts: {}", e);
                                }
                                let _ = sender.send(crate::AppMessage::EnrichedGame(updated));
                            }
                        }
                    })
                }).collect();
                for h in handles {
                    let _ = h.join();
                }
            });
        });
    }
}

pub(crate) fn apply_game_update(state: &SharedState, updated: Game) {
    let db_id = updated.db_id;

    let (game_for_display, game_for_grid, sidebar_update, variant_grid_updates) = {
        let mut s = state.borrow_mut();
        let Some(i) = s.games.iter().position(|g| g.db_id == db_id && g.variant_id.is_none()) else {
            return;
        };

        let was_placeholder =
            s.games[i].name.is_empty() || s.games[i].name.starts_with("App ID:");

        let old_grid_path = s.games[i].grid_path.clone();
        let old_header_path = s.games[i].header_path.clone();
        let old_earned = s.games[i].earned_count;
        let old_total = s.games[i].total_count;

        let updated = merge_game_enrichment(&s.games[i], &updated);

        if updated.earned_count != old_earned || updated.total_count != old_total {
            let db = s.db.clone();
            let mtime = if updated.trophy_source == ira_models::TrophySource::Ra {
                crate::game_loader::ra_achievement_mtime(&s.save_dir, &updated.app_id)
            } else {
                0
            };
            if let Err(e) = ira_db::update_achievement_counts(&db, updated.db_id, updated.earned_count as i64, updated.total_count as i64, mtime) {
                eprintln!("Failed to update achievement counts: {}", e);
            }
        }

        if was_placeholder && !updated.name.is_empty() && !updated.name.starts_with("App ID:") {
            let db = s.db.clone();
            if let Err(e) = ira_db::update_game_title(&db, updated.db_id, &updated.name) {
                eprintln!("Failed to persist game title: {}", e);
            }
        }

        s.game_names.lock().unwrap().insert(updated.app_id.clone(), updated.name.clone());

        let icon_path = if updated.icon_path.is_empty() {
            String::new()
        } else {
            updated.icon_path.clone()
        };
        let sidebar_update = (updated.db_id, updated.name.clone(), icon_path);

        let needs_rebuild = s.selected_id == updated.grid_id() && !s.content_unloaded;
        let counts_changed = updated.earned_count != old_earned
            || updated.total_count != old_total;
        let visual_changed = updated.grid_path != old_grid_path
            || updated.header_path != old_header_path
            || counts_changed;
        let needs_grid_update = visual_changed && !s.content_unloaded;
        let game_for_grid = if needs_grid_update { Some(updated.clone()) } else { None };
        let game = if needs_rebuild { Some(updated.clone()) } else { None };

        // Sync variant entries: copy achievements and counts from base game.
        // Playtime and last_played are variant-specific — do not overwrite.
        let sync_db_id = updated.db_id;
        let sync_achievements = updated.achievements.clone();
        let sync_earned = updated.earned_count;
        let sync_total = updated.total_count;
        let mut variant_grid_updates: Vec<Game> = Vec::new();
        {
            for g in &mut s.games {
                if g.db_id == sync_db_id && g.variant_id.is_some() {
                    let g_counts_changed = g.earned_count != sync_earned || g.total_count != sync_total;
                    g.achievements = sync_achievements.clone();
                    g.earned_count = sync_earned;
                    g.total_count = sync_total;
                    if g_counts_changed {
                        variant_grid_updates.push(g.clone());
                    }
                }
            }
        }

        s.games[i] = updated;

        // If the selected game is a variant of this base game, refresh its display
        // (variant entries were just synced with achievements from the base game)
        let game = game.or_else(|| {
            if s.content_unloaded { return None; }
            s.games.iter()
                .find(|g| g.grid_id() == s.selected_id && g.db_id == sync_db_id && g.variant_id.is_some())
                .cloned()
        });

        (game, game_for_grid, sidebar_update, variant_grid_updates)
    };

    let (db_id, name, icon_path) = sidebar_update;
    update_sidebar_game(state, db_id, &name, &icon_path);

    if let Some(game) = game_for_display {
        display_game(&game, state);
    }
    if let Some(g) = game_for_grid {
        let store = state.borrow().grid_store.clone();
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<GameItem>().ok()) {
                if item.game().is_some_and(|gi| gi.db_id == g.db_id && gi.variant_id.is_none()) {
                    store.splice(i, 1, &[GameItem::new(&g)]);
                    break;
                }
            }
        }
    }
    for vg in variant_grid_updates {
        let store = state.borrow().grid_store.clone();
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i).and_then(|o| o.downcast::<GameItem>().ok()) {
                if item.game().is_some_and(|gi| gi.grid_id() == vg.grid_id()) {
                    store.splice(i, 1, &[GameItem::new(&vg)]);
                    break;
                }
            }
        }
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
                rebuild_sidebar_and_show_grid(&state_clone);
            });
        }
    }
}

pub fn switch_to_game(state: &SharedState, db_id: i64, variant_id: Option<i64>) {
    let _span = tracing::info_span!("switch_to_game", db_id, variant_id = ?variant_id).entered();
    state.borrow_mut().selected_id = match variant_id {
        Some(vid) => format!("{}-v{}", db_id, vid),
        None => db_id.to_string(),
    };

    if let Some(index) = find_game_index(state, db_id, variant_id) {
        select_row_silently(state, Some(index));
    }

    let game = state.borrow().games.iter()
        .find(|g| g.db_id == db_id && g.variant_id == variant_id)
        .cloned();
    if let Some(game) = game {
        display_game(&game, state);

        if game.achievements.is_empty() && !game.app_id.is_empty() && game.trophy_source != ira_models::TrophySource::Empty {
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
                game: None,
            });
        } else if game.kind == ira_models::GameKind::Retro
            && game.trophy_source == ira_models::TrophySource::Ra
            && !game.app_id.is_empty()
            && game.achievements.iter().any(|a| a.icon_path.is_empty())
        {
            let app_id = game.app_id.clone();
            let db_id = game.db_id;
            let save_dir = state.borrow().save_dir.clone();
            let db = state.borrow().db.clone();
            let sender = state.borrow().sender.clone();
            std::thread::spawn(move || {
                if ira_platforms::retroachievements::redownload_missing_ra_badges(&save_dir, &app_id) {
                    if let Some(entry) = ira_db::find_by_db_id(&db, db_id).ok().flatten() {
                        if let Ok(updated) = crate::game_loader::load_game(&entry, &save_dir) {
                            let _ = sender.send(crate::AppMessage::EnrichedGame(updated));
                        }
                    }
                }
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
