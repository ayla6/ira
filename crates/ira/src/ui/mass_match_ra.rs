//! The mass matcher's RetroAchievements pass: retro ROMs on RA-covered
//! consoles are resolved the way the library scan does — exact hash, then
//! an exact normalized title — and hits are persisted as they land.

use adw::prelude::*;
use std::rc::Rc;
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchHit, BatchItem, RowActions};
use super::ra_match_dialog::{persist_ra_match, show_ra_search_dialog};
use super::state::SharedState;
use super::steam_search_dialog::status_label;
use crate::Game;
use ira_platforms::retroachievements::api::RaClient;

/// The RA status line of a match-list row: the attach fills the line's
/// action box, which the row builder already placed under an RA label.
/// Starts at "Searching…" when a pass will run, else straight at
/// not-matched with the manual search button.
pub(super) fn attach_ra_actions(
    ra_box: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    dialog: &gtk4::Widget,
    will_search: bool,
) {
    if will_search {
        ra_box.append(&status_label(&crate::tr!("Searching…"), CSS_DIM_LABEL));
    } else {
        // Nothing will run for this row: terminal from the start.
        show_unmatched(ra_box, state, game.db_id, &game.name, &game.platform_id, dialog);
    }
}

/// Whether the RA pass can run at all: the integration is on and the
/// credentials it needs are configured.
pub(super) fn ra_pass_available(state: &SharedState) -> bool {
    let s = state.borrow();
    s.cfg.ra_enabled && RaClient::from_config(&s.cfg).is_some()
}

/// The row's not-matched end state: the manual search button says it —
/// a "not matched" label beside it would only add text.
fn show_unmatched(
    ra_box: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &gtk4::Widget,
) {
    let btn = gtk4::Button::with_label(&crate::tr!("Search"));
    btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    let gn = game_name.to_string();
    let pid = platform_id.to_string();
    let dlg = dialog.clone();
    let inner = ra_box.clone();
    btn.connect_clicked(move |_| {
        let inner = inner.clone();
        let sc2 = sc.clone();
        show_ra_search_dialog(
            &sc,
            db_id,
            &gn,
            &pid,
            &dlg,
            // A manual pick shows the same id link as an automatic one —
            // without a veto, since the user just chose it.
            Some(Rc::new(move || {
                let matched = ira_db::find_by_db_id(&sc2.borrow().db, db_id)
                    .ok()
                    .flatten()
                    .map(|entry| (entry.trophy_id, entry.title))
                    .unwrap_or_default();
                if !matched.0.is_empty() {
                    paint_ra_link(&inner, &matched.0, &matched.1);
                }
            })),
        );
    });
    ra_box.append(&btn);
}

fn paint_ra_link(ra_box: &gtk4::Box, id: &str, title: &str) {
    replace_row_actions(ra_box, |ab| {
        ab.append(&super::mass_match_dialog::match_id_link(
            super::mass_match_dialog::MatchSource::Ra,
            id,
            title,
        ));
    });
}

/// Unmatch for an automatically applied RA match: clear the ids and block
/// every automatic pass like the settings unmatch does, then offer the
/// manual search again. Manual picks repaint through `show_unmatched`
/// without a veto — the user just chose them.
fn append_ra_veto(
    state: &SharedState,
    ra_box: &gtk4::Box,
    db_id: i64,
    platform_id: &str,
    game_name: &str,
    dialog: &gtk4::Widget,
) {
    let unmatch = gtk4::Button::with_label(&crate::tr!("Unmatch"));
    unmatch.add_css_class(CSS_DESTRUCTIVE_ACTION);
    unmatch.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let ra_box = ra_box.clone();
        let platform_id = platform_id.to_string();
        let game_name = game_name.to_string();
        let dialog = dialog.clone();
        unmatch.connect_clicked(move |_| {
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::update_game_ids(
                &db,
                db_id,
                "",
                "",
                ira_models::TrophySource::Empty,
                &platform_id,
            ) {
                eprintln!("RA veto: failed to clear the match: {e}");
            }
            if let Err(e) = ira_db::set_manual_unmatch(&db, db_id, true) {
                eprintln!("RA veto: failed to block re-matching: {e}");
            }
            if let Some(g) = state
                .borrow_mut()
                .games
                .iter_mut()
                .find(|g| g.db_id == db_id)
            {
                g.app_id.clear();
                g.trophy_source = ira_models::TrophySource::Empty;
                g.achievements.clear();
                g.earned_count = 0;
                g.total_count = 0;
                g.manual_unmatch = true;
            }
            show_unmatched(&ra_box, &state, db_id, &game_name, &platform_id, &dialog);
        });
    }
    ra_box.append(&unmatch);
}

