//! Manual ScreenScraper matching: search the source by title, and persist
//! the picked game's whole metadata — ids, dates, companies, genres,
//! synopsis — to the entry. A pick also clears any recorded miss.

use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::css::*;
use super::helpers::{clear_children, poll_channel, replace_row_actions, status_row};
use super::rom_name::{clean_rom_name, looks_like_title_id};
use super::steam_search_dialog::{
    build_search_dialog, match_result_row, status_label, SearchDialogWidgets,
};
use super::state::SharedState;
use ira_api::screenscraper::ScrapedGame;
use ira_api::ScraperCreds;

/// Where a picked match lands. The game settings' dialog stages the pick
/// onto its draft — nothing is written until Save, and the closure is the
/// scraper section's own staging step — while the batch matchers (mass
/// matcher, auto-add wizard) have no Save button and persist at once.
pub type StagePick = Rc<dyn Fn(&SharedState, i64, &ScrapedGame)>;

#[derive(Clone)]
pub enum SsMatchSink {
    Persist,
    Stage(StagePick),
}

/// Store a picked ScreenScraper game on the entry: the metadata write also
/// refreshes the company/genre lookup tables, so every pick widens the
/// local entity cache future pickers search first. The match wins the
/// scalar fields it has data for; companies, genres and age ratings the
/// entry already carries are mixed together with the answer, never
/// replaced by it.
pub(super) fn persist_ss_match(state: &SharedState, db_id: i64, picked: &ScrapedGame) {
    let timestamp = ira_db::scraper_release_timestamp(&picked.release_date);
    let mut merged = ira_db::scraper_metadata_for_game(&state.borrow().db, db_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    merged.merge_match(&picked.metadata(timestamp));
    if let Err(e) = ira_db::store_scraper_metadata(&state.borrow().db, db_id, &merged) {
        eprintln!("Failed to store ScreenScraper metadata: {e}");
        return;
    }
    if let Err(e) = ira_db::clear_scraper_miss(&state.borrow().db, db_id) {
        eprintln!("Failed to clear the ScreenScraper miss marker: {e}");
    }
    // The SS title is authoritative for consoles whose own names came
    // from file stems or shortened ROM headers; trusted sources (official
    // console headers, RA, the user's own edits) keep theirs.
    let replace_title = entry_title_trusted(state, db_id)
        .map(|trusted| !trusted)
        .unwrap_or(false);
    let mut new_title = None;
    if replace_title && !picked.name.is_empty() {
        if let Err(e) = ira_db::update_game_title(&state.borrow().db, db_id, &picked.name) {
            eprintln!("Failed to store the ScreenScraper title: {e}");
        } else if let Err(e) = ira_db::set_title_trusted(&state.borrow().db, db_id, true) {
            eprintln!("Failed to mark the title trusted: {e}");
        } else {
            new_title = Some(picked.name.clone());
        }
    }
    if let Some(g) = state
        .borrow_mut()
        .games
        .iter_mut()
        .find(|g| g.db_id == db_id)
    {
        g.screenscraper_id = picked.ss_id.clone();
        if let Some(title) = new_title {
            g.set_name(title);
        }
    }
}

/// The search dialog starts from the game's own title when the row's
/// name comes from a trusted source or the console carries
/// authoritative internal titles (switch); every other console prefills
/// the ROM file's name — the library title there is user-editable and
/// drifts from the dump. The cleaner takes the dump tags off either way,
/// and a name that is only a bare title id falls through to the other.
fn rom_stem(state: &SharedState, db_id: i64) -> Option<String> {
    let entry = ira_db::find_by_db_id(&state.borrow().db, db_id).ok().flatten()?;
    // PS3/PS4 games carry a native product code here, not a console.
    let console = ira_models::scraper_console_id(entry.kind, &entry.platform_id);
    // PC titles come from Steam or the user — search from them; dump
    // stems there are executable names at best. Product-code platforms'
    // titles come from the console's own metadata, so they lead too —
    // their stems are directory names at best.
    let trusted = entry.title_trusted
        || entry.kind.is_pc()
        || ira_models::title_from_trusted_source(&console);
    let stem = clean_rom_name(
        &std::path::Path::new(&entry.rom_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let title = clean_rom_name(&entry.title);
    let (first, second) = if trusted {
        (Some(title), Some(stem))
    } else {
        (Some(stem), Some(title))
    };
    let usable = |s: &String| !s.is_empty() && !looks_like_title_id(s);
    first.filter(|s| usable(s)).or_else(|| second.filter(|s| usable(s)))
}

/// Whether the row's current title is already authoritative: edited by the
/// user, from an RA match, or from an official console header.
pub(super) fn entry_title_trusted(state: &SharedState, db_id: i64) -> Option<bool> {
    Some(
        ira_db::find_by_db_id(&state.borrow().db, db_id)
            .ok()
            .flatten()?
            .title_trusted
            || ira_models::title_from_trusted_source(
                &state.borrow().games.iter().find(|g| g.db_id == db_id)?.platform_id,
            ),
    )
}

fn apply_ss_match(
    state: &SharedState,
    db_id: i64,
    picked: ScrapedGame,
    sink: &SsMatchSink,
    on_match: &Option<Rc<dyn Fn()>>,
    dialog: &adw::Dialog,
) {
    match sink {
        SsMatchSink::Persist => persist_ss_match(state, db_id, &picked),
        SsMatchSink::Stage(stage) => stage(state, db_id, &picked),
    }
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
    sink: &SsMatchSink,
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
        let sink_c = sink.clone();
        let on_match_c = on_match.clone();
        // The console matters most on the wide PC search, where the
        // candidates come from every system at once.
        let mut subtitle = crate::tr!("SS ID: {}").replacen("{}", &game.ss_id, 1);
        if !game.system_name.is_empty() {
            subtitle.push_str(" · ");
            subtitle.push_str(&game.system_name);
        }
        if !game.release_date.is_empty() {
            subtitle.push_str(" · ");
            subtitle.push_str(&game.release_date);
        }
        let row = match_result_row(&game.name, &subtitle, {
            let game = game.clone();
            move || apply_ss_match(&sc, db_id, game.clone(), &sink_c, &on_match_c, &dc)
        });
        // The row's background opens the entry's page on the site.
        let uri = ira_api::screenscraper::game_page_url(&game.ss_id);
        row.set_activatable(true);
        row.connect_activated(move |row| {
            super::helpers::open_uri(row.upcast_ref(), &uri);
        });
        list.append(&row);
    }
}

/// Search ScreenScraper by title and let the user pick the match for
/// `db_id`. The prefill is a starting point, nothing more — the request
/// only goes out when the user searches. `sink` decides whether a pick
/// persists at once or stages onto the settings dialog's draft; `on_match`
/// runs after a pick lands, so callers can repaint their row.
pub fn show_ss_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    parent: &impl IsA<gtk4::Widget>,
    sink: SsMatchSink,
    on_match: Option<Rc<dyn Fn()>>,
) {
    // A staged match may sit on an entry the database still calls
    // matched (an unmatch pending on Save), so only the persist sink —
    // the batch matchers, whose rows only exist for unmatched games —
    // needs the guard.
    if matches!(sink, SsMatchSink::Persist) {
        if let Ok(Some(entry)) = ira_db::find_by_db_id(&state.borrow().db, db_id) {
            if !entry.screenscraper_id.is_empty() {
                eprintln!("SS search: game {db_id} is already matched");
                return;
            }
        }
    }
    // The master switch outranks every entry point into the source — a
    // disabled ScreenScraper means no requests at all, manual included.
    let disabled = !state.borrow().cfg.screenscraper_enabled;
    // Say which console the search is scoped to.
    let entry = ira_db::find_by_db_id(&state.borrow().db, db_id).ok().flatten();
    let console = match &entry {
        Some(entry) if entry.kind.is_pc() => crate::tr!("PC"),
        _ => ira_models::find_console(platform_id)
            .map(|def| def.display_name.to_string())
            .unwrap_or_else(|| platform_id.to_string()),
    };
    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Match to ScreenScraper · {}").replacen("{}", &console, 1),
        500,
        400,
        500,
        &rom_stem(state, db_id).unwrap_or_else(|| game_name.to_string()),
        Some(&crate::tr!("Game name…")),
    );
    // Clicking a result's background opens its page on the site — the
    // way to check what a candidate actually is before matching it.
    // (Rows activate themselves; no list wiring needed.)
    if disabled {
        list.append(&status_row(&crate::tr!(
            "ScreenScraper is disabled in Settings"
        )));
    }

    let state_c = state.clone();
    let platform_id = platform_id.to_string();

    let entry_c = entry.clone();
    let dialog_c = dialog.clone();
    let do_search = move || {
        let term = entry_c.text().trim().to_string();
        if term.is_empty() {
            return;
        }
        if !state_c.borrow().cfg.screenscraper_enabled {
            clear_children(&list);
            list.append(&status_row(&crate::tr!(
                "ScreenScraper is disabled in Settings"
            )));
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
        // The request runs off-thread; say so instead of leaving the
        // previous results (or an empty list) looking frozen.
        clear_children(&list);
        list.append(&status_row(&crate::tr!("Searching ScreenScraper…")));
        let (tx, rx) = mpsc::channel::<Result<Vec<ScrapedGame>, String>>();
        let platform_id_c = platform_id.clone();
        std::thread::spawn(move || {
            let outcome = steam
                .screenscraper_search(&creds, &term, &platform_id_c)
                .map(|mut games| {
                    ira_api::screenscraper::sort_by_similarity(&mut games, &term);
                    games
                });
            let _ = tx.send(outcome);
        });
        let list_c = list.clone();
        let state_c2 = state_c.clone();
        let dialog_c2 = dialog_c.clone();
        let sink_c2 = sink.clone();
        let on_match_c2 = on_match.clone();
        poll_channel(rx, move |outcome| {
            populate_results(
                &list_c,
                &state_c2,
                db_id,
                &dialog_c2,
                &sink_c2,
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
    // The prefill is already the best query — run it on open. A disabled
    // source just gets the note above; the search button stays for form.
    if !disabled {
        do_search();
    }
    search_btn.connect_clicked({
        let ds = do_search.clone();
        move |_| ds()
    });

    dialog.present(Some(parent));
}

/// A dim "not matched" label plus the manual search button, shown on a
/// mass matcher row's ScreenScraper box once the pass is done with it.
pub(super) fn show_unmatched(
    ss_box: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &gtk4::Widget,
) {
    replace_row_actions(ss_box, |ab| {
        unmatched_actions(ab, "", state, db_id, game_name, platform_id, dialog);
    });
}

/// The pass is still running in the background (the dialog was reopened
/// mid-job): an in-flight note instead of "not matched", same manual
/// search button.
pub(super) fn show_background_match(
    ss_box: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &gtk4::Widget,
) {
    replace_row_actions(ss_box, |ab| {
        unmatched_actions(
            ab,
            &crate::tr!("Matching in background…"),
            state,
            db_id,
            game_name,
            platform_id,
            dialog,
        );
    });
}

/// The end state of a mass matcher row's ScreenScraper box. `label` is
/// only for states that still need words — the background-match note —
/// since a plain miss says it with the search button alone: less text
/// per row, and the button is the affordance that matters.
fn unmatched_actions(
    ab: &gtk4::Box,
    label: &str,
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    dialog: &gtk4::Widget,
) {
    if !label.is_empty() {
        ab.append(&status_label(label, CSS_DIM_LABEL));
    }
    let btn = gtk4::Button::with_label(&crate::tr!("Search SS"));
    btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    let gn = game_name.to_string();
    let pid = platform_id.to_string();
    let dlg = dialog.clone();
    let inner = ab.clone();
    btn.connect_clicked(move |_| {
        let inner = inner.clone();
        show_ss_search_dialog(
            &sc,
            db_id,
            &gn,
            &pid,
            &dlg,
            SsMatchSink::Persist,
            Some(Rc::new(move || show_matched(&inner))),
        );
    });
    ab.append(&btn);
}

pub(super) fn show_matched(ss_box: &gtk4::Box) {
    replace_row_actions(ss_box, |ab| {
        ab.append(&status_label(&crate::tr!("SS: matched"), CSS_SUCCESS_LABEL));
    });
}
