use gtk4::prelude::*;
use adw::prelude::*;
use crate::api::SteamClient;
use crate::Game;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::state::{SharedState, SAVE_DIR};
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

    let (unmatched, title_map) = {
        let s = state.borrow();
        let games = s.games.clone();
        let unmatched: Vec<Game> = games.into_iter().filter(|g| g.app_id.is_empty() && !g.manual_unmatch).collect();
        let data_dir = std::path::Path::new(SAVE_DIR).join("data").join("steam");
        let mut map: Vec<(String, String, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let app_id = match entry.file_name().to_str() {
                    Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                    _ => continue,
                };
                if let Some(name) = crate::parser::read_app_name(SAVE_DIR, &app_id) {
                    map.push((normalize_title(&name), app_id, name));
                }
            }
        }
        (unmatched, map)
    };

    if unmatched.is_empty() {
        let d = adw::AlertDialog::new(Some("No unmatched games"), Some("Every game already has a trophy source linked."));
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

    let header = gtk4::Label::new(Some(&format!("{} unmatched game(s)", unmatched.len())));
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

    for game in &unmatched {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);

        let name_label = gtk4::Label::new(Some(&game.name));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&name_label);

        let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        action_box.append(&searching);
        row.append(&action_box);

        list.append(&row);
        row_action_boxes.push(action_box);
    }

    let ps4_unmatched: Vec<Game> = {
        let s = state.borrow();
        s.games.iter().filter(|g| g.kind == "ps4" && g.sgdb_id.is_empty()).cloned().collect()
    };
    let mut ps4_row_boxes: Vec<(gtk4::Box, i64, String)> = Vec::new();
    if !ps4_unmatched.is_empty() {
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(12);
        sep.set_margin_bottom(8);
        list.append(&sep);

        let ps4_header = gtk4::Label::new(Some(&format!("PS4 Games (match SGDB images) — {}", ps4_unmatched.len())));
        ps4_header.set_margin_start(12);
        ps4_header.set_margin_bottom(6);
        ps4_header.set_xalign(0.0);
        ps4_header.add_css_class("heading");
        list.append(&ps4_header);

        for game in &ps4_unmatched {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_margin_start(12);
            row.set_margin_end(12);
            row.set_margin_top(6);
            row.set_margin_bottom(6);

            let name_label = gtk4::Label::new(Some(&game.name));
            name_label.set_xalign(0.0);
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            row.append(&name_label);

            let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            let searching = gtk4::Label::new(Some("Searching SGDB..."));
            searching.add_css_class("dim-label");
            action_box.append(&searching);
            row.append(&action_box);

            list.append(&row);
            ps4_row_boxes.push((action_box, game.db_id, game.name.clone()));
        }
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

    let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, String, i64)>();
    let rx = std::cell::RefCell::new(rx);
    let remaining = Cell::new(unmatched.len());

    let steam = state.borrow().steam.clone();
    let games_for_thread: Vec<(String, i64)> = unmatched.iter().map(|g| (g.name.clone(), g.lutris_id)).collect();
    std::thread::spawn(move || {
        for (i, (game_name, lutris_id)) in games_for_thread.iter().enumerate() {
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

            let _ = tx.send((i, final_match, game_name.clone(), *lutris_id));
        }
    });

    let state_rx = state.clone();
    let steam_rx = state.borrow().steam.clone();
    let row_boxes = row_action_boxes.clone();
    let parent_dialog = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok((row_idx, matched, game_name, lutris_id)) = rx.borrow_mut().try_recv() {
            if row_idx < row_boxes.len() {
                let action_box = &row_boxes[row_idx];
                clear_children(action_box);

                if let Some((sid, matched_name)) = matched {
                    match_game_to_steam(&state_rx, lutris_id, sid.clone(), game_name.clone());

                    let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sid)));
                    label.add_css_class("success-label");
                    action_box.append(&label);

                    let undo_btn = gtk4::Button::with_label("Undo");
                    let sc = state_rx.clone();
                    let lid = lutris_id;
                    undo_btn.connect_clicked(move |_| {
                        let _ = crate::db::unmatch_game(&sc.borrow().db, lid);
                        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lid) {
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
                    let sc = state_rx.clone();
                    let name = game_name.clone();
                    let lid = lutris_id;
                    let cb = on_match.clone();
                    id_btn.connect_clicked(move |_| {
                        let sc2 = sc.clone();
                        let name2 = name.clone();
                        let cb2 = cb.clone();
                        let body = format!("Enter the Steam app ID for \u{201C}{}\u{201D}:", name);
                        super::add_game::prompt_for_steam_id(&sc, "Match to Steam", &body, move |app_id| {
                            match_game_to_steam(&sc2, lid, app_id.to_string(), name2.clone());
                            cb2(&app_id, "");
                        });
                    });
                    action_box.append(&id_btn);

                    let steam_btn = gtk4::Button::with_label("Search Steam");
                    let sc2 = state_rx.clone();
                    let name2 = game_name.clone();
                    let steam2 = steam_rx.clone();
                    let cb2 = on_match.clone();
                    let pd = parent_dialog.clone();
                    steam_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc2, steam2.clone(), "Steam", &name2, lutris_id, SearchSource::Steam, cb2.clone(), pd.upcast_ref());
                    });
                    action_box.append(&steam_btn);

                    let sgdb_btn = gtk4::Button::with_label("Search SGDB");
                    let sc3 = state_rx.clone();
                    let name3 = game_name.clone();
                    let steam3 = steam_rx.clone();
                    let cb3 = on_match.clone();
                    let pd = parent_dialog.clone();
                    sgdb_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc3, steam3.clone(), "SteamGridDB", &name3, lutris_id, SearchSource::SGDB, cb3.clone(), pd.upcast_ref());
                    });
                    action_box.append(&sgdb_btn);
                }

                let left = remaining.get();
                if left <= 1 {
                    return glib::ControlFlow::Break;
                }
                remaining.set(left - 1);
            }
        }
        glib::ControlFlow::Continue
    });

    if !ps4_row_boxes.is_empty() {
        let (ps4_tx, ps4_rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, i64)>();
        let ps4_rx = std::cell::RefCell::new(ps4_rx);
        let ps4_row_info: Vec<(i64, String)> = ps4_row_boxes.iter().map(|(_, db_id, name)| (*db_id, name.clone())).collect();
        let steam_ps4 = state.borrow().steam.clone();
        let ps4_remaining = Cell::new(ps4_row_boxes.len());
        std::thread::spawn(move || {
            for (i, (db_id, game_name)) in ps4_row_info.iter().enumerate() {
                let results = steam_ps4.search_sgdb(game_name);
                let matched = results.first().map(|(sid, name)| (sid.clone(), name.clone()));
                let _ = ps4_tx.send((i, matched, *db_id));
            }
        });

        let state_ps4 = state.clone();
        let parent_dialog_ps4 = dialog.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok((row_idx, matched, db_id)) = ps4_rx.borrow_mut().try_recv() {
                if row_idx < ps4_row_boxes.len() {
                    let action_box = &ps4_row_boxes[row_idx].0;
                    clear_children(action_box);

                    if let Some((sgdb_id, matched_name)) = matched {
                        let _ = crate::db::set_sgdb_id(&state_ps4.borrow().db, db_id, &sgdb_id);
                        if let Some(g) = state_ps4.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                            g.sgdb_id = sgdb_id.clone();
                        }
                        let steam_dl = state_ps4.borrow().steam.clone();
                        let sender = state_ps4.borrow().sender.clone();
                        let sgdb_id_dl = sgdb_id.clone();
                        std::thread::spawn(move || {
                            let (icon, hero, grid, logo, header) = steam_dl.ensure_sgdb_assets(&sgdb_id_dl);
                            let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                                db_id, sgdb_id: sgdb_id_dl, icon, hero, grid, logo, header,
                            });
                        });

                        let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sgdb_id)));
                        label.add_css_class("success-label");
                        action_box.append(&label);

                        let undo_btn = gtk4::Button::with_label("Undo");
                        let sc = state_ps4.clone();
                        undo_btn.connect_clicked(move |_| {
                            let _ = crate::db::set_sgdb_id(&sc.borrow().db, db_id, "");
                            if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                                g.sgdb_id.clear();
                            }
                            super::helpers::refresh_settings_images_page(&sc, db_id, |s, game, win| {
                                build_image_manager_content(s, game, win).upcast()
                            });
                        });
                        action_box.append(&undo_btn);
                    } else {
                        let label = gtk4::Label::new(Some("Not found on SGDB"));
                        label.add_css_class("dim-label");
                        action_box.append(&label);

                        let sgdb_btn = gtk4::Button::with_label("Search SGDB…");
                        sgdb_btn.add_css_class("suggested-action");
                        let sc = state_ps4.clone();
                        let gn = ps4_row_boxes[row_idx].2.clone();
                        let did = db_id;
                        let dlg = parent_dialog_ps4.clone();
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

                    let left = ps4_remaining.get();
                    if left <= 1 {
                        return glib::ControlFlow::Break;
                    }
                    ps4_remaining.set(left - 1);
                }
            }
            glib::ControlFlow::Continue
        });
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
