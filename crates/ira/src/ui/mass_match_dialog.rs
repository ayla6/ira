use crate::Game;
use adw::prelude::*;
use glib::clone::Downgrade;
use ira_models::GameKind;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchItem, RowActions, BATCH_FINISHED};
use super::mass_match_ra::{attach_ra_actions, ra_pass_available, start_ra_batch_matching};
use super::mass_match_ss::{attach_ss_actions, start_ss_batch_matching};
use super::sgdb_match_dialog::handle_unified_sgdb_result;
use super::state::SharedState;
use super::steam_search_dialog::{handle_steam_search_result, matched_text, status_label};

pub fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    let suffixes = [
        "the",
        "final",
        "cut",
        "edition",
        "complete",
        "definitive",
        "remastered",
        "hd",
    ];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

/// Games with nothing a name search can use: empty names and the
/// synthesized "App ID: …" placeholders for rows whose real title was
/// never learned. SGDB, SS and Steam-title passes all search by name —
/// firing them off for these rows returns noise, never matches.
fn has_searchable_name(name: &str) -> bool {
    !name.trim().is_empty() && !crate::game_loader::is_placeholder_name(name)
}

/// Games with no store or SGDB match at all: candidates for a Steam store
/// match. Console-emulator games and Retro ROMs are excluded — their names
/// come from title ids/ROM files, so Steam search is noise; they are
/// enriched through SGDB (and RA) instead. Gates on the match id, not
/// `app_id`: platform-linked games (Goldberg appids, title ids) carry an
/// `app_id` with no match behind it and must still qualify.
fn needs_steam_match(g: &Game) -> bool {
    g.steam_id.is_empty()
        && g.sgdb_id.is_empty()
        && !g.manual_unmatch
        && !g.kind.is_console_emulator()
        && g.kind != ira_models::GameKind::Retro
}

/// Games an SGDB match can enrich: everything without an SGDB id that has
/// no Steam-driven enrichment path (console-emulator games, Retro ROMs, and
/// games with no matches at all). A Steam match wins over SGDB, but a bare
/// platform linkage (Goldberg appid, title id) is not a match: those games
/// still qualify. Rows without a searchable name sit out — an SGDB name
/// search for "App ID: …" can only return junk.
fn needs_sgdb_match(g: &Game) -> bool {
    g.sgdb_id.is_empty()
        && !g.manual_unmatch
        && has_searchable_name(&g.name)
        && (g.steam_id.is_empty()
            || g.kind == ira_models::GameKind::Retro
            || g.kind.is_console_emulator())
}

/// Games the RetroAchievements matcher can serve: matched by ROM hash on
/// platforms RA actually covers — the Switch has no RA support at all.
fn needs_ra_match(g: &Game) -> bool {
    g.kind == ira_models::GameKind::Retro
        && g.trophy_source == ira_models::TrophySource::Empty
        && !g.manual_unmatch
        && ira_models::console_has_ra(&g.platform_id)
}

/// Games the ScreenScraper matcher can enrich: console games on platforms
/// ScreenScraper covers, until metadata from one is on record, plus PC
/// games — which search ScreenScraper's Windows/Linux systems and then
/// diff the whole source against their Steam data. Purely additive —
/// stored pieces only ever fill blanks. Unsearchable names sit out, same
/// as the SGDB pass.
fn needs_ss_match(g: &Game) -> bool {
    if g.manual_unmatch || !g.screenscraper_id.is_empty() || !has_searchable_name(&g.name) {
        return false;
    }
    let console = ira_models::scraper_console_id(g.kind, &g.platform_id);
    match g.kind {
        GameKind::Wine | GameKind::Linux | GameKind::Steam => true,
        _ => {
            (g.kind == GameKind::Retro || g.kind.is_console_emulator())
                && ira_models::screenscraper_system_id(&console).is_some()
        }
    }
}

/// Console games with no Steam link yet: the exact-title Steam pass
/// links their Steam identity and garnishes them, without ever
/// touching their console identity. Completeness of the ScreenScraper
/// record is no gate — a fully scraped game still wants its Steam
/// version found.
fn needs_steam_title_match(g: &Game) -> bool {
    !g.manual_unmatch
        && has_searchable_name(&g.name)
        && g.steam_id.is_empty()
        && (g.kind.is_console_emulator() || g.kind == ira_models::GameKind::Retro)
}

/// One matchable source with its own line on a mass-match row. Lines
/// render in this order; every line carries its source name once, so the
/// statuses themselves stay short ("Matched", "Searching…").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MatchSource {
    Steam,
    Sgdb,
    Ra,
    ScreenScraper,
    SteamLink,
}

/// The sources that apply to this game, in display order. Built from the
/// same `needs_*` gates as the batch queues, so a line exists exactly
/// where a pass can speak.
fn match_sources_for(game: &Game) -> Vec<MatchSource> {
    let mut sources = Vec::new();
    if needs_steam_match(game) {
        sources.push(MatchSource::Steam);
    }
    if needs_sgdb_match(game) {
        sources.push(MatchSource::Sgdb);
    }
    if needs_ra_match(game) {
        sources.push(MatchSource::Ra);
    }
    if needs_ss_match(game) {
        sources.push(MatchSource::ScreenScraper);
    }
    if needs_steam_title_match(game) {
        sources.push(MatchSource::SteamLink);
    }
    sources
}

