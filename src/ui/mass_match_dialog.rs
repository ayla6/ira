use gtk4::prelude::*;
use adw::prelude::*;
use crate::api::SteamClient;
use crate::Game;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::state::SharedState;
use super::sidebar::rebuild_sidebar;
use super::matching::{match_game_to_steam, match_game_to_sgdb};
use super::dialogs::build_image_manager_content;
use super::helpers::clear_children;

pub fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    let suffixes = ["the", "final", "cut", "edition", "complete", "definitive", "remastered", "hd"];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

#[derive(Clone, Copy)]
pub enum SearchSource {
    Steam,
    SGDB,
}

pub fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    let (needs_matching, title_map) = {
        let s = state.borrow();
        let games = s.games.clone();
        let needs_matching: Vec<Game> = games.into_iter().filter(|g| {
            (g.app_id.is_empty() && !g.manual_unmatch)
            || (g.kind == "retro" && g.trophy_source.is_empty())
            || (g.sgdb_id.is_empty() && (g.app_id.is_empty() || g.kind == "retro"))
        }).collect();
        let save_dir = &s.save_dir;
        let data_dir = std::path::Path::new(save_dir).join("data").join("steam");
        let mut map: Vec<(String, String, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let app_id = match entry.file_name().to_str() {
                    Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                    _ => continue,
                };
                if let Some(name) = crate::parser::read_app_name(save_dir, &app_id) {
                    map.push((normalize_title(&name), app_id, name));
                }
            }
        }
        (needs_matching, map)
    };

    if needs_matching.is_empty() {
        let d = adw::AlertDialog::new(Some("Nothing to match"), Some("Every game already has a trophy source and image assets linked."));
        d.add_response("ok", "OK");
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
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("Match unmatched games"))));
    outer.append(&header_bar);

    let header = gtk4::Label::new(Some(&format!("{} game(s) to match", needs_matching.len())));
    header.set_margin_top(16);
    header.set_margin_bottom(8);
    header.set_margin_start(16);
    header.set_margin_end(16);
    header.set_xalign(0.0);
    header.add_css_class("heading");
    outer.append(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);

    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();

    for game in &needs_matching {
        let searching_text = if game.app_id.is_empty() && !g_manual_unmatch(game) {
            "Searching Steam..."
        } else if game.kind == "retro" && game.trophy_source.is_empty() {
            "Searching RA..."
        } else {
            "Searching SGDB..."
        };
        let action_box = create_match_row(&list, &game.name, searching_text);
        row_action_boxes.push(action_box);
    }

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
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

    let steam = state.borrow().steam.clone();

    // --- Steam auto-batch thread (Lutris games without app_id) ---
    let steam_games: Vec<(String, i64, i64, String)> = needs_matching.iter()
        .filter(|g| g.app_id.is_empty() && !g.manual_unmatch)
        .enumerate()
        .map(|(_row_idx, g)| (g.name.clone(), g.lutris_id, g.db_id, g.kind.clone()))
        .map(|(name, lutris_id, db_id, kind)| (name, lutris_id, db_id, kind))
        .collect();
    let steam_row_indices: Vec<usize> = needs_matching.iter().enumerate()
        .filter(|(_, g)| g.app_id.is_empty() && !g.manual_unmatch)
        .map(|(i, _)| i)
        .collect();

    let (steam_tx, steam_rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, String, i64)>();
    let steam_rx = std::cell::RefCell::new(steam_rx);
    let steam_remaining = Cell::new(steam_games.len());

    {
        let steam_games = steam_games.clone();
        let title_map = title_map.clone();
        let steam = steam.clone();
        std::thread::spawn(move || {
            for (i, (game_name, lutris_id, _db_id, _kind)) in steam_games.iter().enumerate() {
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

                let _ = steam_tx.send((i, final_match, game_name.clone(), *lutris_id));
            }
        });
    }

    let state_rx = state.clone();
    let steam_rx_steam = state.borrow().steam.clone();
    let row_boxes = row_action_boxes.clone();
    let parent_dialog = dialog.clone();
    let steam_row_indices = steam_row_indices.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok((idx, matched, game_name, lutris_id)) = steam_rx.borrow_mut().try_recv() {
            if let Some(&row_idx) = steam_row_indices.get(idx) {
                if row_idx < row_boxes.len() {
                    handle_steam_search_result(
                        &state_rx, &row_boxes[row_idx], &steam_rx_steam,
                        &game_name, lutris_id, matched, &parent_dialog,
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

    // --- SGDB auto-batch thread (games without sgdb_id, including RA-matched retro games) ---
    let sgdb_games: Vec<(String, i64, usize)> = needs_matching.iter().enumerate()
        .filter(|(_, g)| g.sgdb_id.is_empty() && (g.app_id.is_empty() || g.kind == "retro"))
        .map(|(row_idx, g)| (g.name.clone(), g.db_id, row_idx))
        .collect();
    let (sgdb_tx, sgdb_rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, i64, String)>();
    let sgdb_rx = std::cell::RefCell::new(sgdb_rx);
    let sgdb_remaining = Cell::new(sgdb_games.len());

    {
        let sgdb_games = sgdb_games.clone();
        let steam_sgdb = state.borrow().steam.clone();
        std::thread::spawn(move || {
            for (i, (game_name, db_id, _row_idx)) in sgdb_games.iter().enumerate() {
                let results = steam_sgdb.search_sgdb(game_name);
                let matched = results.first().map(|(sid, name)| (sid.clone(), name.clone()));
                let _ = sgdb_tx.send((i, matched, *db_id, game_name.clone()));
            }
        });
    }

    let state_sgdb = state.clone();
    let parent_dialog_sgdb = dialog.clone();
    let sgdb_row_boxes = row_action_boxes.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if let Ok((idx, matched, db_id, game_name)) = sgdb_rx.borrow_mut().try_recv() {
            if let Some(row_idx) = sgdb_games.get(idx).map(|(_, _, r)| *r) {
                if row_idx < sgdb_row_boxes.len() {
                    handle_unified_sgdb_result(
                        &state_sgdb, &sgdb_row_boxes[row_idx], db_id,
                        &game_name, matched, &parent_dialog_sgdb,
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

fn g_manual_unmatch(g: &Game) -> bool {
    g.manual_unmatch
}

fn handle_unified_sgdb_result(
    state: &SharedState,
    action_box: &gtk4::Box,
    db_id: i64,
    game_name: &str,
    matched: Option<(String, String)>,
    parent_dialog: &adw::Window,
) {
    // Only update if the action box still shows a searching state
    // (don't overwrite a Steam match result)
    let has_result = action_box.last_child().is_some_and(|c| {
        c.downcast_ref::<gtk4::Label>().is_some_and(|l| l.text().starts_with("Matched") || l.text().starts_with("Not found") || l.text().starts_with("Enter"))
    });
    if has_result {
        // Steam result already shown — just add SGDB status beside it
        // Skip for now; the user can manually search SGDB
        return;
    }

    clear_children(action_box);

    if let Some((sgdb_id, matched_name)) = matched {
        let _ = crate::db::set_sgdb_id(&state.borrow().db, db_id, &sgdb_id);
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
            g.sgdb_id = sgdb_id.clone();
        }
        let steam_dl = state.borrow().steam.clone();
        let sender = state.borrow().sender.clone();
        let sgdb_id_dl = sgdb_id.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (icon, hero, grid, logo, header) = steam_dl.ensure_sgdb_assets(&sgdb_id_dl);
            let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                db_id, sgdb_id: sgdb_id_dl, icon, hero, grid, logo, header,
            });
        });

        let label = gtk4::Label::new(Some(&format!("SGDB: {}", matched_name)));
        label.add_css_class("success-label");
        action_box.append(&label);

        let undo_btn = gtk4::Button::with_label("Undo SGDB");
        let sc = state.clone();
        undo_btn.connect_clicked(move |_| {
            let _ = crate::db::set_sgdb_id(&sc.borrow().db, db_id, "");
            {
                let mut s = sc.borrow_mut();
                if let Some(g) = s.games.iter_mut().find(|g| g.db_id == db_id) {
                    g.sgdb_id.clear();
                    g.icon_path.clear();
                    g.hero_image_path.clear();
                    g.grid_path.clear();
                    g.header_path.clear();
                    g.logo_path.clear();
                }
            }
            let (db, save_dir, app_id) = {
                let s = sc.borrow();
                let app_id = s.games.iter().find(|g| g.db_id == db_id).map(|g| g.app_id.clone()).unwrap_or_default();
                (s.db.clone(), s.save_dir.clone(), app_id)
            };
            if let Some(entry) = crate::db::find_by_steam_id(&db, &app_id).ok().flatten() {
                if let Ok(game) = crate::parser::load_game(&entry, &save_dir) {
                    let mut s = sc.borrow_mut();
                    if let Some(g) = s.games.iter_mut().find(|g| g.db_id == db_id) {
                        g.icon_path = game.icon_path;
                        g.hero_image_path = game.hero_image_path;
                        g.grid_path = game.grid_path;
                        g.header_path = game.header_path;
                        g.logo_path = game.logo_path;
                    }
                }
            }
            let selected_id = sc.borrow().selected_id.clone();
            if selected_id == db_id.to_string() {
                let game = sc.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
                if let Some(game) = game {
                    super::game_display::display_game(&game, &sc);
                }
            }
            super::helpers::refresh_settings_images_page(&sc, db_id, |s, game, win| {
                build_image_manager_content(s, game, win).upcast()
            });
            let is_grid_showing = sc.borrow().selected_id.is_empty() && !sc.borrow().content_unloaded;
            if is_grid_showing {
                super::grid_view::show_grid_view(&sc);
            }
        });
        action_box.append(&undo_btn);
    } else {
        let label = gtk4::Label::new(Some("SGDB: not found"));
        label.add_css_class("dim-label");
        action_box.append(&label);

        let sgdb_btn = gtk4::Button::with_label("Search SGDB…");
        let sc = state.clone();
        let gn = game_name.to_string();
        let did = db_id;
        let dlg = parent_dialog.clone();
        let ab = action_box.clone();
        sgdb_btn.connect_clicked(move |_| {
            let cb: Rc<dyn Fn()> = Rc::new({
                let ab = ab.clone();
                let name = gn.clone();
                move || {
                    clear_children(&ab);
                    let label = gtk4::Label::new(Some(&format!("Matched to SGDB: {}", name)));
                    label.add_css_class("success-label");
                    ab.append(&label);
                }
            });
            show_sgdb_search_dialog(&sc, did, &gn, &dlg, Some(cb));
        });
        action_box.append(&sgdb_btn);
    }
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
    searching.add_css_class("dim-label");
    action_box.append(&searching);
    row.append(&action_box);

    list.append(&row);
    action_box
}

fn handle_steam_search_result(
    state: &SharedState,
    action_box: &gtk4::Box,
    steam: &Arc<SteamClient>,
    game_name: &str,
    lutris_id: i64,
    matched: Option<(String, String)>,
    parent_dialog: &adw::Window,
) {
    clear_children(action_box);

    if let Some((sid, matched_name)) = matched {
        match_game_to_steam(state, lutris_id, sid.clone(), game_name.to_string());

        let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sid)));
        label.add_css_class("success-label");
        action_box.append(&label);

        let undo_btn = gtk4::Button::with_label("Undo");
        let sc = state.clone();
        undo_btn.connect_clicked(move |_| {
            let _ = crate::db::unmatch_game(&sc.borrow().db, lutris_id);
            if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.app_id.clear();
                g.kind.clear();
                g.achievements.clear();
                g.manual_unmatch = true;
            }
            rebuild_sidebar(&sc);
        });
        action_box.append(&undo_btn);
    } else {
        let label = gtk4::Label::new(Some("Not found"));
        label.add_css_class("dim-label");
        action_box.append(&label);

        let ab = action_box.clone();
        let on_match: Rc<dyn Fn(&str, &str)> = Rc::new(move |sid, name| {
            clear_children(&ab);
            let text = if name.is_empty() {
                format!("Matched: {}", sid)
            } else {
                format!("Matched: {} ({})", name, sid)
            };
            let l = gtk4::Label::new(Some(&text));
            l.add_css_class("success-label");
            ab.append(&l);
        });

        let id_btn = gtk4::Button::with_label("Enter ID");
        let sc = state.clone();
        let name = game_name.to_string();
        let cb = on_match.clone();
        id_btn.connect_clicked(move |_| {
            let sc2 = sc.clone();
            let name2 = name.clone();
            let cb2 = cb.clone();
            let body = format!("Enter the Steam app ID for \u{201C}{}\u{201D}:", name);
            super::add_game::prompt_for_steam_id(&sc, "Match to Steam", &body, move |app_id| {
                match_game_to_steam(&sc2, lutris_id, app_id.to_string(), name2.clone());
                cb2(&app_id, "");
            });
        });
        action_box.append(&id_btn);

        let steam_btn = gtk4::Button::with_label("Search Steam");
        let sc2 = state.clone();
        let name2 = game_name.to_string();
        let steam2 = steam.clone();
        let cb2 = on_match.clone();
        let pd = parent_dialog.clone();
        steam_btn.connect_clicked(move |_| {
            show_search_results_dialog(&sc2, steam2.clone(), "Steam", &name2, lutris_id, SearchSource::Steam, cb2.clone(), pd.upcast_ref());
        });
        action_box.append(&steam_btn);

        let sgdb_btn = gtk4::Button::with_label("Search SGDB");
        let sc3 = state.clone();
        let name3 = game_name.to_string();
        let steam3 = steam.clone();
        let cb3 = on_match.clone();
        let pd = parent_dialog.clone();
        sgdb_btn.connect_clicked(move |_| {
            show_search_results_dialog(&sc3, steam3.clone(), "SteamGridDB", &name3, lutris_id, SearchSource::SGDB, cb3.clone(), pd.upcast_ref());
        });
        action_box.append(&sgdb_btn);
    }
}

