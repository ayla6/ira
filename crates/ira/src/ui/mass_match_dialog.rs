use crate::Game;
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use super::css::*;
use super::helpers::clear_children;
use super::ra_match_dialog::show_ra_search_dialog;
use super::sgdb_match_dialog::handle_unified_sgdb_result;
use super::state::SharedState;
use super::steam_search_dialog::handle_steam_search_result;

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

fn collect_unmatched_games(state: &SharedState) -> (Vec<Game>, Vec<(String, String, String)>) {
    let s = state.borrow();
    let games = s.games.clone();
    let needs_matching: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            needs_steam_match(g)
                || (g.kind == ira_models::GameKind::Retro
                    && g.trophy_source == ira_models::TrophySource::Empty
                    && !g.manual_unmatch)
                || needs_sgdb_match(g)
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
    dialog: &adw::Window,
) -> Vec<gtk4::Box> {
    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();

    for game in needs_matching.iter() {
        let action_box = if game.kind == ira_models::GameKind::Retro
            && game.trophy_source == ira_models::TrophySource::Empty
        {
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
                        let label = gtk4::Label::new(Some(&crate::tr!("RA: matched")));
                        label.add_css_class(CSS_SUCCESS_LABEL);
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

fn start_steam_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    title_map: Vec<(String, String, String)>,
    row_action_boxes: &[gtk4::Box],
    dialog: &adw::Window,
) {
    let steam_games: Vec<(String, i64, ira_models::GameKind)> = needs_matching
        .iter()
        .filter(|g| needs_steam_match(g))
        .map(|g| (g.name.clone(), g.db_id, g.kind))
        .collect();
    let steam_row_indices: Vec<usize> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_steam_match(g))
        .map(|(i, _)| i)
        .collect();

    if steam_games.is_empty() {
        return;
    }

    let (steam_tx, steam_rx) =
        std::sync::mpsc::channel::<(usize, Option<(String, String)>, String, i64)>();
    let steam_rx = std::cell::RefCell::new(steam_rx);
    let steam_remaining = Cell::new(steam_games.len());

    let steam = state.borrow().steam.clone();

    {
        let steam = steam.clone();
        std::thread::spawn(move || {
            for (i, (game_name, db_id, _kind)) in steam_games.iter().enumerate() {
                let norm = normalize_title(game_name);

                let matched: Option<(String, String)> = if norm.is_empty() {
                    None
                } else {
                    title_map
                        .iter()
                        .find(|(t, _, _)| t == &norm)
                        .map(|(_, id, name)| (id.clone(), name.clone()))
                };

                let final_match = if matched.is_some() {
                    matched
                } else {
                    let results = steam.search_steam_store(game_name);
                    if results.is_empty() {
                        None
                    } else {
                        results
                            .iter()
                            .find(|(_, name)| normalize_title(name) == norm)
                            .map(|(id, name)| (id.clone(), name.clone()))
                    }
                };

                let _ = steam_tx.send((i, final_match, game_name.clone(), *db_id));
            }
        });
    }

    let state_rx = state.clone();
    let steam_rx_steam = steam;
    let row_boxes = row_action_boxes.to_vec();
    let parent_dialog = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok((idx, matched, game_name, db_id)) = steam_rx.borrow_mut().try_recv() {
            if let Some(&row_idx) = steam_row_indices.get(idx) {
                if row_idx < row_boxes.len() {
                    handle_steam_search_result(
                        &state_rx,
                        &row_boxes[row_idx],
                        &steam_rx_steam,
                        &game_name,
                        db_id,
                        matched,
                        &parent_dialog,
                    );
                }
            }
            let left = steam_remaining.get();
            if left <= 1 {
                return glib::ControlFlow::Break;
            }
            steam_remaining.set(left - 1);
        }
        glib::ControlFlow::Continue
    });
}

fn start_sgdb_batch_matching(
    state: &SharedState,
    needs_matching: &[Game],
    row_action_boxes: &[gtk4::Box],
    dialog: &adw::Window,
) {
    let sgdb_games: Vec<(String, i64, usize)> = needs_matching
        .iter()
        .enumerate()
        .filter(|(_, g)| needs_sgdb_match(g))
        .map(|(row_idx, g)| (g.name.clone(), g.db_id, row_idx))
        .collect();

    if sgdb_games.is_empty() {
        return;
    }

    let (sgdb_tx, sgdb_rx) =
        std::sync::mpsc::channel::<(usize, Option<(String, String)>, i64, String)>();
    let sgdb_rx = std::cell::RefCell::new(sgdb_rx);
    let sgdb_remaining = Cell::new(sgdb_games.len());

    {
        let sgdb_games = sgdb_games.clone();
        let steam_sgdb = state.borrow().steam.clone();
        std::thread::spawn(move || {
            for (i, (game_name, db_id, _row_idx)) in sgdb_games.iter().enumerate() {
                let results = steam_sgdb.search_sgdb(game_name);
                let matched = results
                    .first()
                    .map(|(sid, name)| (sid.clone(), name.clone()));
                let _ = sgdb_tx.send((i, matched, *db_id, game_name.clone()));
            }
        });
    }

    let state_sgdb = state.clone();
    let parent_dialog_sgdb = dialog.clone();
    let sgdb_row_boxes = row_action_boxes.to_vec();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if let Ok((idx, matched, db_id, game_name)) = sgdb_rx.borrow_mut().try_recv() {
            if let Some(row_idx) = sgdb_games.get(idx).map(|(_, _, r)| *r) {
                if row_idx < sgdb_row_boxes.len() {
                    handle_unified_sgdb_result(
                        &state_sgdb,
                        &sgdb_row_boxes[row_idx],
                        db_id,
                        &game_name,
                        matched,
                        &parent_dialog_sgdb,
                    );
                }
            }
            let left = sgdb_remaining.get();
            if left <= 1 {
                return glib::ControlFlow::Break;
            }
            sgdb_remaining.set(left - 1);
        }
        glib::ControlFlow::Continue
    });
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

    let dialog = adw::Window::new();
    dialog.set_default_width(600);
    dialog.set_default_height(500);
    dialog.set_transient_for(Some(&window));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!(
        "Match unmatched games"
    )))));
    outer.append(&header_bar);

    let header = gtk4::Label::new(Some(&crate::tr!("{} game(s) to match").replacen(
        "{}",
        &needs_matching.len().to_string(),
        1,
    )));
    header.set_margin_top(16);
    header.set_margin_bottom(8);
    header.set_margin_start(16);
    header.set_margin_end(16);
    header.set_xalign(0.0);
    header.add_css_class(CSS_HEADING);
    outer.append(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);

    let row_action_boxes = populate_match_list(&list, &needs_matching, state, &dialog);

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label(&crate::tr!("Close"));
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_top(8);
    close_btn.set_margin_bottom(12);
    close_btn.set_margin_start(16);
    close_btn.set_margin_end(16);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));
    dialog.present();

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
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.set_margin_top(6);
    row.set_margin_bottom(6);

    let name_label = gtk4::Label::new(Some(name));
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&name_label);

    let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let searching = gtk4::Label::new(Some(searching_text));
    searching.add_css_class(CSS_DIM_LABEL);
    action_box.append(&searching);
    row.append(&action_box);

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
    fn test_needs_sgdb_match_skips_steam_enriched_games() {
        let mut g = game(ira_models::GameKind::Wine);
        g.app_id = "420530".to_string();
        assert!(!needs_sgdb_match(&g), "steam-driven enrichment owns these");
    }
}
