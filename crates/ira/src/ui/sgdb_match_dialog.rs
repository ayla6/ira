use gtk4::prelude::*;
use adw::prelude::*;
use std::rc::Rc;
use std::sync::Arc;

use super::state::SharedState;
use super::image_manager::build_image_manager_content_with_drafts;
use super::helpers::clear_children;
use super::helpers::refresh_settings_images_page;
use super::game_display::display_game;
use super::grid_view::show_grid_view;
use super::css::*;

pub(super) fn handle_unified_sgdb_result(
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
        if let Err(e) = ira_db::set_sgdb_id(&state.borrow().db, db_id, &sgdb_id) {
            eprintln!("Failed to set SGDB ID: {}", e);
        }
        if let Some(g) = state.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
            g.sgdb_id = sgdb_id.clone();
        }
        let steam_dl = state.borrow().steam.clone();
        let sender = state.borrow().sender.clone();
        let sgdb_id_dl = sgdb_id.clone();
        let save_dir = state.borrow().save_dir.clone();
        let is_retro = state.borrow().games.iter().find(|g| g.db_id == db_id).is_some_and(|g| g.kind == ira_models::GameKind::Retro);
        std::thread::spawn(move || {
            let _s = tracing::info_span!("handle_unified_sgdb_result", db_id = db_id, sgdb_id = %sgdb_id_dl).entered();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (icon, hero, grid, logo, header) = if is_retro {
                let dir = ira_parser::retro_data_dir(&save_dir, db_id);
                steam_dl.ensure_sgdb_assets_in_dir(&dir, &sgdb_id_dl)
            } else {
                steam_dl.ensure_sgdb_assets(&sgdb_id_dl)
            };
            let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                db_id, sgdb_id: sgdb_id_dl, icon, hero, grid, logo, header,
            });
        });

        let label = gtk4::Label::new(Some(&format!("SGDB: {}", matched_name)));
        label.add_css_class(CSS_SUCCESS_LABEL);
        action_box.append(&label);

        let undo_btn = gtk4::Button::with_label("Undo SGDB");
        let sc = state.clone();
        let action_box_c = action_box.clone();
        let parent_c = parent_dialog.clone();
        let game_name_c = game_name.to_string();
        undo_btn.connect_clicked(move |_| {
            if let Err(e) = ira_db::set_sgdb_id(&sc.borrow().db, db_id, "") {
                eprintln!("Failed to clear SGDB ID: {}", e);
            }
            if let Err(e) = ira_db::set_manual_unmatch(&sc.borrow().db, db_id, true) {
                eprintln!("Failed to set manual unmatch: {}", e);
            }
            {
                let mut s = sc.borrow_mut();
                if let Some(g) = s.games.iter_mut().find(|g| g.db_id == db_id) {
                    g.sgdb_id.clear();
                    g.manual_unmatch = true;
                    g.icon_path.clear();
                    g.hero_image_path.clear();
                    g.grid_path.clear();
                    g.header_path.clear();
                    g.logo_path.clear();
                }
            }
            let (db, save_dir) = {
                let s = sc.borrow();
                (s.db.clone(), s.save_dir.clone())
            };
            if let Some(entry) = ira_db::find_by_db_id(&db, db_id).ok().flatten() {
                if let Ok(game) = crate::game_loader::load_game(&entry, &save_dir) {
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
            let (game_to_display, is_grid_showing) = {
                let s = sc.borrow();
                let sid = s.selected_id.clone();
                let game = if ira_models::parse_db_id(&sid) == db_id {
                    s.games.iter().find(|g| g.grid_id() == sid).cloned()
                } else {
                    None
                };
                (game, s.selected_id.is_empty() && !s.content_unloaded)
            };
            if let Some(game) = game_to_display {
                display_game(&game, &sc);
            }
            refresh_settings_images_page(&sc, db_id, |s, game, win, pc| {
                build_image_manager_content_with_drafts(s, game, win, pc).upcast()
            });
            if is_grid_showing {
                show_grid_view(&sc);
            }
            // Update the mass match row to show unmatched state with manual search
            clear_children(&action_box_c);
            let label = gtk4::Label::new(Some("SGDB: unmatched"));
            label.add_css_class(CSS_DIM_LABEL);
            action_box_c.append(&label);
            let sgdb_btn = gtk4::Button::with_label("Search SGDB…");
            let sc2 = sc.clone();
            let gn = game_name_c.clone();
            let dlg = parent_c.clone();
            let ab = action_box_c.clone();
            sgdb_btn.connect_clicked(move |_| {
                let cb: Rc<dyn Fn()> = Rc::new({
                    let ab = ab.clone();
                    let name = gn.clone();
                    move || {
                        clear_children(&ab);
                        let label = gtk4::Label::new(Some(&format!("Matched to SGDB: {}", name)));
                        label.add_css_class(CSS_SUCCESS_LABEL);
                        ab.append(&label);
                    }
                });
                show_sgdb_search_dialog(&sc2, db_id, &gn, &dlg, Some(cb));
            });
            action_box_c.append(&sgdb_btn);
        });
        action_box.append(&undo_btn);
    } else {
        let label = gtk4::Label::new(Some("SGDB: not found"));
        label.add_css_class(CSS_DIM_LABEL);
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
                    label.add_css_class(CSS_SUCCESS_LABEL);
                    ab.append(&label);
                }
            });
            show_sgdb_search_dialog(&sc, did, &gn, &dlg, Some(cb));
        });
        action_box.append(&sgdb_btn);
    }
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
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
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
                        row.set_title(&super::helpers::esc(name));
                        row.set_subtitle(&format!("SGDB ID: {}", sgdb_id));
                        let match_btn = gtk4::Button::with_label("Match");
                        match_btn.add_css_class(CSS_SUGGESTED_ACTION);
                        match_btn.set_valign(gtk4::Align::Center);
                        let sgdb_id_c = sgdb_id.clone();
                        let state_c3 = state_c2.clone();
                        let dialog_c3 = dialog_c2.clone();
                        let on_match_cb = on_match_clone.clone();
                        match_btn.connect_clicked(move |_| {
                            if let Err(e) = ira_db::set_sgdb_id(&state_c3.borrow().db, db_id, &sgdb_id_c) {
                                eprintln!("Failed to set SGDB ID: {}", e);
                            }
                            if let Err(e) = ira_db::set_manual_unmatch(&state_c3.borrow().db, db_id, false) {
                                eprintln!("Failed to clear manual unmatch: {}", e);
                            }
                            if let Some(g) = state_c3.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                                g.sgdb_id = sgdb_id_c.clone();
                                g.manual_unmatch = false;
                            }
                            if let Some(ref sd) = state_c3.borrow().settings_data {
                                if sd.db_id == db_id {
                                    sd.pending_copies.borrow_mut().remove("__unmatch__");
                                }
                            }
                            refresh_settings_images_page(&state_c3, db_id, |s, game, win, pc| {
                                build_image_manager_content_with_drafts(s, game, win, pc).upcast()
                            });
                            let steam = state_c3.borrow().steam.clone();
                            let sgdb_id_d = sgdb_id_c.clone();
                            let sender = state_c3.borrow().sender.clone();
                            let db_id_for_msg = db_id;
                            let save_dir = state_c3.borrow().save_dir.clone();
                            let is_retro = state_c3.borrow().games.iter().find(|g| g.db_id == db_id).is_some_and(|g| g.kind == ira_models::GameKind::Retro);
                            std::thread::spawn(move || {
                                let _s = tracing::info_span!("sgdb_search_result_match", db_id = db_id_for_msg, sgdb_id = %sgdb_id_d).entered();
                                let (icon, hero, grid, logo, header) = if is_retro {
                                    let dir = ira_parser::retro_data_dir(&save_dir, db_id_for_msg);
                                    steam.ensure_sgdb_assets_in_dir(&dir, &sgdb_id_d)
                                } else {
                                    steam.ensure_sgdb_assets(&sgdb_id_d)
                                };
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
