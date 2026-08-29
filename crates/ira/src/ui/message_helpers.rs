use super::enrichment::enrich_game_async;
use super::game_display::{display_game, display_game_cached};
use super::grid_view::{show_grid_view, show_loading_view};
use super::helpers::{clear_children, merge_game_enrichment, replace_grid_game};
use super::sidebar::{
    find_game_index, rebuild_sidebar, rebuild_sidebar_and_show_grid, select_row_silently,
    update_sidebar_game,
};
use super::state::SharedState;
use crate::Game;
use std::collections::{HashMap, HashSet};
pub(super) fn refresh_steam_playtimes_for(state: &SharedState, db_ids: &[i64]) {
    let id_set: HashSet<i64> = db_ids.iter().copied().collect();
    let app_ids: Vec<(i64, String)> = {
        let s = state.borrow();
        s.games
            .iter()
            .filter(|g| id_set.contains(&g.db_id) && g.kind == ira_models::GameKind::Steam)
            .map(|g| (g.db_id, g.app_id.clone()))
            .collect()
    };
    if app_ids.is_empty() {
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let rx = std::cell::RefCell::new(rx);
    std::thread::spawn(move || {
        let _s = tracing::info_span!("steam_read_playtimes").entered();
        let all_playtimes = ira_platforms::steam::read_all_playtimes();
        let map: HashMap<i64, (f64, i64)> = app_ids
            .iter()
            .filter_map(|(db_id, app_id)| {
                all_playtimes
                    .get(app_id)
                    .map(|&(pt, lp)| (*db_id, (pt, lp)))
            })
            .collect();
        let _ = tx.send(map);
    });
    let state = state.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(map) => {
                apply_playtime_updates_db(&state, &map);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
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
        refresh_selected_game(state, selected_db_id);
    }
}

pub(super) fn refresh_selected_game(state: &SharedState, db_id: i64) {
    refresh_selected_game_if(state, |selected_id| {
        ira_models::parse_db_id(selected_id) == db_id
    });
}

pub(super) fn refresh_selected_base_game(state: &SharedState, db_id: i64) {
    refresh_selected_game_if(state, |selected_id| selected_id == db_id.to_string());
}

fn refresh_selected_game_if(state: &SharedState, is_selected: impl FnOnce(&str) -> bool) {
    let selected_id = state.borrow().selected_id.clone();
    if !is_selected(&selected_id) {
        return;
    }
    let game = state
        .borrow()
        .games
        .iter()
        .find(|g| g.grid_id() == selected_id)
        .cloned();
    if let Some(game) = game {
        display_game(&game, state);
    }
}

pub(super) fn handle_games_loaded(state: &SharedState, games: Vec<Game>) {
    {
        let mut s = state.borrow_mut();
        s.games = games;

        let db = s.db.clone();
        let mut db_ids_to_check: Vec<i64> = s
            .games
            .iter()
            .filter(|g| g.variant_id.is_none())
            .map(|g| g.db_id)
            .collect();
        db_ids_to_check.sort();
        db_ids_to_check.dedup();
        std::thread::spawn(move || {
            let _s = tracing::info_span!("validate_default_variants").entered();
            for db_id in &db_ids_to_check {
                if let Ok(Some(default_vid)) = ira_db::get_default_variant(&db, *db_id) {
                    let eligible = ira_db::get_variants(&db, *db_id)
                        .unwrap_or_default()
                        .iter()
                        .find(|v| v.id == default_vid)
                        .is_some_and(|v| v.count_playtime && !v.show_as_entry);
                    if !eligible {
                        if let Err(e) = ira_db::set_default_variant(&db, *db_id, None) {
                            eprintln!("Failed to clear default variant for {}: {e}", db_id);
                        }
                    }
                }
            }
        });

        let mut names = s.game_names.lock().unwrap();
        for g in &s.games {
            if !g.app_id.is_empty() {
                names.insert(g.app_id.clone(), g.name.clone());
            }
        }
    }

    rebuild_sidebar(state);

    show_grid_view(state);

    start_background_enrichment(state);
}

pub(super) fn reload_games(state: &SharedState) {
    let (db, save_dir, cfg, sender) = {
        let mut s = state.borrow_mut();
        s.games.clear();
        s.selected_id.clear();
        (
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.clone(),
            s.sender.clone(),
        )
    };
    rebuild_sidebar(state);
    show_loading_view(state, &crate::tr!("Preparing game library…"), 0, 1);
    crate::game_list::start_game_list_load(db, save_dir, cfg, sender);
}

fn start_background_enrichment(state: &SharedState) {
    let (
        steam,
        sender,
        db,
        save_dir,
        ra_username,
        ra_web_api_key,
        cfg,
        enrich_targets,
        ra_games,
        sgdb_games,
    ) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.ra_username.clone(),
            s.cfg.ra_web_api_key.clone(),
            s.cfg.clone(),
            // Retro games join enrichment only while icon-less, so the
            // RA → native → SGDB default icon chain can fill them in.
            s.games
                .iter()
                .filter(|g| {
                    !g.app_id.is_empty()
                        && g.variant_id.is_none()
                        && g.kind != ira_models::GameKind::Ps4
                        && g.kind != ira_models::GameKind::Ps3
                        && (g.kind != ira_models::GameKind::Retro || g.icon_path.is_empty())
                })
                .map(|g| {
                    (
                        g.app_id.clone(),
                        g.trophy_source,
                        g.platform_id.clone(),
                        g.db_id,
                        g.name.clone(),
                        g.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            s.games
                .iter()
                .filter(|g| {
                    g.kind == ira_models::GameKind::Retro
                        && g.trophy_source == ira_models::TrophySource::Ra
                        && !g.app_id.is_empty()
                })
                .map(|g| (g.db_id, g.app_id.clone(), g.platform_id.clone()))
                .collect::<Vec<_>>(),
            s.games
                .iter()
                .filter(|g| {
                    !g.sgdb_id.is_empty()
                        && g.variant_id.is_none()
                        && g.kind != ira_models::GameKind::Ps4
                        && (g.icon_path.is_empty()
                            || g.hero_image_path.is_empty()
                            || g.grid_path.is_empty()
                            || g.logo_path.is_empty()
                            || g.header_path.is_empty())
                })
                .map(|g| {
                    (
                        g.db_id,
                        g.sgdb_id.clone(),
                        g.kind,
                        g.app_id.clone(),
                        g.trophy_source,
                    )
                })
                .collect::<Vec<_>>(),
        )
    };

    if !enrich_targets.is_empty() {
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(enrich_targets.len())
            .max(1);
        let chunk_size = enrich_targets.len().div_ceil(n_threads).max(1);
        let steam = steam.clone();
        let sender = sender.clone();
        let db = db.clone();
        let save_dir = save_dir.clone();
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let _s =
                tracing::info_span!("background_enrich", count = enrich_targets.len()).entered();
            std::thread::scope(|s| {
                let handles: Vec<_> = enrich_targets
                    .chunks(chunk_size)
                    .map(|chunk| {
                        let steam = &steam;
                        let sender = &sender;
                        let db = &db;
                        let save_dir = &save_dir;
                        let ra_username = &ra_username;
                        let ra_web_api_key = &ra_web_api_key;
                        let cfg = &cfg;
                        s.spawn(move || {
                            for (app_id, trophy_source, platform_id, db_id, title, game) in chunk {
                                let _ =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        crate::ui::enrichment::enrich_game_blocking(
                                            crate::ui::enrichment::EnrichGameParams {
                                                app_id: app_id.clone(),
                                                trophy_source: *trophy_source,
                                                platform_id: platform_id.clone(),
                                                db_id: *db_id,
                                                title: title.clone(),
                                                steam: steam.clone(),
                                                sender: sender.clone(),
                                                save_dir: save_dir.clone(),
                                                db: db.clone(),
                                                ra_username: ra_username.clone(),
                                                ra_web_api_key: ra_web_api_key.clone(),
                                                cfg: cfg.clone(),
                                                game: Some(game.clone()),
                                            },
                                        );
                                    }));
                            }
                        })
                    })
                    .collect();
                for h in handles {
                    let _ = h.join();
                }
            });
        });
    }

    if !ra_games.is_empty() {
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
                let handles: Vec<_> = ra_games
                    .chunks(chunk_size)
                    .map(|chunk| {
                        let db = &db;
                        let save_dir = &save_dir;
                        let sender = &sender;
                        s.spawn(move || {
                            for (db_id, app_id, _platform_id) in chunk {
                                let Some(entry) = ira_db::find_by_db_id(db, *db_id).ok().flatten()
                                else {
                                    continue;
                                };
                                let current_mtime =
                                    crate::game_loader::ra_achievement_mtime(save_dir, app_id);
                                if current_mtime > 0
                                    && current_mtime == entry.cached_achievement_mtime
                                {
                                    continue;
                                }
                                if let Ok(updated) = crate::game_loader::load_game(&entry, save_dir)
                                {
                                    if let Err(e) = ira_db::update_achievement_counts(
                                        db,
                                        updated.db_id,
                                        updated.earned_count as i64,
                                        updated.total_count as i64,
                                        current_mtime,
                                    ) {
                                        eprintln!("Failed to update achievement counts: {}", e);
                                    }
                                    let _ = sender.send(crate::AppMessage::EnrichedGame(updated));
                                }
                            }
                        })
                    })
                    .collect();
                for h in handles {
                    let _ = h.join();
                }
            });
        });
    }

    if !sgdb_games.is_empty() {
        std::thread::spawn(move || {
            let _s = tracing::info_span!("background_sgdb_redownload", count = sgdb_games.len())
                .entered();
            for (db_id, sgdb_id, kind, app_id, trophy_source) in sgdb_games {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let dir = if kind == ira_models::GameKind::Ps3 {
                        ira_parser::ps3_data_dir(&save_dir, &app_id)
                    } else if kind == ira_models::GameKind::Retro {
                        ira_parser::retro_data_dir(&save_dir, db_id)
                    } else if trophy_source.has_steam_enrichment() {
                        ira_parser::data_dir(&save_dir, &app_id)
                    } else {
                        ira_parser::sgdb_data_dir(&save_dir, &sgdb_id)
                    };
                    steam.ensure_sgdb_assets_in_dir(&dir, &sgdb_id)
                }));
                if let Ok((icon, hero, grid, logo, header)) = result {
                    let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                        db_id,
                        sgdb_id,
                        icon,
                        hero,
                        grid,
                        logo,
                        header,
                    });
                }
            }
        });
    }
}