pub fn show_search_results_dialog(
    state: &SharedState,
    steam: Arc<SteamClient>,
    source_name: &str,
    game_name: &str,
    lutris_id: i64,
    source: SearchSource,
    on_match: Rc<dyn Fn(&str, &str)>,
    parent: &gtk4::Window,
) {
    let dialog = adw::Window::new();
    dialog.set_default_width(450);
    dialog.set_default_height(400);
    dialog.set_transient_for(Some(parent));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    let title_label = gtk4::Label::new(Some(&format!("Search {}", source_name)));
    header_bar.set_title_widget(Some(&title_label));
    outer.append(&header_bar);

    let entry = gtk4::Entry::new();
    entry.set_text(game_name);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(12);
    entry.set_margin_bottom(8);
    outer.append(&entry);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let results_list = gtk4::ListBox::new();
    results_list.set_selection_mode(gtk4::SelectionMode::None);
    results_list.set_margin_start(12);
    results_list.set_margin_end(12);
    results_list.set_margin_bottom(8);

    let placeholder = gtk4::Label::new(Some("Searching..."));
    placeholder.add_css_class("dim-label");
    results_list.append(&placeholder);

    scrolled.set_child(Some(&results_list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));

    let entry_clone = entry.clone();
    let results_clone = results_list.clone();
    let state_clone = state.clone();
    let steam_clone = steam.clone();
    let name_clone = game_name.to_string();
    let dialog_clone = dialog.clone();
    let on_match_clone = on_match.clone();
    entry.connect_activate(move |_| {
        let term = entry_clone.text().to_string();
        if term.is_empty() {
            return;
        }

        clear_children(&results_clone);
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        results_clone.append(&searching);

        let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
        let rx = std::cell::RefCell::new(rx);

        let steam = steam_clone.clone();
        let src = source;
        std::thread::spawn(move || {
            let search_results = match src {
                SearchSource::Steam => steam.search_steam_store(&term),
                SearchSource::SGDB => steam.search_sgdb(&term),
            };
            let _ = tx.send(search_results);
        });

        let sc = state_clone.clone();
        let results = results_clone.clone();
        let name = name_clone.clone();
        let cb = on_match_clone.clone();
        let dlg = dialog_clone.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok(search_results) = rx.borrow_mut().try_recv() {
                clear_children(&results);

                if search_results.is_empty() {
                    let none = gtk4::Label::new(Some("No results found"));
                    none.add_css_class("dim-label");
                    results.append(&none);
                    return glib::ControlFlow::Break;
                }

                for (app_id, result_name) in search_results {
                    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    row.set_margin_top(4);
                    row.set_margin_bottom(4);

                    let label = gtk4::Label::new(Some(&format!("{} ({})", result_name, app_id)));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    row.append(&label);

                    let match_btn = gtk4::Button::with_label("Match");
                    match_btn.add_css_class("suggested-action");
                    let sc2 = sc.clone();
                    let name2 = name.clone();
                    let sid = app_id.clone();
                    let matched_name = result_name.clone();
                    let lid = lutris_id;
                    let dialog_clone = dlg.clone();
                    let callback = cb.clone();
                    let src_type = source;
                    match_btn.connect_clicked(move |_| {
                        match src_type {
                            SearchSource::Steam => match_game_to_steam(&sc2, lid, sid.clone(), name2.clone()),
                            SearchSource::SGDB => match_game_to_sgdb(&sc2, lid, sid.clone(), name2.clone()),
                        }
                        callback(&sid, &matched_name);
                        dialog_clone.close();
                    });
                    row.append(&match_btn);

                    results.append(&row);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });

    dialog.present();
    entry.emit_activate();
}

pub fn show_sgdb_search_dialog(state: &SharedState, db_id: i64, game_name: &str, parent: &adw::Window, on_match: Option<Rc<dyn Fn()>>) {
    let dialog = adw::Window::new();
    dialog.set_default_width(500);
    dialog.set_default_height(400);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some("Match to SteamGridDB"))));
    outer.append(&header);

    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Game name…"));
    entry.set_text(game_name);
    entry.set_hexpand(true);
    let search_btn = gtk4::Button::with_label("Search");
    search_btn.add_css_class("suggested-action");
    search_box.append(&entry);
    search_box.append(&search_btn);
    outer.append(&search_box);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_margin_top(8);
    let list = gtk4::ListBox::new();
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(8);
    list.set_margin_bottom(12);
    list.set_valign(gtk4::Align::Start);
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let state_c = state.clone();
    let dialog_c = dialog.clone();
    let list_c = list.clone();

    let entry_s = entry.clone();
    let do_search = move || {
        let term = entry_s.text().to_string();
        if term.is_empty() {
            return;
        }
        let steam = state_c.borrow().steam.clone();
        let results_shared = Arc::new(std::sync::Mutex::new(None::<Vec<(String, String)>>));
        let results_thread = results_shared.clone();
        std::thread::spawn(move || {
            let r = steam.search_sgdb(&term);
            *results_thread.lock().unwrap() = Some(r);
        });
        let results_poll = results_shared.clone();
        let list_c2 = list_c.clone();
        let dialog_c2 = dialog_c.clone();
        let state_c2 = state_c.clone();
        let on_match_clone = on_match.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !dialog_c2.is_visible() {
                return glib::ControlFlow::Break;
            }
            if let Some(results) = results_poll.lock().unwrap().take() {
                clear_children(&list_c2);
                if results.is_empty() {
                    let row = adw::ActionRow::new();
                    row.set_title("No results found");
                    row.set_sensitive(false);
                    list_c2.append(&row);
                } else {
                    for (sgdb_id, name) in &results {
                        let row = adw::ActionRow::new();
                        row.set_title(name);
                        row.set_subtitle(&format!("SGDB ID: {}", sgdb_id));
                        let match_btn = gtk4::Button::with_label("Match");
                        match_btn.add_css_class("suggested-action");
                        match_btn.set_valign(gtk4::Align::Center);
                        let sgdb_id_c = sgdb_id.clone();
                        let state_c3 = state_c2.clone();
                        let dialog_c3 = dialog_c2.clone();
                        let on_match_cb = on_match_clone.clone();
                        match_btn.connect_clicked(move |_| {
                            let _ = crate::db::set_sgdb_id(&state_c3.borrow().db, db_id, &sgdb_id_c);
                            if let Some(g) = state_c3.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                                g.sgdb_id = sgdb_id_c.clone();
                            }
                            super::helpers::refresh_settings_images_page(&state_c3, db_id, |s, game, win| {
                                build_image_manager_content(s, game, win).upcast()
                            });
                            let steam = state_c3.borrow().steam.clone();
                            let sgdb_id_d = sgdb_id_c.clone();
                            let sender = state_c3.borrow().sender.clone();
                            let db_id_for_msg = db_id;
                            std::thread::spawn(move || {
                                let (icon, hero, grid, logo, header) = steam.ensure_sgdb_assets(&sgdb_id_d);
                                let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                                    db_id: db_id_for_msg,
                                    sgdb_id: sgdb_id_d,
                                    icon, hero, grid, logo, header,
                                });
                            });
                            dialog_c3.close();
                            if let Some(ref cb) = on_match_cb {
                                cb();
                            }
                        });
                        row.add_suffix(&match_btn);
                        list_c2.append(&row);
                    }
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    };

    let ds = do_search.clone();
    entry.connect_activate(move |_| ds());
    let ds2 = do_search.clone();
    search_btn.connect_clicked(move |_| ds2());

    dialog.set_content(Some(&outer));
    dialog.present();
    do_search();
}