/// Runs the RA pass over every row that has an RA box. The worker resolves
/// each game from its stored hash and its title/ROM name against the
/// console's RA list (fetched on demand); hits are persisted as they land
/// and misses get the manual search button.
pub(super) fn start_ra_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    vis: super::mass_match_dialog::RowVis,
) {
    if !ra_pass_available(state) {
        return;
    }
    let (cfg, save_dir, db) = {
        let s = state.borrow();
        (s.cfg.clone(), s.save_dir.clone(), s.db.clone())
    };
    let Some(client) = RaClient::from_config(&cfg) else {
        return;
    };
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(i, _)| rows.get(*i).is_some_and(|r| r.ra.is_some()))
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
        })
        .collect();
    if queue.is_empty() {
        return;
    }

    let client = Arc::new(client);
    run_batch(
        queue,
        0,
        None,
        move |item| resolve(&client, &db, &save_dir, item),
        {
            let state = state.clone();
            let rows = rows.to_vec();
            let dialog = dialog.clone();
            move |hit| apply_hit(&state, &rows, &dialog, hit, &vis)
        },
    );
}

/// Off-thread: console, stored hash and candidate names come from the DB
/// row. The display title is tried first, then the stored title and the
/// ROM file name, so a renamed game still resolves through its file.
fn resolve(
    client: &RaClient,
    db: &ira_db::DbConn,
    save_dir: &str,
    item: &BatchItem,
) -> Option<(String, String)> {
    let entry = ira_db::find_by_db_id(db, item.db_id).ok().flatten()?;
    let console_id = ira_models::find_console(&entry.platform_id)?.ra_console_id;
    if console_id == 0 {
        return None;
    }
    let stem = std::path::Path::new(&entry.rom_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let names = [item.name.as_str(), entry.title.as_str(), stem.as_str()];
    client
        .match_ra_game(save_dir, console_id, entry.hashes.ra_key(), &names)
        .map(|g| (g.id.to_string(), g.title))
}

/// UI loop: persist a hit and repaint the row's RA box either way.
fn apply_hit(
    state: &SharedState,
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    hit: BatchHit<(String, String)>,
    vis: &super::mass_match_dialog::RowVis,
) {
    let Some(ra_box) = rows.get(hit.row_idx).and_then(|r| r.ra.clone()) else {
        return;
    };
    let platform_id = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == hit.db_id)
        .map(|g| g.platform_id.clone())
        .unwrap_or_default();
    match hit.matched {
        Some((id, title)) => {
            match id.parse::<u32>() {
                Ok(trophy_id) => persist_ra_match(state, hit.db_id, &platform_id, trophy_id, &title),
                Err(e) => eprintln!("RA batch: bad game id {id:?}: {e}"),
            }
            paint_ra_link(&ra_box, &id, &title);
            append_ra_veto(state, &ra_box, hit.db_id, &platform_id, &hit.name, dialog);
        }
        None => replace_row_actions(&ra_box, |ab| {
            show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
        }),
    }
    vis.source_done(hit.row_idx, super::mass_match_dialog::MatchSource::Ra);
}
