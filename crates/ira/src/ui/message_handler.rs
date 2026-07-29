use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::strings as S;
use super::state::SharedState;
use super::sidebar::{rebuild_sidebar, set_sidebar_playing};
use super::game_display::display_game;
use super::game_item::GameItem;
use super::image_manager::build_image_manager_content_with_drafts;
use super::message_helpers::{apply_game_update, insert_or_update_game, refresh_steam_playtimes_for, handle_games_loaded, switch_to_game};

pub fn handle_app_message(state: &SharedState, msg: AppMessage) {
    match msg {
        AppMessage::EnrichedGame(game) | AppMessage::WatcherGameUpdated(game) => {
            apply_game_update(state, game);
        }
        AppMessage::NewGame(game) => {
            insert_or_update_game(state, game);
        }
        AppMessage::AddGameError(e) => handle_add_game_error(state, e),
        AppMessage::GameStopped(db_id, _) => handle_game_stopped(state, db_id),
        AppMessage::GameStarted(db_id, _) => handle_game_started(state, db_id),
        AppMessage::SessionRecorded { game_id, variant_id, duration_seconds, .. } => {
            handle_session_recorded(state, game_id, variant_id, duration_seconds);
        }
        AppMessage::ShadPS4PlaytimeChanged => handle_shadps4_playtime_changed(state),
        AppMessage::Rpcs3PlaytimeChanged => handle_rpcs3_playtime_changed(state),
        AppMessage::GamesLoaded(games) => handle_games_loaded(state, games),
        AppMessage::SgdbAssetsDownloaded { db_id, sgdb_id, icon, hero, grid, logo, header } => {
            handle_sgdb_assets_downloaded(state, db_id, sgdb_id, SgdbAssetPaths { icon, hero, grid, logo, header });
        }
        AppMessage::VariantSelected(db_id, variant_id) => {
            handle_variant_selected(state, db_id, variant_id);
        }
        AppMessage::VariantsChanged(db_id) => handle_variants_changed(state, db_id),
    }
}

