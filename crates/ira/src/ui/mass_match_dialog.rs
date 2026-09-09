use crate::Game;
use adw::prelude::*;
use std::collections::HashSet;
use std::cell::Cell;
use std::rc::Rc;

use super::css::*;
use super::helpers::clear_children;
use super::ra_match_dialog::show_ra_search_dialog;
use super::screenscraper_match_dialog::show_screenscraper_search_dialog;
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

/// Games a ScreenScraper match can enrich: everything without one yet —
/// PC entries included, the metadata source is not console-only.
fn needs_scraper_match(g: &Game, scraped_ids: &HashSet<i64>) -> bool {
    !scraped_ids.contains(&g.db_id) && !g.manual_unmatch
}

/// What the matcher works from: the games needing something, the Steam
/// app-id name map, and the ids of already-scraped entries.
type UnmatchedGames = (Vec<Game>, Vec<(String, String, String)>, HashSet<i64>);

fn collect_unmatched_games(state: &SharedState) -> UnmatchedGames {
    let s = state.borrow();
    let games = s.games.clone();
    let scraped_ids: HashSet<i64> = ira_db::load_all_games(&s.db)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !entry.screenscraper_id.is_empty())
        .map(|entry| entry.id)
        .collect();
    let needs_matching: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            needs_steam_match(g)
                || needs_ra_match(g)
                || needs_sgdb_match(g)
                || needs_scraper_match(g, &scraped_ids)
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
    (needs_matching, map, scraped_ids)
}

fn populate_match_list(
    list: &gtk4::ListBox,
    needs_matching: &[Game],
    state: &SharedState,
    dialog: &adw::Dialog,
    scraped_ids: &HashSet<i64>,
) -> (Vec<gtk4::Box>, Vec<gtk4::Box>) {
    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();
    let mut scraper_row_boxes: Vec<gtk4::Box> = Vec::new();

    for game in needs_matching.iter() {
        let action_box = if needs_ra_match(game) {
            let ac = create_match_row(list, &game.name, &crate::tr!("RA: not matched"));
            let inner = ac.clone();
            let sc = state.clone();
            let gn = game.name.clone();
            let pid = game.platform_id.clone();
            let did = game.db_id;
            let dlg = dialog.clone();
            let ra_btn = gtk4::Button::with_label(&crate::tr!("Search RA…"));
            ra_btn.add_css_class(CSS_SUGGESTED_ACTION);
            let sc2 = sc.clone();
            let gn2 = gn.clone();
            let pid2 = pid.clone();
            let dlg2 = dlg.clone();
            let did2 = did;
            let inner_c = inner.clone();
            ra_btn.connect_clicked(move |_| {
                let inner_update = inner_c.clone();
                show_ra_search_dialog(
                    &sc2,
                    did2,
                    &gn2,
                    &pid2,
                    &dlg2,
                    Some(Rc::new(move || {
                        clear_children(&inner_update);
                        let label = status_label(&crate::tr!("RA: matched"), CSS_SUCCESS_LABEL);
                        inner_update.append(&label);
                    })),
                );
            });
            inner.append(&ra_btn);
            ac
        } else if needs_steam_match(game) || needs_sgdb_match(game) {
            let searching_text = if needs_steam_match(game) {
                crate::tr!("Searching Steam...")
            } else {
                crate::tr!("Searching SGDB...")
            };
            create_match_row(list, &game.name, &searching_text)
        } else {
            // ScreenScraper is this entry's only pass: its own row below
            // covers it, no SGDB-labelled placeholder here.
            continue;
        };
        row_action_boxes.push(action_box);
    }

    // ScreenScraper rows: one per entry without a match yet, PC entries
    // included. They are separate from the Steam/SGDB rows because both
    // passes work the same list row otherwise — a match landing on one
    // would wipe the other's status.
    for game in needs_matching.iter() {
        if !needs_scraper_match(game, scraped_ids) {
            continue;
        }
        let box_ = create_match_row(
            list,
            &game.name,
            &crate::tr!("Searching ScreenScraper..."),
        );
        let sc = state.clone();
        let gn = game.name.clone();
        let pid = game.platform_id.clone();
        let did = game.db_id;
        let dlg = dialog.clone();
        let ss_btn = gtk4::Button::with_label(&crate::tr!("Search SS…"));
        ss_btn.add_css_class(CSS_SUGGESTED_ACTION);
        let inner_c = box_.clone();
        ss_btn.connect_clicked(move |_| {
            let inner_update = inner_c.clone();
            show_screenscraper_search_dialog(
                &sc,
                did,
                &gn,
                &pid,
                &dlg,
                Some(Rc::new(move || {
                    clear_children(&inner_update);
                    let label = status_label(&crate::tr!("SS: matched"), CSS_SUCCESS_LABEL);
                    inner_update.append(&label);
                })),
            );
        });
        box_.append(&ss_btn);
        scraper_row_boxes.push(box_);
    }

    (row_action_boxes, scraper_row_boxes)
}

