use gtk4::prelude::*;
use adw::prelude::*;
use crate::AppMessage;
use crate::GameEntry;
use crate::strings as S;
use super::state::SharedState;
use super::sidebar::{rebuild_sidebar, scroll_to_row, set_sidebar_playing};
use super::game_display::display_game;
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
        AppMessage::GameStarted(db_id, _variant_id) => {
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
                            rebuild_sidebar(&state);
                            let selected_id = state.borrow().selected_id.clone();
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
        AppMessage::Rpcs3PlaytimeChanged => {
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
                        let mut updated_ids = Vec::new();
                        for g in state.borrow_mut().games.iter_mut() {
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
                        if !updated_ids.is_empty() {
                            rebuild_sidebar(&state);
                            let selected_id = state.borrow().selected_id.clone();
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
            if let Some(vid) = variant_id {
                let is_show_as_entry = ira_db::get_variants(&state.borrow().db, db_id)
                    .unwrap_or_default()
                    .iter()
                    .find(|v| v.id == vid)
                    .is_some_and(|v| v.show_as_entry);
                if is_show_as_entry {
                    switch_to_game(state, db_id, Some(vid));
                    scroll_to_row(state, db_id, Some(vid));
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
            rebuild_sidebar(state);
        }
    }
}
