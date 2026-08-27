use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::enrichment::{enrich_game_async, EnrichGameParams};
use super::helpers::{clear_children, poll_channel, status_row};
use super::state::SharedState;
use super::steam_search_dialog::{build_search_dialog, match_result_row, SearchDialogWidgets};
use ira_platforms::retroachievements::api::{RaClient, RaGameEntry};

fn apply_ra_match(
    sc: &SharedState,
    db_id: i64,
    platform_id: &str,
    ra_id: u32,
    ra_title: &str,
    on_match: &Option<Rc<dyn Fn()>>,
    dialog: &adw::Dialog,
) {
    let app_id = ra_id.to_string();
    if let Err(e) = ira_db::update_game_ids(
        &sc.borrow().db,
        db_id,
        "",
        &app_id,
        ira_models::TrophySource::Ra,
        platform_id,
    ) {
        eprintln!("Failed to update game IDs for RA match: {}", e);
    }
    if let Err(e) = ira_db::set_manual_unmatch(&sc.borrow().db, db_id, false) {
        eprintln!("Failed to clear manual unmatch: {}", e);
    }
    if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
        g.app_id = app_id;
        g.trophy_source = ira_models::TrophySource::Ra;
        if !ra_title.is_empty() {
            g.set_name(ra_title);
        }
        g.total_count = 0;
        g.achievements.clear();
    }
    if let Some(ref sd) = sc.borrow().settings_data {
        if sd.db_id == db_id {
            let key = format!("__ra_unmatch_{}", db_id);
            sd.pending_copies.borrow_mut().remove(&key);
        }
    }
    let (ra_username, ra_web_api_key, steam, sender, save_dir, db) = {
        let s = sc.borrow();
        (
            s.cfg.ra_username.clone(),
            s.cfg.ra_web_api_key.clone(),
            s.steam.clone(),
            s.sender.clone(),
            s.save_dir.clone(),
            s.db.clone(),
        )
    };
    let g = sc.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
    if let Some(g) = g {
        enrich_game_async(EnrichGameParams {
            app_id: g.app_id.clone(),
            trophy_source: g.trophy_source,
            platform_id: g.platform_id.clone(),
            db_id: g.db_id,
            title: g.name,
            steam,
            sender,
            save_dir,
            db,
            ra_username,
            ra_web_api_key,
            game: None,
        });
    }
    if let Some(ref cb) = on_match {
        cb();
    }
    dialog.close();
    let sc_refresh = sc.clone();
    glib::idle_add_local_once(move || {
        super::game_settings::refresh_ra_section(&sc_refresh, db_id);
        super::helpers::refresh_settings_images_page(
            &sc_refresh,
            db_id,
            |s, game, win, pc, scache| {
                super::image_manager::build_image_manager_content_with_drafts(
                    s, game, win, pc, scache,
                )
                .upcast()
            },
        );
    });
}

fn populate_results(
    list: &gtk4::ListBox,
    state: &SharedState,
    db_id: i64,
    platform_id: &str,
    dialog: &adw::Dialog,
    on_match: &Option<Rc<dyn Fn()>>,
    outcome: (Option<String>, Option<RaGameEntry>, Vec<RaGameEntry>),
) {
    let (notice, hash_hit, results) = outcome;
    clear_children(list);
    if let Some(notice) = notice {
        list.append(&status_row(&notice));
        return;
    }
    let mut rows: Vec<(RaGameEntry, bool)> = Vec::new();
    if let Some(hit) = hash_hit {
        rows.push((hit, true));
    }
    for game in results {
        if rows.iter().any(|(g, _)| g.id == game.id) {
            continue;
        }
        rows.push((game, false));
    }
    if rows.is_empty() {
        list.append(&status_row(&crate::tr!("No results found")));
        return;
    }
    for (game, exact_hash) in rows {
        let tag = if exact_hash {
            crate::tr!("Exact hash match")
        } else if game.num_achievements == 0 {
            crate::tr!("No achievements yet")
        } else {
            crate::tr!("{} achievements").replacen("{}", &game.num_achievements.to_string(), 1)
        };
        let sc = state.clone();
        let dc = dialog.clone();
        let ra_id = game.id;
        let ra_title = game.title.clone();
        let on_match_c = on_match.clone();
        let pid = platform_id.to_string();
        let row = match_result_row(
            &game.title,
            &format!("RA ID: {} · {}", game.id, tag),
            move || apply_ra_match(&sc, db_id, &pid, ra_id, &ra_title, &on_match_c, &dc),
        );
        list.append(&row);
    }
}

pub fn show_ra_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    parent: &impl IsA<gtk4::Widget>,
    on_match: Option<Rc<dyn Fn()>>,
) {
    let console_id = match ira_models::find_console(platform_id) {
        Some(c) => c.ra_console_id,
        None => return,
    };
    let rom_hash = {
        let s = state.borrow();
        ira_db::find_by_db_id(&s.db, db_id)
            .ok()
            .flatten()
            .map(|e| e.rom_hash)
            .unwrap_or_default()
    };

    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Match to RetroAchievements"),
        500,
        400,
        500,
        game_name,
        Some(&crate::tr!("Game name…")),
    );

    let state_c = state.clone();
    let platform_id = platform_id.to_string();

    let entry_c = entry.clone();
    let dialog_c = dialog.clone();
    let do_search = move || {
        let term = entry_c.text().trim().to_string();
        if term.is_empty() && rom_hash.is_empty() {
            return;
        }
        let (cfg, save_dir) = {
            let s = state_c.borrow();
            (s.cfg.clone(), s.save_dir.clone())
        };
        let (tx, rx) = mpsc::channel::<(Option<String>, Option<RaGameEntry>, Vec<RaGameEntry>)>();
        let rom_hash_c = rom_hash.clone();
        std::thread::spawn(move || {
            let (notice, hash_hit, results) = match RaClient::from_config(&cfg) {
                Some(client) => {
                    let hash_hit = client.find_game_by_hash(&save_dir, console_id, &rom_hash_c);
                    let results = if term.is_empty() {
                        Vec::new()
                    } else {
                        client.search_ra_games(&save_dir, console_id, &term)
                    };
                    (None, hash_hit, results)
                }
                None => (
                    Some(crate::tr!("RetroAchievements credentials not configured")),
                    None,
                    Vec::new(),
                ),
            };
            let _ = tx.send((notice, hash_hit, results));
        });
        let list_c2 = list.clone();
        let state_c2 = state_c.clone();
        let dialog_c2 = dialog_c.clone();
        let on_match_c2 = on_match.clone();
        let platform_id_c2 = platform_id.clone();
        poll_channel(rx, move |outcome| {
            populate_results(
                &list_c2,
                &state_c2,
                db_id,
                &platform_id_c2,
                &dialog_c2,
                &on_match_c2,
                outcome,
            );
        });
    };

    let do_search = Rc::new(do_search);
    entry.connect_activate({
        let ds = do_search.clone();
        move |_| ds()
    });
    search_btn.connect_clicked({
        let ds = do_search.clone();
        move |_| ds()
    });

    dialog.present(Some(parent));
    do_search();
}