fn source_label(source: MatchSource) -> String {
    match source {
        MatchSource::Steam => crate::tr!("Steam"),
        MatchSource::Sgdb => crate::tr!("SteamGridDB"),
        MatchSource::Ra => crate::tr!("RetroAchievements"),
        MatchSource::ScreenScraper => crate::tr!("ScreenScraper"),
        MatchSource::SteamLink => crate::tr!("Steam link"),
    }
}

/// Per-dialog visibility for the match list: rows whose game has
/// gotten a final word from every pass that applies to it (matched, or
/// a negative result this session) are hidden while the toggle is on,
/// so the list only shows games still in play.
#[derive(Clone)]
pub(super) struct RowVis {
    rows: RefCell<Vec<gtk4::ListBoxRow>>,
    games: Vec<Game>,
    show_finished: Rc<Cell<bool>>,
    attempted: Rc<RefCell<HashSet<i64>>>,
    scrolled: glib::WeakRef<gtk4::ScrolledWindow>,
}

impl RowVis {
    pub(super) fn new(
        games: Vec<Game>,
        show_finished: Rc<Cell<bool>>,
        scrolled: &gtk4::ScrolledWindow,
    ) -> Self {
        Self {
            rows: RefCell::new(Vec::new()),
            games,
            show_finished,
            attempted: Rc::new(RefCell::new(HashSet::new())),
            scrolled: Downgrade::downgrade(scrolled),
        }
    }

    /// One widget per game, in order; set right after the rows exist.
    pub(super) fn set_rows(&self, rows: Vec<gtk4::ListBoxRow>) {
        *self.rows.borrow_mut() = rows;
    }

    /// A pass reached a terminal result for this row — count it and
    /// hide the row when nothing applicable is left open.
    pub fn pass_done(&self, row_idx: usize) {
        if let Some(g) = self.games.get(row_idx) {
            self.attempted.borrow_mut().insert(g.db_id);
        }
        self.refresh(row_idx);
    }

    /// A pass will never run for this row (its box attached already
    /// concluded) — that counts as its final word too.
    pub fn mark_attempted(&self, db_id: i64) {
        self.attempted.borrow_mut().insert(db_id);
    }

    fn refresh(&self, row_idx: usize) {
        if self.show_finished.get() {
            return;
        }
        let rows = self.rows.borrow();
        let (Some(row), Some(game)) = (rows.get(row_idx), self.games.get(row_idx)) else {
            return;
        };
        if game_concluded(game, &self.attempted.borrow()) {
            hide_row_stable(&self.scrolled, std::slice::from_ref(&row));
        }
    }

    /// The toggle flipped: show everything, or re-hide the concluded.
    fn apply_all(&self) {
        if self.show_finished.get() {
            self.rows.borrow().iter().for_each(|row| row.set_visible(true));
            return;
        }
        let rows = self.rows.borrow();
        let attempted = self.attempted.borrow();
        let hiding: Vec<&gtk4::ListBoxRow> = rows
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                self.games
                    .get(*idx)
                    .is_some_and(|game| game_concluded(game, &attempted))
            })
            .map(|(_, row)| row)
            .collect();
        hide_row_stable(&self.scrolled, &hiding);
    }
}

/// Hide concluded rows without yanking the viewport: every collapsed
/// row above the fold would otherwise pull everything below it upward
/// by its height. Positions are captured before anything hides (the
/// layout goes stale as rows collapse), then the scroll offset moves
/// up by exactly the hidden-above total.
fn hide_row_stable(
    scrolled: &glib::WeakRef<gtk4::ScrolledWindow>,
    hiding: &[&gtk4::ListBoxRow],
) {
    if hiding.is_empty() {
        return;
    }
    let below_fold = |value: f32| {
        hiding
            .iter()
            .filter_map(|row| {
                let list = row.parent()?.downcast::<gtk4::ListBox>().ok()?;
                let point = row.compute_point(&list, &gtk4::graphene::Point::new(0.0, 0.0))?;
                let height = row.height() as f32;
                (point.y() + height <= value).then_some(height)
            })
            .sum::<f32>()
    };
    let shift = scrolled
        .upgrade()
        .map(|scrolled| below_fold(scrolled.vadjustment().value() as f32))
        .unwrap_or(0.0);
    for row in hiding {
        row.set_visible(false);
    }
    if shift > 0.0 {
        if let Some(scrolled) = scrolled.upgrade() {
            let vadj = scrolled.vadjustment();
            vadj.set_value((vadj.value() - f64::from(shift)).max(vadj.lower()));
        }
    }
}

/// Whether the game has nothing left to try automatically: every pass
/// that applies either matched, or gave its negative word this session.
fn game_concluded(g: &Game, attempted: &HashSet<i64>) -> bool {
    if needs_steam_match(g) && !attempted.contains(&g.db_id) {
        return false;
    }
    if needs_ra_match(g) && !attempted.contains(&g.db_id) {
        return false;
    }
    if needs_sgdb_match(g) && !attempted.contains(&g.db_id) {
        return false;
    }
    if needs_ss_match(g) && !attempted.contains(&g.db_id) {
        return false;
    }
    if needs_steam_title_match(g) && !attempted.contains(&g.db_id) {
        return false;
    }
    true
}