/// One queued batch candidate: the game to match plus which list row its
/// result belongs to.
struct BatchItem {
    name: String,
    db_id: i64,
    row_idx: usize,
    platform_id: String,
}

/// A finished candidate handed from the worker thread back to the UI loop.
struct BatchHit<T> {
    row_idx: usize,
    db_id: i64,
    name: String,
    matched: Option<T>,
}

/// Shared shape of both batch passes: one sequential worker thread computes
/// matches over `queue`, and results are applied on the UI loop every
/// `interval_ms` until the queue drains. `worker` runs off-thread and must
/// not touch GTK; it waits `pace_ms` before every request but the first,
/// so a rate-limited service sees one request per pace, never a burst.
/// `on_result` runs on the main loop.
fn run_batch<T: Send + 'static>(
    queue: Vec<BatchItem>,
    interval_ms: u64,
    pace_ms: u64,
    worker: impl Fn(&BatchItem) -> Option<T> + Send + 'static,
    on_result: impl Fn(BatchHit<T>) + 'static,
) {
    let total = queue.len();
    let (tx, rx) = std::sync::mpsc::channel::<BatchHit<T>>();
    std::thread::spawn(move || {
        for (index, item) in queue.iter().enumerate() {
            if index > 0 && pace_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(pace_ms));
            }
            let matched = worker(item);
            let _ = tx.send(BatchHit {
                row_idx: item.row_idx,
                db_id: item.db_id,
                name: item.name.clone(),
                matched,
            });
        }
    });

    let rx = std::cell::RefCell::new(rx);
    let remaining = Cell::new(total);
    glib::timeout_add_local(std::time::Duration::from_millis(interval_ms), move || {
        if let Ok(hit) = rx.borrow_mut().try_recv() {
            on_result(hit);
            let left = remaining.get();
            if left <= 1 {
                return glib::ControlFlow::Break;
            }
            remaining.set(left - 1);
        }
        glib::ControlFlow::Continue
    });
}

fn start_steam_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    title_map: Vec<(String, String, String)>,
    row_action_boxes: &[gtk4::Box],
    dialog: &adw::Dialog,
) {
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_steam_match(g))
        .map(|(i, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx: i,
            platform_id: g.platform_id.clone(),
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        50,
        0,
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
            let row_boxes = row_action_boxes.to_vec();
            let parent_dialog = dialog.clone();
            move |hit| {
                if hit.row_idx < row_boxes.len() {
                    handle_steam_search_result(
                        &state,
                        &row_boxes[hit.row_idx],
                        &steam,
                        &hit.name,
                        hit.db_id,
                        hit.matched,
                        &parent_dialog,
                    );
                }
            }
        },
    );
}

fn start_sgdb_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    row_action_boxes: &[gtk4::Box],
    dialog: &adw::Dialog,
) {
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_sgdb_match(g))
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
            platform_id: g.platform_id.clone(),
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        150,
        0,
        {
            let steam = steam.clone();
            move |item| {
                steam
                    .search_sgdb(&item.name)
                    .first()
                    .map(|(sid, name)| (sid.clone(), name.clone()))
            }
        },
        {
            let state = state.clone();
            let row_boxes = row_action_boxes.to_vec();
            let parent_dialog = dialog.clone();
            move |hit| {
                if hit.row_idx < row_boxes.len() {
                    handle_unified_sgdb_result(
                        &state,
                        &row_boxes[hit.row_idx],
                        hit.db_id,
                        &hit.name,
                        hit.matched,
                        &parent_dialog,
                    );
                }
            }
        },
    );
}