pub(super) fn apply_game_update(state: &SharedState, updated: Game) {
    let db_id = updated.db_id;

    let (game_for_display, game_for_grid, sidebar_update, variant_grid_updates) = {
        let mut s = state.borrow_mut();
        let Some(i) = s
            .games
            .iter()
            .position(|g| g.db_id == db_id && g.variant_id.is_none())
        else {
            return;
        };

        let was_placeholder = s.games[i].name.is_empty() || s.games[i].name.starts_with("App ID:");

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
            if let Err(e) = ira_db::update_achievement_counts(
                &db,
                updated.db_id,
                updated.earned_count as i64,
                updated.total_count as i64,
                mtime,
            ) {
                eprintln!("Failed to update achievement counts: {}", e);
            }
        }

        if was_placeholder && !updated.name.is_empty() && !updated.name.starts_with("App ID:") {
            let db = s.db.clone();
            if let Err(e) = ira_db::update_game_title(&db, updated.db_id, &updated.name) {
                eprintln!("Failed to persist game title: {}", e);
            }
        }

        s.game_names
            .lock()
            .unwrap()
            .insert(updated.app_id.clone(), updated.name.clone());

        let icon_path = if updated.icon_path.is_empty() {
            String::new()
        } else {
            updated.icon_path.clone()
        };
        let sidebar_update = (updated.db_id, updated.name.clone(), icon_path);

        let needs_rebuild = s.selected_id == updated.grid_id() && !s.content_unloaded;
        if s.displayed_db_id == updated.db_id && !needs_rebuild {
            s.displayed_content_dirty = true;
        }
        let counts_changed = updated.earned_count != old_earned || updated.total_count != old_total;
        let visual_changed = updated.grid_path != old_grid_path
            || updated.header_path != old_header_path
            || counts_changed;
        let needs_grid_update = visual_changed && !s.content_unloaded;
        let game_for_grid = if needs_grid_update {
            Some(updated.clone())
        } else {
            None
        };
        let game = if needs_rebuild {
            Some(updated.clone())
        } else {
            None
        };

        let sync_db_id = updated.db_id;
        let sync_achievements = updated.achievements.clone();
        let sync_earned = updated.earned_count;
        let sync_total = updated.total_count;
        let mut variant_grid_updates: Vec<Game> = Vec::new();
        {
            for g in &mut s.games {
                if g.db_id == sync_db_id && g.variant_id.is_some() {
                    let g_counts_changed =
                        g.earned_count != sync_earned || g.total_count != sync_total;
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

        let game = game.or_else(|| {
            if s.content_unloaded {
                return None;
            }
            s.games
                .iter()
                .find(|g| {
                    g.grid_id() == s.selected_id && g.db_id == sync_db_id && g.variant_id.is_some()
                })
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
        replace_grid_game(state, &g);
    }
    for vg in variant_grid_updates {
        replace_grid_game(state, &vg);
    }
}

pub(super) fn insert_or_update_game(state: &SharedState, game: Game) {
    let app_id = game.app_id.clone();
    let (db_id, is_existing) = {
        let mut s = state.borrow_mut();
        if !app_id.is_empty() {
            s.game_names
                .lock()
                .unwrap()
                .insert(app_id, game.name.clone());
        }

        let found = s.games.iter().position(|g| g.db_id == game.db_id);

        if let Some(i) = found {
            let mut g = game;
            g.hidden = s.games[i].hidden;
            g.manual_unmatch = s.games[i].manual_unmatch;
            if s.displayed_db_id == g.db_id {
                s.displayed_content_dirty = true;
            }
            s.games[i] = g;
            (s.games[i].db_id, true)
        } else {
            let db_id = game.db_id;
            s.games.push(game);
            let sort_mode = s.cfg.sort_mode;
            let sort_descending = s.cfg.sort_descending;
            s.games.sort_by(|a, b| {
                let ord = sort_mode.compare(a, b);
                if sort_descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
            (db_id, false)
        }
    };

    if state.borrow().content_unloaded {
        return;
    }

    if is_existing {
        let (name, icon_path) = {
            let s = state.borrow();
            let g = s.games.iter().find(|g| g.db_id == db_id);
            g.map(|g| (g.name.clone(), g.icon_path.clone()))
                .unwrap_or_default()
        };
        super::sidebar::update_sidebar_game(state, db_id, &name, &icon_path);
    } else {
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

    let game = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id && g.variant_id == variant_id)
        .cloned();
    if let Some(game) = game {
        display_game_cached(&game, state);

        if game.achievements.is_empty()
            && !game.app_id.is_empty()
            && game.trophy_source != ira_models::TrophySource::Empty
        {
            let (ra_username, ra_web_api_key, steam, sender, save_dir, db, cfg) = {
                let s = state.borrow();
                (
                    s.cfg.ra_username.clone(),
                    s.cfg.ra_web_api_key.clone(),
                    s.steam.clone(),
                    s.sender.clone(),
                    s.save_dir.clone(),
                    s.db.clone(),
                    s.cfg.clone(),
                )
            };
            enrich_game_async(crate::ui::enrichment::EnrichGameParams {
                app_id: game.app_id.clone(),
                trophy_source: game.trophy_source,
                platform_id: game.platform_id.clone(),
                db_id: game.db_id,
                title: game.name,
                steam,
                sender,
                save_dir,
                db,
                ra_username,
                ra_web_api_key,
                cfg,
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
                if ira_platforms::retroachievements::redownload_missing_ra_badges(
                    &save_dir, &app_id,
                ) {
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

pub(super) fn clear_content(state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let grid_header = state.borrow().grid_header.clone();
    clear_children(&content_box);
    clear_children(&grid_header);
}