/// The games the console→Steam pass already failed: terminal, never
/// re-searched. A Steam miss blocks only the Steam pass — every other
/// pass still applies.
fn steam_missed_ids(state: &SharedState) -> HashSet<i64> {
    ira_db::match_missed_ids(&state.borrow().db, ira_db::miss_source::STEAM)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn collect_unmatched_games(state: &SharedState) -> (Vec<Game>, Vec<(String, String, String)>) {
    let s = state.borrow();
    let games = s.games.clone();
    let steam_missed = steam_missed_ids(state);
    let needs_matching: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            // A Steam miss blocks only the Steam pass: every other
            // pass still applies to the game.
            if steam_missed.contains(&g.db_id) {
                needs_steam_match(g)
                    || needs_ra_match(g)
                    || needs_sgdb_match(g)
                    || needs_ss_match(g)
            } else {
                needs_steam_match(g)
                    || needs_ra_match(g)
                    || needs_sgdb_match(g)
                    || needs_ss_match(g)
                    // A console game with every other match done still
                    // wants its Steam version found.
                    || needs_steam_title_match(g)
            }
        })
        .collect();
    let save_dir = &s.save_dir;
    let data_dir = std::path::Path::new(save_dir).join("data").join("steam");
    let mut map: Vec<(String, String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            if let Some(name) = ira_parser::read_app_name(save_dir, &app_id) {
                map.push((normalize_title(&name), app_id, name));
            }
        }
    }
    (needs_matching, map)
}

fn populate_match_list(
    list: &gtk4::ListBox,
    needs_matching: &[Game],
    state: &SharedState,
    dialog: &gtk4::Widget,
    ss_missed: &HashSet<i64>,
    vis: &RowVis,
) -> Vec<RowActions> {
    let ra_available = ra_pass_available(state);
    let steam_missed = steam_missed_ids(state);
    needs_matching
        .iter()
        .map(|game| {
            let (sources, mut actions) = create_match_row(list, game);
            for source in match_sources_for(game) {
                match source {
                    MatchSource::Steam | MatchSource::Sgdb => {
                        let actions_box = source_sub_row(&sources, source);
                        actions_box.append(&status_label(
                            &crate::tr!("Searching…"),
                            CSS_DIM_LABEL,
                        ));
                        if source == MatchSource::Steam {
                            actions.steam = Some(actions_box);
                        } else {
                            actions.sgdb = Some(actions_box);
                        }
                    }
                    MatchSource::Ra => {
                        let actions_box = source_sub_row(&sources, source);
                        attach_ra_actions(
                            &actions_box,
                            state,
                            game,
                            dialog,
                            ra_available,
                            vis,
                        );
                        actions.ra = Some(actions_box);
                    }
                    MatchSource::ScreenScraper => {
                        let actions_box = source_sub_row(&sources, source);
                        attach_ss_actions(
                            &actions_box,
                            state,
                            game,
                            dialog,
                            ss_missed.contains(&game.db_id),
                            vis,
                        );
                        actions.ss = Some(actions_box);
                    }
                    MatchSource::SteamLink => {
                        if steam_missed.contains(&game.db_id) {
                            // Steam already failed this one and no line
                            // runs for it: terminal from the start, like
                            // the SS misses.
                            vis.mark_attempted(game.db_id);
                        } else {
                            let actions_box = source_sub_row(&sources, source);
                            attach_steam_link_actions(&actions_box);
                            actions.steam_link = Some(actions_box);
                        }
                    }
                }
            }
            actions
        })
        .collect()
}

/// The exact normalized-title hit in a Steam store answer, if any —
/// the shared precision gate behind the PC batch pass and the
/// console-title pass, so punctuation variants ("X: Y" vs "X - Y")
/// never split a match.
fn exact_title_hit(results: &[(String, String)], norm: &str) -> Option<(String, String)> {
    results
        .iter()
        .find(|(_, name)| normalize_title(name) == norm)
        .map(|(id, name)| (id.clone(), name.clone()))
}

fn start_steam_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    title_map: Vec<(String, String, String)>,
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    vis: RowVis,
) {
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_steam_match(g))
        .map(|(i, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx: i,
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        0,
        None,
        {
            let steam = steam.clone();
            move |item| {
                let norm = normalize_title(&item.name);
                let matched = if norm.is_empty() {
                    None
                } else {
                    title_map
                        .iter()
                        .find(|(t, _, _)| t == &norm)
                        .map(|(_, id, name)| (id.clone(), name.clone()))
                };
                if matched.is_some() {
                    return matched;
                }
                let results = steam.search_steam_store(&item.name);
                exact_title_hit(&results, &norm)
            }
        },
        {
            let state = state.clone();
            let steam = steam;
            let rows = rows.to_vec();
            let parent_dialog = dialog.clone();
            let vis = vis;
            move |hit| {
                let Some(steam_box) = rows.get(hit.row_idx).and_then(|r| r.steam.clone())
                else {
                    return;
                };
                let auto_matched = hit.matched.is_some();
                handle_steam_search_result(
                    &state,
                    &steam_box,
                    &steam,
                    &hit.name,
                    hit.db_id,
                    hit.matched,
                    &parent_dialog,
                );
                if auto_matched {
                    append_steam_veto(
                        &state,
                        steam.clone(),
                        &steam_box,
                        &hit.name,
                        hit.db_id,
                        &parent_dialog,
                    );
                }
                // The pass spoke its final word for this row —
                // without this the hide toggle can never retire it.
                vis.pass_done(hit.row_idx);
            }
        },
    );
}

