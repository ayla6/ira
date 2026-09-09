//! ScreenScraper metadata matching: search screenscraper.fr for a game's
//! entries and store the picked one's metadata — developer, publisher,
//! genre, release date, players, synopsis. PS1 games additionally take a
//! square image from the match's media, the one art that source is
//! uniquely good for.

use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::helpers::{clear_children, poll_channel, status_row};
use super::state::SharedState;
use super::steam_search_dialog::{
    build_search_dialog, match_result_row, SearchDialogWidgets,
};
use ira_api::screenscraper::{ScraperCreds, ScrapedGame};

fn scraper_creds(state: &SharedState) -> ScraperCreds {
    let s = state.borrow();
    ScraperCreds {
        dev_id: s.cfg.screenscraper_dev_id.clone(),
        dev_password: s.cfg.screenscraper_dev_password.clone(),
        user: s.cfg.screenscraper_id.clone(),
        password: s.cfg.screenscraper_password.clone(),
    }
}

/// Store the picked match and, for PS1, fetch its square image in the
/// background (screenscraper's media is the one art PS1 entries lack).
fn apply_match(
    state: &SharedState,
    db_id: i64,
    game: ScrapedGame,
    dialog: &adw::Dialog,
    on_match: &Option<Rc<dyn Fn()>>,
) {
    let timestamp = ira_db::scraper_release_timestamp(&game.release_date);
    if let Err(e) =
        ira_db::store_scraper_metadata(&state.borrow().db, db_id, &game.metadata(timestamp))
    {
        eprintln!("Failed to store ScreenScraper metadata: {e}");
        return;
    }
    if let Some(cb) = on_match {
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
    outcome: (Option<String>, Vec<ScrapedGame>),
) {
    let (notice, results) = outcome;
    clear_children(list);
    if let Some(notice) = notice {
        list.append(&status_row(&notice));
        return;
    }
    if results.is_empty() {
        list.append(&status_row(&crate::tr!("No results found")));
        return;
    }
    for game in results {
        let mut subtitle_bits: Vec<String> = Vec::new();
        if let Some(year) = game.release_date.get(..4) {
            subtitle_bits.push(year.to_string());
        }
        if let Some(developer) = &game.developer {
            subtitle_bits.push(developer.name.clone());
        }
        if !game.players.is_empty() {
            subtitle_bits.push(
                crate::tr!("{} players").replacen("{}", &game.players, 1),
            );
        }
        subtitle_bits.push(format!("SS {}", game.ss_id));
        let sc = state.clone();
        let dc = dialog.clone();
        let om = on_match.clone();
        let name = game.name.clone();
        let row = match_result_row(&name, &subtitle_bits.join("  ·  "), move || {
            apply_match(&sc, db_id, game.clone(), &dc, &om);
        });
        list.append(&row);
    }
}

pub fn show_screenscraper_search_dialog(
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
        &crate::tr!("Match metadata on ScreenScraper"),
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
    let list_c = list.clone();
    let do_search = move || {
        let term = entry_c.text().trim().to_string();
        if term.is_empty() {
            return;
        }
        let steam = state_c.borrow().steam.clone();
        let creds = scraper_creds(&state_c);
        let pid = platform_id.clone();
        let (tx, rx) = mpsc::channel::<(Option<String>, Vec<ScrapedGame>)>();
        std::thread::spawn(move || {
            let outcome = match steam.screenscraper_search(&creds, &term, &pid) {
                Ok(games) => (None, games),
                Err(e) => (Some(e), Vec::new()),
            };
            let _ = tx.send(outcome);
        });
        let list_c2 = list.clone();
        let state_c2 = state_c.clone();
        let dialog_c2 = dialog_c.clone();
        let om_c2 = on_match.clone();
        poll_channel(rx, move |outcome| {
            populate_results(&list_c2, &state_c2, db_id, &dialog_c2, &om_c2, outcome);
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
    // Credentials checked once up front: an empty account should say so
    // instead of quietly returning no results.
    if !scraper_creds(state).is_configured() {
        clear_children(&list_c);
        list_c.append(&status_row(&crate::tr!(
            "ScreenScraper credentials not configured — set them in Settings → RetroAchievements"
        )));
        return;
    }
    do_search();
}
