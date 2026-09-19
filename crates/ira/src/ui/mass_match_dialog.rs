use crate::Game;
use adw::prelude::*;
use ira_models::GameKind;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use super::css::*;
use super::helpers::replace_row_actions;
use super::mass_match_batch::{run_batch, BatchItem, RowActions, BATCH_FINISHED};
use super::mass_match_ss::RefetchOutcome;
use super::mass_match_ra::{attach_ra_actions, ra_pass_available, start_ra_batch_matching};
use super::mass_match_ss::{attach_ss_actions, start_ss_batch_matching};
use super::sgdb_match_dialog::handle_unified_sgdb_result;
use super::state::SharedState;
use super::steam_search_dialog::{handle_steam_search_result, status_label};

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

/// Games with no store or SGDB id at all: candidates for a Steam store
/// match. Console-emulator games and Retro ROMs are excluded — their names
/// come from title ids/ROM files, so Steam search is noise; they are
/// enriched through SGDB (and RA) instead.
fn needs_steam_match(g: &Game) -> bool {
    g.app_id.is_empty()
        && g.sgdb_id.is_empty()
        && !g.manual_unmatch
        && !g.kind.is_console_emulator()
        && g.kind != ira_models::GameKind::Retro
}

/// Games an SGDB match can enrich: everything without an SGDB id that has
/// no Steam-driven enrichment path (console-emulator games, Retro ROMs, and
/// games with no ids at all).
fn needs_sgdb_match(g: &Game) -> bool {
    g.sgdb_id.is_empty()
        && !g.manual_unmatch
        && (g.app_id.is_empty()
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
/// stored pieces only ever fill blanks.
fn needs_ss_match(g: &Game) -> bool {
    if g.manual_unmatch || !g.screenscraper_id.is_empty() {
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

/// Console games whose metadata misses something Steam can give: the
/// exact-title Steam garnish serves them without ever touching their
/// console identity.
fn needs_steam_title_match(state: &SharedState, g: &Game) -> bool {
    if g.manual_unmatch
        || g.name.trim().is_empty()
        || !g.steam_link_id.is_empty()
        || !(g.kind.is_console_emulator() || g.kind == ira_models::GameKind::Retro)
    {
        return false;
    }
    let s = state.borrow();
    match ira_db::scraper_metadata_for_game(&s.db, g.db_id) {
        Ok(None) => true,
        Ok(Some(meta)) => {
            super::mass_match_ss::release_date_is_broken(&meta.release_date)
                || (meta.developers.is_empty() && meta.publishers.is_empty())
                || meta.synopses.is_empty()
                || meta.classifications.is_empty()
        }
        Err(_) => false,
    }
}

/// Per-dialog visibility for the match list: rows whose game has
/// gotten a final word from every pass that applies to it (matched, or
/// a negative result this session) are hidden while the toggle is on,
/// so the list only shows games still in play.
#[derive(Clone)]
pub(super) struct RowVis {
    state: SharedState,
    rows: RefCell<Vec<gtk4::ListBoxRow>>,
    games: Vec<Game>,
    show_finished: Rc<Cell<bool>>,
    attempted: Rc<RefCell<HashSet<i64>>>,
}

impl RowVis {
    pub(super) fn new(
        state: &SharedState,
        games: Vec<Game>,
        show_finished: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            state: std::rc::Rc::clone(state),
            rows: RefCell::new(Vec::new()),
            games,
            show_finished,
            attempted: Rc::new(RefCell::new(HashSet::new())),
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
        if game_concluded(&self.state, game, &self.attempted.borrow()) {
            row.set_visible(false);
        }
    }

    /// The toggle flipped: show everything, or re-hide the concluded.
    fn apply_all(&self) {
        if self.show_finished.get() {
            self.rows.borrow().iter().for_each(|row| row.set_visible(true));
            return;
        }
        let rows = self.rows.borrow();
        for (idx, row) in rows.iter().enumerate() {
            if let Some(game) = self.games.get(idx) {
                row.set_visible(!game_concluded(&self.state, game, &self.attempted.borrow()));
            }
        }
    }
}

/// Whether the game has nothing left to try automatically: every pass
/// that applies either matched, or gave its negative word this session.
fn game_concluded(state: &SharedState, g: &Game, attempted: &HashSet<i64>) -> bool {
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
    if needs_steam_title_match(state, g) && !attempted.contains(&g.db_id) {
        return false;
    }
    true
}

fn collect_unmatched_games(state: &SharedState) -> (Vec<Game>, Vec<(String, String, String)>) {
    let s = state.borrow();
    let games = s.games.clone();
    let needs_matching: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            needs_steam_match(g) || needs_ra_match(g) || needs_sgdb_match(g) || needs_ss_match(g)
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
    needs_matching
        .iter()
        .map(|game| {
            let searching_text = if needs_steam_match(game) {
                crate::tr!("Searching Steam...")
            } else if needs_sgdb_match(game) {
                crate::tr!("Searching SGDB...")
            } else {
                String::new()
            };
            let (row, main) = create_match_row(list, &game.name, &searching_text);
            let ra = needs_ra_match(game).then(|| {
                attach_ra_actions(&row, state, game, dialog, ra_available, vis)
            });
            let ss = needs_ss_match(game).then(|| {
                attach_ss_actions(
                    &row,
                    state,
                    game,
                    dialog,
                    ss_missed.contains(&game.db_id),
                    vis,
                )
            });
            let steam = needs_steam_title_match(state, game)
                .then(|| attach_steam_title_actions(&row));
            RowActions { row: row.upcast(), main, ra, ss, steam }
        })
        .collect()
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
                results
                    .iter()
                    .find(|(_, name)| normalize_title(name) == norm)
                    .map(|(id, name)| (id.clone(), name.clone()))
            }
        },
        {
            let state = state.clone();
            let steam = steam;
            let rows = rows.to_vec();
            let parent_dialog = dialog.clone();
            let vis = vis;
            move |hit| {
                if let Some(row) = rows.get(hit.row_idx) {
                    handle_steam_search_result(
                        &state,
                        &row.main,
                        &steam,
                        &hit.name,
                        hit.db_id,
                        hit.matched,
                        &parent_dialog,
                    );
                    // The pass spoke its final word for this row —
                    // without this the hide toggle can never retire it.
                    vis.pass_done(hit.row_idx);
                }
            }
        },
    );
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
                if let Some(row) = rows.get(hit.row_idx) {
                    handle_unified_sgdb_result(
                        &state,
                        &row.main,
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

/// The row's Steam box: a dim status while the exact-title pass runs.
fn attach_steam_title_actions(row: &adw::ActionRow) -> gtk4::Box {
    let steam_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    steam_box.set_valign(gtk4::Align::Center);
    steam_box.append(&status_label(
        &crate::tr!("Searching Steam..."),
        CSS_DIM_LABEL,
    ));
    row.add_suffix(&steam_box);
    steam_box
}

/// The console→Steam pass: every row with a Steam box gets an exact
/// title search over the store, and whatever matches is garnished
/// metadata-only — the game's console identity is never touched.
fn start_steam_title_matching(
    state: &SharedState,
    needs_matching: &[Game],
    rows: &[RowActions],
    vis: RowVis,
) {
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(i, g)| {
            rows.get(*i).is_some_and(|r| r.steam.is_some())
                && needs_steam_title_match(state, g)
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

    let steam = state.borrow().steam.clone();
    let db = state.borrow().db.clone();
    let rows = rows.to_vec();
    run_batch(
        queue,
        400, // a title search plus up to two metadata reads per game
        None,
        {
            let steam = steam.clone();
            let db = db.clone();
            move |item| {
                match super::fetch_metadata::steam_refetch_one(&steam, &db, item.db_id) {
                    RefetchOutcome::Filled => Some(true),
                    _ => Some(false),
                }
            }
        },
        move |hit| {
            if hit.row_idx == BATCH_FINISHED {
                return;
            }
            let Some(steam_box) = rows.get(hit.row_idx).and_then(|r| r.steam.clone()) else {
                return;
            };
            let vis = vis.clone();
            replace_row_actions(&steam_box, |ab| match hit.matched {
                Some(true) => ab.append(&status_label(
                    &crate::tr!("Steam: matched"),
                    CSS_SUCCESS_LABEL,
                )),
                _ => ab.append(&status_label(
                    &crate::tr!("No Steam match"),
                    CSS_DIM_LABEL,
                )),
            });
            vis.pass_done(hit.row_idx);
        },
    );
}

pub fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    let (needs_matching, title_map) = collect_unmatched_games(state);

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
    let ss_missed: HashSet<i64> = match ira_db::scraper_missed_ids(&state.borrow().db) {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            eprintln!("Mass matcher: could not read ScreenScraper misses: {e}");
            HashSet::new()
        }
    };
    // Hiding starts on, matching the active switch below: done and
    // failed rows stay out of the list until it is switched off.
    let show_finished = Rc::new(Cell::new(false));
    let vis = RowVis::new(state, needs_matching.to_vec(), show_finished.clone());
    let vis_toggle = vis.clone();
    let rows = populate_match_list(&list, &needs_matching, state, dialog.upcast_ref(), &ss_missed, &vis);
    vis.set_rows(rows.iter().map(|r| r.row.clone()).collect());
    vis.apply_all();
    let show_finished_c = show_finished.clone();
    let hide_switch = gtk4::Switch::new();
    hide_switch.set_active(true);
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

/// One list row: the game's title plus its main action box, which starts
/// as a dim status label when `searching_text` is set.
fn create_match_row(
    list: &gtk4::ListBox,
    name: &str,
    searching_text: &str,
) -> (adw::ActionRow, gtk4::Box) {
    let row = adw::ActionRow::new();
    // Game titles are shown as typed — "Fear & Hunger" is not markup.
    row.set_use_markup(false);
    row.set_title(name);
    // Long local names wrap to two lines at most, then ellipsize, so the
    // suffix status label and buttons keep a usable share of the row width.
    row.set_title_lines(2);

    let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    action_box.set_valign(gtk4::Align::Center);
    if !searching_text.is_empty() {
        action_box.append(&status_label(searching_text, CSS_DIM_LABEL));
    }
    row.add_suffix(&action_box);

    list.append(&row);
    (row, action_box)
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
        g.app_id = "00040000000e5c00".to_string();
        assert!(needs_sgdb_match(&g), "3ds games match via sgdb by default");
        g.sgdb_id = "42".to_string();
        assert!(!needs_sgdb_match(&g));
        g.sgdb_id.clear();
        g.manual_unmatch = true;
        assert!(!needs_sgdb_match(&g));
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
        g.app_id = "420530".to_string();
        assert!(!needs_sgdb_match(&g), "steam-driven enrichment owns these");
    }

    #[test]
    fn test_needs_ss_match_targets_mapped_console_platforms() {
        // 3ds is a console-emulator kind on a mapped platform: the prime
        // ScreenScraper candidate.
        let mut g = game(ira_models::GameKind::ThreeDS);
        g.platform_id = "3ds".to_string();
        assert!(needs_ss_match(&g));
        // The live shape: a 3DS game carries its title id as the platform
        // id and usually an SGDB id already — neither may keep the SS pass
        // away, or the game silently never gets searched.
        let mut live = game(ira_models::GameKind::ThreeDS);
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
        assert!(needs_ss_match(&game(ira_models::GameKind::Wine)));
        assert!(needs_ss_match(&game(ira_models::GameKind::Linux)));
        assert!(needs_ss_match(&game(ira_models::GameKind::Steam)));
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
        g.platform_id = "CUSA12112".to_string();
        assert!(needs_ss_match(&g));
        let mut g = game(ira_models::GameKind::Ps3);
        g.platform_id = "NPUB30698".to_string();
        assert!(needs_ss_match(&g));
    }
}
