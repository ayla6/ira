//! The mass matcher's ScreenScraper pass: console games are resolved
//! through their stored ROM md5 — the exact-match search — and the
//! metadata a hit carries (dates, companies, genres, players, synopses)
//! is persisted as it lands. Misses stay dim labels: there is no manual
//! ScreenScraper search dialog to offer yet.

use adw::prelude::*;
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchHit, BatchItem, RowActions};
use super::state::SharedState;
use super::steam_search_dialog::status_label;
use crate::Game;
use ira_api::screenscraper::ScrapedGame;
use ira_api::{ScraperCreds, SteamDataClient};
use ira_models::screenscraper_system_id;

/// The status box of a match-list row's ScreenScraper pass, added as its
/// own suffix so the Steam/SGDB/RA boxes stay independent. The pass runs
/// for every row it is attached to, so it starts at "Searching".
pub(super) fn attach_ss_actions(row: &adw::ActionRow) -> gtk4::Box {
    let ss_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    ss_box.set_valign(gtk4::Align::Center);
    ss_box.append(&status_label(
        &crate::tr!("Searching ScreenScraper..."),
        CSS_DIM_LABEL,
    ));
    row.add_suffix(&ss_box);
    ss_box
}

/// Runs the ScreenScraper pass over every row that has an SS box. One
/// request per second keeps the batch well inside the service's quota.
pub(super) fn start_ss_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
) {
    let (steam, db, creds) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.db.clone(),
            ScraperCreds::from_account(s.cfg.screenscraper_id.clone(), s.cfg.screenscraper_password.clone()),
        )
    };
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(i, _)| rows.get(*i).is_some_and(|r| r.ss.is_some()))
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
        })
        .collect();
    if queue.is_empty() {
        return;
    }

    run_batch(
        queue,
        150,
        1000,
        {
            let steam = Arc::clone(&steam);
            let db = db.clone();
            move |item| resolve(&steam, &creds, &db, item)
        },
        {
            let state = state.clone();
            let rows = rows.to_vec();
            move |hit| apply_hit(&state, &rows, hit)
        },
    );
}

/// Off-thread: console, ROM name and md5 come from the DB row, and the
/// exact-match search answers authoritatively when the hash is on record.
fn resolve(
    steam: &SteamDataClient,
    creds: &ScraperCreds,
    db: &ira_db::DbConn,
    item: &BatchItem,
) -> Option<ScrapedGame> {
    let entry = ira_db::find_by_db_id(db, item.db_id).ok().flatten()?;
    let platform_id = entry.platform_id.clone();
    screenscraper_system_id(&platform_id)?;
    // The hash search needs the digest and the file size together; a
    // missing file or hash falls back to the name-only exact search.
    let md5 = (!entry.rom_hash.is_empty())
        .then(|| std::fs::metadata(&entry.rom_path).ok().map(|m| m.len()))
        .flatten()
        .map(|size| (entry.rom_hash.clone(), size));
    let romnom = std::path::Path::new(&entry.rom_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| item.name.clone());
    let games = steam.screenscraper_rom_lookup(
        creds,
        &romnom,
        &platform_id,
        md5.as_ref().map(|(hash, size)| (hash.as_str(), *size)),
    )
    .ok()?;
    games.into_iter().next()
}

/// UI loop: persist a hit's metadata and repaint the row's SS box.
fn apply_hit(state: &SharedState, rows: &[RowActions], hit: BatchHit<ScrapedGame>) {
    let Some(ss_box) = rows.get(hit.row_idx).and_then(|r| r.ss.clone()) else {
        return;
    };
    match hit.matched {
        Some(game) => {
            let timestamp = ira_db::scraper_release_timestamp(&game.release_date);
            if let Err(e) =
                ira_db::store_scraper_metadata(&state.borrow().db, hit.db_id, &game.metadata(timestamp))
            {
                eprintln!("ScreenScraper batch: failed to store metadata: {e}");
            }
            if let Some(g) = state
                .borrow_mut()
                .games
                .iter_mut()
                .find(|g| g.db_id == hit.db_id)
            {
                g.screenscraper_id = game.ss_id.clone();
            }
            replace_row_actions(&ss_box, |ab| {
                ab.append(&status_label(&crate::tr!("SS: matched"), CSS_SUCCESS_LABEL));
            });
        }
        None => replace_row_actions(&ss_box, |ab| {
            ab.append(&status_label(&crate::tr!("SS: not matched"), CSS_DIM_LABEL));
        }),
    }
}