/// Unmatch for an automatically applied Steam match: clear the link and
/// block every automatic pass like any other veto, then offer the manual
/// search buttons again. Manual picks repaint label-only on purpose — the
/// user just chose them, so there is nothing to veto.
fn append_steam_veto(
    state: &SharedState,
    steam: Arc<ira_api::SteamDataClient>,
    steam_box: &gtk4::Box,
    game_name: &str,
    db_id: i64,
    parent_dialog: &gtk4::Widget,
) {
    let unmatch = gtk4::Button::with_label(&crate::tr!("Unmatch"));
    unmatch.add_css_class(CSS_DESTRUCTIVE_ACTION);
    unmatch.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let steam_box = steam_box.clone();
        let game_name = game_name.to_string();
        let parent_dialog = parent_dialog.clone();
        unmatch.connect_clicked(move |_| {
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::set_steam_id(&db, db_id, "") {
                eprintln!("Steam veto: failed to clear the link: {e}");
            }
            if let Err(e) = ira_db::set_manual_unmatch(&db, db_id, true) {
                eprintln!("Steam veto: failed to block re-matching: {e}");
            }
            if let Some(g) = state
                .borrow_mut()
                .games
                .iter_mut()
                .find(|g| g.db_id == db_id)
            {
                g.steam_id.clear();
                g.manual_unmatch = true;
            }
            handle_steam_search_result(
                &state,
                &steam_box,
                &steam,
                &game_name,
                db_id,
                None,
                &parent_dialog,
            );
        });
    }
    steam_box.append(&unmatch);
}

/// Unmatch for a confirmed Steam link: unlink and tombstone the source —
/// the same end state as Skip, so the pass stops re-asking. Unlike the
/// automatic passes no global block is needed: nothing re-applies a link
/// the user declined.
fn append_steam_link_veto(
    state: &SharedState,
    steam_box: &gtk4::Box,
    db_id: i64,
    row_idx: usize,
    vis: &RowVis,
) {
    let unmatch = gtk4::Button::with_label(&crate::tr!("Unmatch"));
    unmatch.add_css_class(CSS_DESTRUCTIVE_ACTION);
    unmatch.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let steam_box = steam_box.clone();
        let vis = vis.clone();
        unmatch.connect_clicked(move |_| {
            super::fetch_metadata::store_steam_link(&state, db_id, "");
            if let Err(e) =
                ira_db::tombstone_match_miss(&state.borrow().db, db_id, ira_db::miss_source::STEAM)
            {
                eprintln!("Steam link veto: miss record failed: {e}");
            }
            replace_row_actions(&steam_box, |ab| {
                ab.append(&status_label(
                    &crate::tr!("Skipped"),
                    CSS_DIM_LABEL,
                ));
            });
            vis.pass_done(row_idx);
        });
    }
    steam_box.append(&unmatch);
}

fn start_sgdb_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    dialog: &gtk4::Widget,
    vis: RowVis,
) {
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_sgdb_match(g))
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        0,
        None,
        {
            let steam = steam.clone();
            move |item| {
                let results = steam.search_sgdb(&item.name);
                super::helpers::matching_sgdb_result(&results, &item.name)
            }
        },
        {
            let state = state.clone();
            let rows = rows.to_vec();
            let parent_dialog = dialog.clone();
            let vis = vis;
            move |hit| {
                if let Some(sgdb_box) = rows.get(hit.row_idx).and_then(|r| r.sgdb.clone()) {
                    handle_unified_sgdb_result(
                        &state,
                        &sgdb_box,
                        hit.db_id,
                        &hit.name,
                        hit.matched,
                        &parent_dialog,
                    );
                    vis.pass_done(hit.row_idx);
                }
            }
        },
    );
}

/// The row's Steam-link line: a dim status while the exact-title pass
/// runs. The source label on the line already says Steam link, so the
/// status itself stays short.
fn attach_steam_link_actions(link_box: &gtk4::Box) {
    link_box.append(&status_label(
        &crate::tr!("Searching…"),
        CSS_DIM_LABEL,
    ));
}

