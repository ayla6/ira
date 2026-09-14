//! Manual ScreenScraper matching: search the source by title, and persist
//! the picked game's whole metadata — ids, dates, companies, genres,
//! synopsis — to the entry. A pick also clears any recorded miss.

use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::css::*;
use super::helpers::{clear_children, poll_channel, replace_row_actions, status_row};
use super::steam_search_dialog::{
    build_search_dialog, match_result_row, status_label, SearchDialogWidgets,
};
use super::state::SharedState;
use ira_api::screenscraper::ScrapedGame;
use ira_api::ScraperCreds;

/// Store a picked ScreenScraper game on the entry: the metadata write also
/// refreshes the company/genre lookup tables, so every pick widens the
/// local entity cache future pickers search first.
pub(super) fn persist_ss_match(state: &SharedState, db_id: i64, picked: &ScrapedGame) {
    let timestamp = ira_db::scraper_release_timestamp(&picked.release_date);
    if let Err(e) =
        ira_db::store_scraper_metadata(&state.borrow().db, db_id, &picked.metadata(timestamp))
    {
        eprintln!("Failed to store ScreenScraper metadata: {e}");
        return;
    }
    if let Err(e) = ira_db::clear_scraper_miss(&state.borrow().db, db_id) {
        eprintln!("Failed to clear the ScreenScraper miss marker: {e}");
    }
    if let Some(g) = state
        .borrow_mut()
        .games
        .iter_mut()
        .find(|g| g.db_id == db_id)
    {
        g.screenscraper_id = picked.ss_id.clone();
    }
}

fn apply_ss_match(
    state: &SharedState,
    db_id: i64,
    picked: ScrapedGame,
    on_match: &Option<Rc<dyn Fn()>>,
    dialog: &adw::Dialog,
) {
    persist_ss_match(state, db_id, &picked);
    if let Some(ref cb) = on_match {
        cb();
    }
    dialog.close();
}

fn populate_results(
    list: &gtk4::ListBox,
    state: &SharedState,
    db_id: i64,
    dialog: &adw::Dialog,
    on_match: &Option<Rc<dyn Fn()>>,
    outcome: Result<Vec<ScrapedGame>, String>,
) {
    clear_children(list);
    let results = match outcome {
        Ok(results) => results,
        Err(e) => {
            list.append(&status_row(&e));
            return;
        }
    };
    if results.is_empty() {
        list.append(&status_row(&crate::tr!("No results found")));
        return;
    }
    for game in results {
        let sc = state.clone();
        let dc = dialog.clone();
        let on_match_c = on_match.clone();
        let subtitle = if game.release_date.is_empty() {
            format!("SS ID: {}", game.ss_id)
        } else {
            format!("SS ID: {} · {}", game.ss_id, game.release_date)
        };
        let row = match_result_row(&game.name, &subtitle, {
            let game = game.clone();
            move || apply_ss_match(&sc, db_id, game.clone(), &on_match_c, &dc)
        });
        list.append(&row);
    }
}

/// Search ScreenScraper by title and let the user pick the match for
/// `db_id`. `on_match` runs after a pick is stored, so callers can repaint
/// their row.
pub fn show_ss_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    parent: &impl IsA<gtk4::Widget>,
    on_match: Option<Rc<dyn Fn()>>,
) {
    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Match to ScreenScraper"),
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
        if term.is_empty() {
            return;
        }
        let (steam, creds) = {
            let s = state_c.borrow();
            (
                s.steam.clone(),
                ScraperCreds::from_account(
                    s.cfg.screenscraper_id.clone(),
                    s.cfg.screenscraper_password.clone(),
                ),
            )
        };
        let (tx, rx) = mpsc::channel::<Result<Vec<ScrapedGame>, String>>();
        let platform_id_c = platform_id.clone();
        std::thread::spawn(move || {
            let outcome = steam.screenscraper_search(&creds, &term, &platform_id_c);
            let _ = tx.send(outcome);
        });
        let list_c = list.clone();
        let state_c2 = state_c.clone();
        let dialog_c2 = dialog_c.clone();
        let on_match_c2 = on_match.clone();
        poll_channel(rx, move |outcome| {
            populate_results(&list_c, &state_c2, db_id, &dialog_c2, &on_match_c2, outcome);
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

/// A dim "not matched" label plus the manual search button, shown on a
/// mass matcher row's ScreenScraper box once the pass is done with it.
pub(super) fn show_unmatched(
    ss_box: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &adw::Dialog,
) {
    ss_box.append(&status_label(&crate::tr!("SS: not matched"), CSS_DIM_LABEL));
    let btn = gtk4::Button::with_label(&crate::tr!("Search SS…"));
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    let gn = game_name.to_string();
    let pid = platform_id.to_string();
    let dlg = dialog.clone();
    let inner = ss_box.clone();
    btn.connect_clicked(move |_| {
        let inner = inner.clone();
        show_ss_search_dialog(
            &sc,
            db_id,
            &gn,
            &pid,
            &dlg,
            Some(Rc::new(move || show_matched(&inner))),
        );
    });
    ss_box.append(&btn);
}

pub(super) fn show_matched(ss_box: &gtk4::Box) {
    replace_row_actions(ss_box, |ab| {
        ab.append(&status_label(&crate::tr!("SS: matched"), CSS_SUCCESS_LABEL));
    });
}
