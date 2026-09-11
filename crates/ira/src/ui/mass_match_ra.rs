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

/// The RA status box of a match-list row, added as a second suffix so the
/// Steam/SGDB result in the main box stays independent. Starts at
/// "Searching RA..." when a pass will run, else straight at not-matched
/// with the manual search button.
pub(super) fn attach_ra_actions(
    row: &adw::ActionRow,
    state: &SharedState,
    game: &Game,
    dialog: &adw::Dialog,
    will_search: bool,
) -> gtk4::Box {
    let ra_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    ra_box.set_valign(gtk4::Align::Center);
    if will_search {
        ra_box.append(&status_label(&crate::tr!("Searching RA..."), CSS_DIM_LABEL));
    } else {
        show_unmatched(&ra_box, state, game.db_id, &game.name, &game.platform_id, dialog);
    }
    row.add_suffix(&ra_box);
    ra_box
}

/// Whether the RA pass can run at all: the integration is on and the
/// credentials it needs are configured.
pub(super) fn ra_pass_available(state: &SharedState) -> bool {
    let s = state.borrow();
    s.cfg.ra_enabled && RaClient::from_config(&s.cfg).is_some()
}

fn show_unmatched(
    ra_box: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &adw::Dialog,
) {
    ra_box.append(&status_label(&crate::tr!("RA: not matched"), CSS_DIM_LABEL));
    let btn = gtk4::Button::with_label(&crate::tr!("Search RA…"));
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    let sc = state.clone();
    let gn = game_name.to_string();
    let pid = platform_id.to_string();
    let dlg = dialog.clone();
    let inner = ra_box.clone();
    btn.connect_clicked(move |_| {
        let inner = inner.clone();
        show_ra_search_dialog(
            &sc,
            db_id,
            &gn,
            &pid,
            &dlg,
            Some(Rc::new(move || show_matched(&inner))),
        );
    });
    ra_box.append(&btn);
}

fn show_matched(ra_box: &gtk4::Box) {
    replace_row_actions(ra_box, |ab| {
        ab.append(&status_label(&crate::tr!("RA: matched"), CSS_SUCCESS_LABEL));
    });
}

/// Runs the RA pass over every row that has an RA box. The worker resolves
/// each game from its stored hash and its title/ROM name against the
/// console's RA list (fetched on demand); hits are persisted as they land
/// and misses get the manual search button.
pub(super) fn start_ra_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    dialog: &adw::Dialog,
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
        150,
        0,
        move |item| resolve(&client, &db, &save_dir, item),
        {
            let state = state.clone();
            let rows = rows.to_vec();
            let dialog = dialog.clone();
            move |hit| apply_hit(&state, &rows, &dialog, hit)
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
        .match_ra_game(save_dir, console_id, &entry.rom_hash, &names)
        .map(|g| (g.id.to_string(), g.title))
}

/// UI loop: persist a hit and repaint the row's RA box either way.
fn apply_hit(state: &SharedState, rows: &[RowActions], dialog: &adw::Dialog, hit: BatchHit) {
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
                Ok(ra_id) => persist_ra_match(state, hit.db_id, &platform_id, ra_id, &title),
                Err(e) => eprintln!("RA batch: bad game id {id:?}: {e}"),
            }
            show_matched(&ra_box);
        }
        None => replace_row_actions(&ra_box, |ab| {
            show_unmatched(ab, state, hit.db_id, &hit.name, &platform_id, dialog);
        }),
    }
}