/// The console→Steam pass: every row with a Steam box gets an exact
/// title search over the store, and every candidate waits on its
/// Link/Skip confirm — nothing applies itself, so remasters never sneak
/// onto originals. A store answer with no exact hit tombstones the game
/// for this source so later opens stop re-asking; a failed request
/// records nothing and retries next time. Progress shows on the
/// sidebar strip like every other pass.
fn start_steam_title_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    vis: RowVis,
) {
    let missed = steam_missed_ids(state);
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(i, g)| {
            rows.get(*i).is_some_and(|r| r.steam_link.is_some())
            && needs_steam_title_match(g)
            && !missed.contains(&g.db_id)
        })
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
        })
        .collect();
    if queue.is_empty() {
        return;
    }
    eprintln!("Steam title pass: {} game(s)", queue.len());

    let (steam, db) = {
        let s = state.borrow();
        (s.steam.clone(), s.db.clone())
    };
    // The strip shows the pass and carries its cancel button; when
    // another job owns the strip the pass simply runs without visible
    // progress.
    let (job, cancel) = match super::fetch_images::begin_strip_job(
        state,
        &crate::tr!("Matching games…"),
        &crate::tr!("Matching unmatched games…"),
    ) {
        Some(job) => {
            let cancel = job.cancel_flag();
            (Some(job), Some(cancel))
        }
        None => (None, None),
    };
    let total = queue.len();
    let done = Cell::new(0usize);
    let candidates = Cell::new(0usize);
    let rows = rows.to_vec();
    // The applier outlives this call: it needs its own state, like
    // every other pass.
    let state = std::rc::Rc::clone(state);
    // The Skip clicks run on the main loop: they need the database
    // handle the worker used.
    let db_main = db.clone();
    run_batch(
        queue,
        400, // one title search per game; the garnish runs after Link
        cancel,
        {
            let steam = steam.clone();
            let db = db.clone();
            move |item| {
                // Re-check the link live: the user may have linked the
                // game from its settings while the pass runs.
                let unlinked = ira_db::find_by_db_id(&db, item.db_id)
                    .ok()
                    .flatten()
                    .is_some_and(|entry| entry.steam_id.is_empty());
                if !unlinked {
                    return None;
                }
                let norm = normalize_title(&item.name);
                if norm.is_empty() {
                    return None;
                }
                match steam.search_steam_store_result(&item.name) {
                    Ok(results) => match exact_title_hit(&results, &norm) {
                        Some(hit) => Some(hit),
                        // The store answered and nothing matched
                        // exactly: not matchable, stop re-asking.
                        None => {
                            if let Err(e) = ira_db::tombstone_match_miss(
                                &db,
                                item.db_id,
                                ira_db::miss_source::STEAM,
                            ) {
                                eprintln!(
                                    "Steam title pass: '{}': miss record failed: {e}",
                                    item.name
                                );
                            }
                            None
                        }
                    },
                    // No answer at all — quota and network heal, so this
                    // retries next open instead of tombstoning.
                    Err(e) => {
                        eprintln!("Steam title pass: '{0}': search failed, will retry: {e}", item.name);
                        None
                    }
                }
            }
        },
        move |hit| {
            if hit.row_idx == BATCH_FINISHED {
                if let Some(job) = &job {
                    job.finish(
                        &state,
                        &crate::tr!("{} Steam candidates")
                            .replacen("{}", &candidates.get().to_string(), 1),
                        &crate::tr!("Matching finished"),
                    );
                }
                return;
            }
            let Some(steam_box) = rows.get(hit.row_idx).and_then(|r| r.steam_link.clone()) else {
                return;
            };
            let vis = vis.clone();
            let row_idx = hit.row_idx;
            let db_id = hit.db_id;
            let name = hit.name.clone();
            let done_tick = |vis: &RowVis| {
                vis.pass_done(row_idx);
                let finished = done.get() + 1;
                done.set(finished);
                if let Some(job) = &job {
                    job.progress(&state, finished, total, &name);
                }
            };
            let Some((id, candidate)) = hit.matched else {
                replace_row_actions(&steam_box, |ab| {
                    ab.append(&status_label(
                        &crate::tr!("No Steam match"),
                        CSS_DIM_LABEL,
                    ));
                });
                done_tick(&vis);
                return;
            };
            candidates.set(candidates.get() + 1);
            replace_row_actions(&steam_box, |ab| {
                ab.append(&status_label(&matched_text(&id, &candidate), CSS_DIM_LABEL));
                let link_btn = gtk4::Button::with_label(&crate::tr!("Link"));
                link_btn.add_css_class(CSS_SUGGESTED_ACTION);
                let skip_btn = gtk4::Button::with_label(&crate::tr!("Skip"));
                skip_btn.add_css_class(CSS_FLAT);
                ab.append(&link_btn);
                ab.append(&skip_btn);
                {
                    let state = state.clone();
                    let vis = vis.clone();
                    let steam_box = steam_box.clone();
                    link_btn.connect_clicked(move |_| {
                        if super::fetch_metadata::store_steam_link(&state, db_id, &id) {
                            replace_row_actions(&steam_box, |ab| {
                                ab.append(&status_label(
                                    &crate::tr!("Matched"),
                                    CSS_SUCCESS_LABEL,
                                ));
                                append_steam_link_veto(&state, &steam_box, db_id, row_idx, &vis);
                            });
                            vis.pass_done(row_idx);
                        } else {
                            eprintln!("Steam title pass: link for '{candidate}' failed");
                        }
                    });
                }
                {
                    let db = db_main.clone();
                    let vis = vis.clone();
                    let steam_box = steam_box.clone();
                    skip_btn.connect_clicked(move |_| {
                        if let Err(e) = ira_db::tombstone_match_miss(
                            &db,
                            db_id,
                            ira_db::miss_source::STEAM,
                        ) {
                            eprintln!("Steam title pass: miss record failed: {e}");
                        }
                        replace_row_actions(&steam_box, |ab| {
                            ab.append(&status_label(
                                &crate::tr!("Skipped"),
                                CSS_DIM_LABEL,
                            ));
                        });
                        vis.pass_done(row_idx);
                    });
                }
            });
            done_tick(&vis);
        },
    );
}