/// ScreenScraper's batch pass: search each unmatched entry by name and
/// take the first hit's metadata. One request a second — the service
/// rate-limits harder than SGDB.
fn start_scraper_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    scraped_ids: &HashSet<i64>,
    scraper_row_boxes: &[gtk4::Box],
) {
    let creds = ira_api::screenscraper::ScraperCreds {
        user: state.borrow().cfg.screenscraper_id.clone(),
        password: state.borrow().cfg.screenscraper_password.clone(),
    };
    if !creds.is_configured() {
        return;
    }

    // The queue indexes the ScreenScraper rows, not the main list.
    let queue: Vec<BatchItem> = needs_matching
        .iter()
        .filter(|g| needs_scraper_match(g, scraped_ids))
        .cloned()
        .enumerate()
        .map(|(row_idx, g)| BatchItem {
            name: g.name.clone(),
            db_id: g.db_id,
            row_idx,
            platform_id: g.platform_id.clone(),
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    // A credentials rejection fails every request identically: once the
    // worker sees one, remaining rows report that instead of a misleading
    // "no match".
    let rejected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    run_batch(
        queue,
        50,
        1100,
        {
            let steam = steam.clone();
            let rejected = std::sync::Arc::clone(&rejected);
            move |item| {
                match steam.screenscraper_search(&creds, &item.name, &item.platform_id) {
                    Ok(games) => games.into_iter().next(),
                    Err(e) => {
                        if e.contains("rejected the credentials") {
                            rejected.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        eprintln!("ScreenScraper batch search failed: {e}");
                        None
                    }
                }
            }
        },
        {
            let state = state.clone();
            let row_boxes = scraper_row_boxes.to_vec();
            let rejected = std::sync::Arc::clone(&rejected);
            move |hit| {
                if hit.row_idx >= row_boxes.len() {
                    return;
                }
                let row = &row_boxes[hit.row_idx];
                if rejected.load(std::sync::atomic::Ordering::Relaxed) && hit.matched.is_none()
                {
                    clear_children(row);
                    row.append(&status_label(
                        &crate::tr!("SS: credentials rejected"),
                        CSS_DIM_LABEL,
                    ));
                    return;
                }
                match hit.matched {
                    Some(game) => {
                        let timestamp =
                            ira_db::scraper_release_timestamp(&game.release_date);
                        if let Err(e) = ira_db::store_scraper_metadata(
                            &state.borrow().db,
                            hit.db_id,
                            &game.metadata(timestamp),
                        ) {
                            eprintln!("Failed to store ScreenScraper metadata: {e}");
                            return;
                        }
                        clear_children(row);
                        row.append(&status_label(
                            &crate::tr!("SS: matched"),
                            CSS_SUCCESS_LABEL,
                        ));
                    }
                    None => {
                        // No hit: settle the row and leave the manual
                        // search open as the way forward.
                        clear_children(row);
                        row.append(&status_label(
                            &crate::tr!("SS: no match"),
                            CSS_DIM_LABEL,
                        ));
                    }
                }
            }
        },
    );
}

pub fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    let (needs_matching, title_map, scraped_ids) = collect_unmatched_games(state);

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

    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Match unmatched games"));
    dialog.set_content_width(600);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header = gtk4::Label::new(Some(&crate::tr!("{} game(s) to match").replacen(
        "{}",
        &needs_matching.len().to_string(),
        1,
    )));
    header.set_xalign(0.0);
    header.add_css_class(CSS_HEADING);
    content.append(&super::helpers::clamped(&header, 600, (12, 8, 12, 12)));

    let (scrolled, list) = super::helpers::clamped_boxed_list(600);
    // Size to the rows when few, cap and scroll when many.
    scrolled.set_propagate_natural_height(true);
    scrolled.set_min_content_height(160);
    scrolled.set_max_content_height(500);
    let (row_action_boxes, scraper_row_boxes) =
        populate_match_list(&list, &needs_matching, state, &dialog, &scraped_ids);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&window));

    start_steam_batch_matching(
        state,
        &needs_matching,
        title_map,
        &row_action_boxes,
        &dialog,
    );
    start_sgdb_batch_matching(state, &needs_matching, &row_action_boxes, &dialog);
    start_scraper_batch_matching(state, &needs_matching, &scraped_ids, &scraper_row_boxes);
}

fn create_match_row(list: &gtk4::ListBox, name: &str, searching_text: &str) -> gtk4::Box {
    let row = adw::ActionRow::new();
    // Game titles are shown as typed — "Fear & Hunger" is not markup.
    row.set_use_markup(false);
    row.set_title(name);
    // Long local names wrap to two lines at most, then ellipsize, so the
    // suffix status label and buttons keep a usable share of the row width.
    row.set_title_lines(2);

    let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    action_box.set_valign(gtk4::Align::Center);
    action_box.append(&status_label(searching_text, CSS_DIM_LABEL));
    row.add_suffix(&action_box);

    list.append(&row);
    action_box
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
}