fn handle_add_game_error(state: &SharedState, e: String) {
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

fn handle_game_stopped(state: &SharedState, db_id: i64) {
    state.borrow().running_games.lock().unwrap().remove(&db_id);
    set_sidebar_playing(state, db_id, false);
    refresh_steam_playtimes_for(state, &[db_id]);
    if let Some(ref watcher) = state.borrow().watcher {
        watcher.unwatch(db_id);
    }
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

fn handle_game_started(state: &SharedState, db_id: i64) {
    set_sidebar_playing(state, db_id, true);
    let (watcher, game, save_dir) = {
        let s = state.borrow();
        let game = s.games.iter()
            .find(|g| g.db_id == db_id && g.variant_id.is_none())
            .cloned();
        (s.watcher.clone(), game, s.save_dir.clone())
    };
    if let (Some(watcher), Some(game)) = (watcher, game) {
        if let Some(watch_file) = crate::game_loader::achievement_watch_file(&game, &save_dir) {
            let entry = GameEntry::for_reload(game.db_id, game.kind, game.trophy_source, &game.app_id, "", &game.platform_id);
            watcher.watch(&entry, &watch_file, &game.achievements);
        }
    }
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

fn handle_session_recorded(state: &SharedState, game_id: i64, variant_id: Option<i64>, duration_seconds: i64) {
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

fn handle_shadps4_playtime_changed(state: &SharedState) {
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = std::cell::RefCell::new(rx);
    std::thread::spawn(move || {
        let _s = tracing::info_span!("ps4_read_playtimes").entered();
        let play_times = ira_platforms::ps4::read_play_times();
        let _ = tx.send(play_times);
    });
    let state = state.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(play_times) => {
                let (updated_ids, selected_id) = {
                    let mut s = state.borrow_mut();
                    let mut updated_ids = Vec::new();
                    for g in s.games.iter_mut() {
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
                    (updated_ids, s.selected_id.clone())
                };
                if !updated_ids.is_empty() {
                    rebuild_sidebar(&state);
                    if let Some(id) = updated_ids.first() {
                        if ira_models::parse_db_id(&selected_id) == *id {
                            let game = state.borrow().games.iter()
                                .find(|g| g.grid_id() == selected_id)
                                .cloned();
                            if let Some(game) = game {
                                display_game(&game, &state);
                            }
                        }
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn handle_rpcs3_playtime_changed(state: &SharedState) {
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = std::cell::RefCell::new(rx);
    std::thread::spawn(move || {
        let _s = tracing::info_span!("ps3_read_persistent").entered();
        let persistent = ira_platforms::ps3::parse_persistent_settings(
            &ira_platforms::ps3::persistent_settings_path(),
        );
        let _ = tx.send(persistent);
    });
    let state = state.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(persistent) => {
                let (updated_ids, selected_id) = {
                    let mut s = state.borrow_mut();
                    let mut updated_ids = Vec::new();
                    for g in s.games.iter_mut() {
                        if g.kind == ira_models::GameKind::Ps3 {
                            let serial = &g.platform_id;
                            let new_playtime = persistent.playtime_ms.get(serial)
                                .map(|ms| ira_platforms::ps3::ms_to_hours(*ms));
                            let new_last_played = persistent.last_played.get(serial).copied();
                            let changed = match new_playtime {
                                Some(pt) if (g.playtime - pt).abs() > 0.001 => {
                                    g.playtime = pt;
                                    true
                                }
                                _ => false,
                            } || match new_last_played {
                                Some(lp) if lp != g.last_played => {
                                    g.last_played = lp;
                                    true
                                }
                                _ => false,
                            };
                            if changed {
                                updated_ids.push(g.db_id);
                            }
                        }
                    }
                    (updated_ids, s.selected_id.clone())
                };
                if !updated_ids.is_empty() {
                    rebuild_sidebar(&state);
                    if let Some(id) = updated_ids.first() {
                        if ira_models::parse_db_id(&selected_id) == *id {
                            let game = state.borrow().games.iter()
                                .find(|g| g.grid_id() == selected_id)
                                .cloned();
                            if let Some(game) = game {
                                display_game(&game, &state);
                            }
                        }
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

struct SgdbAssetPaths {
    icon: String,
    hero: String,
    grid: String,
    logo: String,
    header: String,
}

fn handle_sgdb_assets_downloaded(state: &SharedState, db_id: i64, sgdb_id: String, paths: SgdbAssetPaths) {
    let _span = tracing::info_span!("SgdbAssetsDownloaded", db_id).entered();
    {
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::set_sgdb_id(&db, db_id, &sgdb_id) {
            eprintln!("Failed to set SGDB ID: {}", e);
        }
    }
    let game_for_grid = {
        let mut s = state.borrow_mut();
        if let Some(g) = s.games.iter_mut().find(|g| g.db_id == db_id) {
            g.sgdb_id = sgdb_id;
            if !paths.icon.is_empty() { g.icon_path = paths.icon; }
            if !paths.hero.is_empty() { g.hero_image_path = paths.hero; }
            if !paths.grid.is_empty() { g.grid_path = paths.grid; }
            if !paths.logo.is_empty() { g.logo_path = paths.logo; }
            if !paths.header.is_empty() { g.header_path = paths.header; }
            Some(g.clone())
        } else {
            None
        }
    };

    if let Some(g) = &game_for_grid {
        for path in [&g.icon_path, &g.hero_image_path, &g.grid_path, &g.header_path, &g.logo_path] {
            if !path.is_empty() {
                ira_images::invalidate_texture(path);
            }
        }
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

fn handle_variant_selected(state: &SharedState, db_id: i64, variant_id: Option<i64>) {
    if let Some(vid) = variant_id {
        let is_show_as_entry = ira_db::get_variants(&state.borrow().db, db_id)
            .unwrap_or_default()
            .iter()
            .find(|v| v.id == vid)
            .is_some_and(|v| v.show_as_entry);
        if is_show_as_entry {
            switch_to_game(state, db_id, Some(vid));
            super::sidebar::scroll_to_row(state, db_id, Some(vid));
            return;
        }
    }

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

fn handle_variants_changed(state: &SharedState, db_id: i64) {
    let (db, save_dir) = {
        let s = state.borrow();
        (s.db.clone(), s.save_dir.clone())
    };
    {
        let mut s = state.borrow_mut();
        s.games.retain(|g| g.db_id != db_id || g.variant_id.is_none());
    }
    if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, db_id) {
        if let Ok(reloaded) = crate::game_loader::load_game_fast(&entry, &save_dir) {
            let variant_entries = crate::game_loader::build_variant_entries(&db, &save_dir, &reloaded);
            let mut s = state.borrow_mut();
            if let Some(idx) = s.games.iter().position(|g| g.db_id == db_id && g.variant_id.is_none()) {
                let old = &s.games[idx];
                let mut merged = reloaded;
                merged.game_path.clone_from(&old.game_path);
                merged.slug.clone_from(&old.slug);
                if merged.achievements.is_empty() {
                    merged.achievements.clone_from(&old.achievements);
                    merged.earned_count = old.earned_count;
                    merged.total_count = old.total_count;
                }
                s.games[idx] = merged;
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
    rebuild_sidebar(state);
}
