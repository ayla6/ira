use crate::Game;
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use super::css::*;
use super::helpers::clear_children;
use super::ra_match_dialog::show_ra_search_dialog;
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

fn collect_unmatched_games(state: &SharedState) -> (Vec<Game>, Vec<(String, String, String)>) {
    let s = state.borrow();
    let games = s.games.clone();
    let needs_matching: Vec<Game> = games
        .into_iter()
        .filter(|g| needs_steam_match(g) || needs_ra_match(g) || needs_sgdb_match(g))
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
    dialog: &adw::Dialog,
) -> Vec<gtk4::Box> {
    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();

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
        } else {
            let searching_text = if needs_steam_match(game) {
                crate::tr!("Searching Steam...")
            } else {
                crate::tr!("Searching SGDB...")
            };
            create_match_row(list, &game.name, &searching_text)
        };
        row_action_boxes.push(action_box);
    }

    row_action_boxes
}

/// One queued batch candidate: the game to match plus which list row its
/// result belongs to.
struct BatchItem {
    name: String,
    db_id: i64,
    row_idx: usize,
}

/// A finished candidate handed from the worker thread back to the UI loop.
struct BatchHit {
    row_idx: usize,
    db_id: i64,
    name: String,
    matched: Option<(String, String)>,
}

/// Shared shape of both batch passes: one sequential worker thread computes
/// matches over `queue`, and results are applied on the UI loop every
/// `interval_ms` until the queue drains. `worker` runs off-thread and must
/// not touch GTK; `on_result` runs on the main loop.
fn run_batch(
    queue: Vec<BatchItem>,
    interval_ms: u64,
    worker: impl Fn(&BatchItem) -> Option<(String, String)> + Send + 'static,
    on_result: impl Fn(BatchHit) + 'static,
) {
    let total = queue.len();
    let (tx, rx) = std::sync::mpsc::channel::<BatchHit>();
    std::thread::spawn(move || {
        for item in &queue {
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
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        50,
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
        })
        .collect();

    if queue.is_empty() {
        return;
    }

    let steam = state.borrow().steam.clone();
    run_batch(
        queue,
        150,
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
    let row_action_boxes = populate_match_list(&list, &needs_matching, state, &dialog);
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
}

fn create_match_row(list: &gtk4::ListBox, name: &str, searching_text: &str) -> gtk4::Box {
    let row = adw::ActionRow::new();
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