pub fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    let (needs_matching, title_map) = collect_unmatched_games(state);
    eprintln!("Mass matcher: opening with {} candidate game(s)", needs_matching.len());
    if needs_matching.is_empty() {
        let d = adw::AlertDialog::new(
            Some(&crate::tr!("Nothing to match")),
            Some(&crate::tr!(
                "Every game already has a trophy source and image assets linked."
            )),
        );
        d.add_response("ok", &crate::tr!("OK"));
        d.present(Some(&window));
        return;
    }

    // A plain window, not an adw::Dialog — the list wants the space and
    // the user the resize handle.
    let dialog = adw::Window::new();
    dialog.set_title(Some(&crate::tr!("Match unmatched games")));
    dialog.set_modal(true);
    dialog.set_transient_for(Some(&window));
    dialog.set_default_size(640, 560);

    let toolbar = adw::ToolbarView::new();

    let header = adw::HeaderBar::new();
    let count = needs_matching.len();
    let subtitle = if count == 1 {
        crate::tr!("1 game to match")
    } else {
        crate::tr!("{} games to match").replacen("{}", &count.to_string(), 1)
    };
    let title = adw::WindowTitle::new(&crate::tr!("Match unmatched games"), &subtitle);
    header.set_title_widget(Some(&title));
    // Matched and PC games with holes in their metadata: every source
    // at once — Steam fills what it can, ScreenScraper gets exact
    // refetches by id. The job runs on the sidebar strip, so this
    // button hands it off and the window is free to close.
    let (has_ss_creds, ss_busy) = {
        let s = state.borrow();
        (!s.cfg.screenscraper_id.is_empty(), s.ss_job_busy.get())
    };
    let refetch_btn = gtk4::Button::with_label(&crate::tr!("Fetch missing metadata"));
    refetch_btn.set_sensitive(has_ss_creds && !ss_busy);
    {
        let state = state.clone();
        refetch_btn.connect_clicked(move |_| {
            super::fetch_metadata::start_full_refetch(&state, false);
        });
    }
    header.pack_end(&refetch_btn);
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let (scrolled, list) = super::helpers::clamped_boxed_list(600);
    scrolled.set_vexpand(true);
    let ss_missed: HashSet<i64> = match ira_db::match_missed_ids(&state.borrow().db, ira_db::miss_source::SS) {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            eprintln!("Mass matcher: could not read ScreenScraper misses: {e}");
            HashSet::new()
        }
    };
    // show_finished mirrors the switch below, inverted: with the switch
    // off, nothing hides and every row plays its pass out in the open.
    let show_finished = Rc::new(Cell::new(true));
    let vis = RowVis::new(needs_matching.to_vec(), show_finished.clone(), &scrolled);
    let vis_toggle = vis.clone();
    let rows = populate_match_list(&list, &needs_matching, state, dialog.upcast_ref(), &ss_missed, &vis);
    vis.set_rows(rows.iter().map(|r| r.row.clone()).collect());
    vis.apply_all();
    let show_finished_c = show_finished.clone();
    let hide_switch = gtk4::Switch::new();
    // Hiding starts off: the whole point of the dialog is watching what
    // each source does with each row — rows flipping to "Matched" or to
    // the manual-search button must stay visible, not vanish mid-pass.
    // The switch still retires them on demand.
    hide_switch.set_active(false);
    hide_switch.set_valign(gtk4::Align::Center);
    hide_switch.connect_active_notify(move |toggle| {
        show_finished_c.set(!toggle.is_active());
        vis_toggle.apply_all();
    });
    let hide_label = gtk4::Label::new(Some(&crate::tr!("Hide done and failed")));
    hide_label.set_valign(gtk4::Align::Center);
    let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bottom.set_halign(gtk4::Align::End);
    bottom.set_margin_start(12);
    bottom.set_margin_end(12);
    bottom.set_margin_top(6);
    bottom.set_margin_bottom(6);
    bottom.append(&hide_label);
    bottom.append(&hide_switch);
    toolbar.add_bottom_bar(&bottom);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    dialog.set_content(Some(&toolbar));
    dialog.present();

    start_steam_batch_matching(
        state,
        &needs_matching,
        title_map,
        &rows,
        dialog.upcast_ref(),
        vis.clone(),
    );
    start_sgdb_batch_matching(state, &needs_matching, &rows, dialog.upcast_ref(), vis.clone());
    start_ra_batch_matching(state, &needs_matching, &rows, dialog.upcast_ref(), vis.clone());
    start_ss_batch_matching(state, &needs_matching, &rows, dialog.upcast_ref(), vis.clone());
    start_steam_title_matching(state, &needs_matching, &rows, vis);
}

/// One list row: the game header plus one labeled sub-row per applicable
/// source, each with its own action box. Sources stack vertically instead
/// of crowding into the header's suffix, so every pass repaints its own
/// line and matched lines gain room for an Unmatch button.
fn create_match_row(
    list: &gtk4::ListBox,
    game: &Game,
) -> (gtk4::Box, RowActions) {
    let row = gtk4::ListBoxRow::new();
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);

    let header = adw::ActionRow::new();
    // Game titles are shown as typed — "Fear & Hunger" is not markup.
    header.set_use_markup(false);
    header.set_title(&game.name);
    header.set_subtitle(&ira_models::platform_display_name(&game.platform_id));
    // Long names ellipsize like every list row; the full name rides the
    // tooltip.
    header.set_tooltip_text(Some(&game.name));
    header.set_activatable(false);
    outer.append(&header);

    let sources = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    outer.append(&sources);

    row.set_child(Some(&outer));
    list.append(&row);
    (
        sources,
        RowActions {
            row,
            ..Default::default()
        },
    )
}

/// One source's line under the game header: the source name plus its
/// action box, right-aligned so every line scans the same way.
fn source_sub_row(sources: &gtk4::Box, source: MatchSource) -> gtk4::Box {
    let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    line.set_margin_start(16);
    line.set_margin_end(12);
    let label = gtk4::Label::new(Some(&source_label(source)));
    label.add_css_class(CSS_DIM_LABEL);
    label.set_valign(gtk4::Align::Center);
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    actions.set_hexpand(true);
    actions.set_halign(gtk4::Align::End);
    actions.set_valign(gtk4::Align::Center);
    line.append(&label);
    line.append(&actions);
    sources.append(&line);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(kind: ira_models::GameKind) -> Game {
        Game {
            kind,
            ..Game::default()
        }
    }

    #[test]
    fn test_needs_steam_match_requires_no_ids_at_all() {
        let mut g = game(ira_models::GameKind::Wine);
        assert!(needs_steam_match(&g));
        g.sgdb_id = "123".to_string();
        assert!(
            !needs_steam_match(&g),
            "SGDB-only game must not steam-match"
        );
        g.sgdb_id.clear();
        g.manual_unmatch = true;
        assert!(!needs_steam_match(&g));
    }

    #[test]
    fn test_needs_steam_match_skips_console_and_retro_kinds() {
        for kind in [
            ira_models::GameKind::ThreeDS,
            ira_models::GameKind::WiiU,
            ira_models::GameKind::Switch,
            ira_models::GameKind::Retro,
        ] {
            assert!(!needs_steam_match(&game(kind)), "{kind} has no steam path");
        }
    }

    #[test]
    fn test_needs_sgdb_match_covers_console_kinds_with_ids() {
        let mut g = game(ira_models::GameKind::ThreeDS);
        g.name = "Kid Icarus: Uprising".to_string();
        g.steam_id = "00040000000e5c00".to_string();
        assert!(needs_sgdb_match(&g), "3ds games match via sgdb by default");
        g.sgdb_id = "42".to_string();
        assert!(!needs_sgdb_match(&g));
        g.sgdb_id.clear();
        g.manual_unmatch = true;
        assert!(!needs_sgdb_match(&g));
    }

    #[test]
    fn test_pc_kinds_auto_match_with_steam_and_never_confirm() {
        // PC games bypass the Link/Skip confirm entirely: the PC batch
        // pass still auto-matches them, and the title pass never claims
        // them.
        for kind in [
            ira_models::GameKind::Wine,
            ira_models::GameKind::Linux,
            ira_models::GameKind::Steam,
        ] {
            let mut g = game(kind);
            g.name = "Hollow Knight".to_string();
            assert!(needs_steam_match(&g), "{kind} auto-matches");
            assert!(!needs_steam_title_match(&g), "{kind} never confirms");
        }
    }

    #[test]
    fn test_game_concluded_needs_every_attempt() {
        // A row with an open pass stays; attempted passes retire it.
        let mut g = game(ira_models::GameKind::Wine);
        g.name = "Hollow Knight".to_string();
        let empty: HashSet<i64> = HashSet::new();
        assert!(!game_concluded(&g, &empty));
        let attempted: HashSet<i64> = HashSet::from([g.db_id]);
        assert!(game_concluded(&g, &attempted));
    }

    #[test]
    fn test_exact_title_hit_ignores_punctuation_variants() {
        // "X: Y" and "X - Y" normalize identically: the store's
        // spelling must not decide the match.
        let results = vec![
            ("787480".to_string(), "Phoenix Wright: Ace Attorney Trilogy".to_string()),
            ("1032760".to_string(), "Phoenix Wright: Ace Attorney Trilogy - Turnabout Tunes".to_string()),
        ];
        let norm = normalize_title("Phoenix Wright - Ace Attorney Trilogy");
        assert_eq!(
            exact_title_hit(&results, &norm),
            Some((
                "787480".to_string(),
                "Phoenix Wright: Ace Attorney Trilogy".to_string()
            ))
        );
        assert!(exact_title_hit(&results, "no such game").is_none());
        assert!(exact_title_hit(&[], &norm).is_none());
    }

    #[test]
    fn test_name_searches_skip_unsearchable_names() {        for name in ["", "   ", "App ID: 1234567"] {
            assert!(!has_searchable_name(name), "{name:?} is not searchable");
            let mut sgdb = game(ira_models::GameKind::Switch);
            sgdb.name = name.to_string();
            assert!(
                !needs_sgdb_match(&sgdb),
                "sgdb must not search {name:?}"
            );
            let mut ss = game(ira_models::GameKind::Switch);
            ss.name = name.to_string();
            ss.platform_id = "switch".to_string();
            assert!(!needs_ss_match(&ss), "ss must not search {name:?}");
        }
        assert!(has_searchable_name("Twilight Syndrome: Saikai"));
    }

    #[test]
    fn test_switch_is_never_ra_matchable() {
        let mut g = game(ira_models::GameKind::Switch);
        g.platform_id = "switch".to_string();
        assert!(!needs_ra_match(&g), "the Switch has no RA support at all");
    }

    #[test]
    fn test_needs_sgdb_match_skips_steam_enriched_games() {
        let mut g = game(ira_models::GameKind::Wine);
        g.name = "Half-Life 2".to_string();
        g.steam_id = "420530".to_string();
        assert!(!needs_sgdb_match(&g), "steam-driven enrichment owns these");
    }

    #[test]
    fn test_platform_linkage_is_not_a_match() {
        // A Goldberg game carries the Steam appid as its platform linkage
        // (`app_id`) with no match behind it: both passes must still claim
        // it, or it sits unmatched forever with only the SS button showing.
        let mut g = game(ira_models::GameKind::Wine);
        g.name = "Hollow Knight".to_string();
        g.app_id = "1966900".to_string();
        assert!(needs_steam_match(&g));
        assert!(needs_sgdb_match(&g));
        // A real Steam match closes both passes again.
        g.steam_id = "1966900".to_string();
        assert!(!needs_steam_match(&g));
        assert!(!needs_sgdb_match(&g));
    }

    #[test]
    fn test_needs_ss_match_targets_mapped_console_platforms() {
        // 3ds is a console-emulator kind on a mapped platform: the prime
        // ScreenScraper candidate.
        let mut g = game(ira_models::GameKind::ThreeDS);
        g.name = "Kid Icarus: Uprising".to_string();
        g.platform_id = "3ds".to_string();
        assert!(needs_ss_match(&g));
        // The live shape: a 3DS game carries its title id as the platform
        // id and usually an SGDB id already — neither may keep the SS pass
        // away, or the game silently never gets searched.
        let mut live = game(ira_models::GameKind::ThreeDS);
        live.name = "The Legend of Zelda: Ocarina of Time 3D".to_string();
        live.platform_id = "0004000000038800".to_string();
        live.sgdb_id = "37340".to_string();
        assert!(needs_ss_match(&live));
        // Once metadata is on record, the pass leaves it alone.
        g.screenscraper_id = "2124".to_string();
        assert!(!needs_ss_match(&g));
        g.screenscraper_id.clear();
        g.manual_unmatch = true;
        assert!(!needs_ss_match(&g));
        // Console kinds on platforms ScreenScraper has no system for.
        let mut unmapped = game(ira_models::GameKind::Switch);
        unmapped.platform_id = "madeup".to_string();
        assert!(!needs_ss_match(&unmapped));
    }

    #[test]
    fn test_needs_ss_match_covers_pc_games() {
        // PC games search ScreenScraper's Windows/Linux systems and then
        // diff the whole source against their Steam data — every PC kind
        // qualifies, whatever their store-app-id platforms map to.
        for kind in [
            ira_models::GameKind::Wine,
            ira_models::GameKind::Linux,
            ira_models::GameKind::Steam,
        ] {
            let mut g = game(kind);
            g.name = "Hollow Knight".to_string();
            assert!(needs_ss_match(&g), "{kind} qualifies");
        }
        // Manual unmatch still wins over the pass.
        let mut g = game(ira_models::GameKind::Wine);
        g.manual_unmatch = true;
        assert!(!needs_ss_match(&g));
    }

    #[test]
    fn test_needs_ss_match_resolves_product_code_platforms() {
        // PS3/PS4 games carry a native product code as the platform id;
        // the kind is what names the console.
        let mut g = game(ira_models::GameKind::Ps4);
        g.name = "Bloodborne".to_string();
        g.platform_id = "CUSA12112".to_string();
        assert!(needs_ss_match(&g));
        let mut g = game(ira_models::GameKind::Ps3);
        g.name = "Demon's Souls".to_string();
        g.platform_id = "NPUB30698".to_string();
        assert!(needs_ss_match(&g));
    }

    /// The row planner claims exactly the sources whose passes apply, in
    /// display order: a PC game gets Steam, SGDB and ScreenScraper lines.
    #[test]
    fn test_match_sources_for_pc_game() {
        let mut g = game(ira_models::GameKind::Wine);
        g.name = "Hollow Knight".to_string();
        assert_eq!(
            match_sources_for(&g),
            vec![
                MatchSource::Steam,
                MatchSource::Sgdb,
                MatchSource::ScreenScraper
            ]
        );
    }

    /// A retro game with no matches gets every retro-applicable line,
    /// including the Steam-link one for its console identity.
    #[test]
    fn test_match_sources_for_unmatched_retro_game() {
        let mut g = game(ira_models::GameKind::Retro);
        g.name = "Chrono Trigger".to_string();
        g.platform_id = "snes".to_string();
        let sources = match_sources_for(&g);
        assert!(sources.contains(&MatchSource::Sgdb));
        assert!(sources.contains(&MatchSource::ScreenScraper));
        assert!(sources.contains(&MatchSource::SteamLink));
        assert!(!sources.contains(&MatchSource::Steam));
        // Display order is stable: Steam first, Steam link last.
        let pos = |s: MatchSource| sources.iter().position(|o| *o == s).unwrap();
        assert!(pos(MatchSource::Sgdb) < pos(MatchSource::SteamLink));
    }

    /// A fully matched game claims no lines at all.
    #[test]
    fn test_match_sources_for_matched_game_is_empty() {
        let mut g = game(ira_models::GameKind::Wine);
        g.name = "Hollow Knight".to_string();
        g.steam_id = "367520".to_string();
        g.sgdb_id = "123".to_string();
        g.screenscraper_id = "456".to_string();
        assert!(match_sources_for(&g).is_empty());
    }

    /// Every source has a distinct label for its line.
    #[test]
    fn test_source_labels_are_distinct() {
        let labels = [
            MatchSource::Steam,
            MatchSource::Sgdb,
            MatchSource::Ra,
            MatchSource::ScreenScraper,
            MatchSource::SteamLink,
        ]
        .map(source_label);
        let mut sorted = labels.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len());
    }
}
