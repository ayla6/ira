use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::css::*;
use super::game_display::display_game;
use super::grid_view::show_grid_view;
use super::helpers::{
    clamped, clamped_boxed_list, clear_children, esc, poll_channel, refresh_settings_images_page,
    status_row,
};
use super::image_manager::build_image_manager_content_with_drafts;
use super::state::SharedState;

pub(super) fn handle_unified_sgdb_result(
    state: &SharedState,
    action_box: &gtk4::Box,
    db_id: i64,
    game_name: &str,
    matched: Option<(String, String)>,
    parent_dialog: &adw::Dialog,
) {
    // Only update if the action box still shows a searching state
    // (don't overwrite a Steam match result)
    let has_result = action_box.last_child().is_some_and(|c| {
        c.downcast_ref::<gtk4::Label>().is_some_and(|l| {
            l.text().starts_with("Matched")
                || l.text().starts_with("Not found")
                || l.text().starts_with("Enter")
        })
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
        if let Some(g) = state
            .borrow_mut()
            .games
            .iter_mut()
            .find(|g| g.db_id == db_id)
        {
            g.sgdb_id = sgdb_id.clone();
        }
        let steam_dl = state.borrow().steam.clone();
        let sender = state.borrow().sender.clone();
        let sgdb_id_dl = sgdb_id;
        let save_dir = state.borrow().save_dir.clone();
        let game_for_dir = state
            .borrow()
            .games
            .iter()
            .find(|g| g.db_id == db_id)
            .cloned();
        std::thread::spawn(move || {
            let _s = tracing::info_span!("handle_unified_sgdb_result", db_id = db_id, sgdb_id = %sgdb_id_dl).entered();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let dir = match &game_for_dir {
                Some(g) => ira_parser::game_data_dir(&save_dir, g),
                None => ira_parser::sgdb_data_dir(&save_dir, &sgdb_id_dl),
            };
            let (icon, hero, grid, logo, header) =
                steam_dl.ensure_sgdb_assets_in_dir(&dir, &sgdb_id_dl);
            let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                db_id,
                sgdb_id: sgdb_id_dl,
                icon,
                hero,
                grid,
                logo,
                header,
            });
        });

        let label = gtk4::Label::new(Some(&crate::tr!("SGDB: {}").replacen(
            "{}",
            &matched_name,
            1,
        )));
        label.add_css_class(CSS_SUCCESS_LABEL);
        action_box.append(&label);

        let undo_btn = gtk4::Button::with_label(&crate::tr!("Undo SGDB"));
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
            refresh_settings_images_page(&sc, db_id, |s, game, win, pc, scache| {
                build_image_manager_content_with_drafts(s, game, win, pc, scache).upcast()
            });
            if is_grid_showing {
                show_grid_view(&sc);
            }
            // Update the mass match row to show unmatched state with manual search
            clear_children(&action_box_c);
            let label = gtk4::Label::new(Some(&crate::tr!("SGDB: unmatched")));
            label.add_css_class(CSS_DIM_LABEL);
            action_box_c.append(&label);
            let sgdb_btn = gtk4::Button::with_label(&crate::tr!("Search SGDB…"));
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
                        let label = gtk4::Label::new(Some(
                            &crate::tr!("Matched to SGDB: {}").replacen("{}", &name, 1),
                        ));
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
        let label = gtk4::Label::new(Some(&crate::tr!("SGDB: not found")));
        label.add_css_class(CSS_DIM_LABEL);
        action_box.append(&label);

        let sgdb_btn = gtk4::Button::with_label(&crate::tr!("Search SGDB…"));
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
                    let label = gtk4::Label::new(Some(
                        &crate::tr!("Matched to SGDB: {}").replacen("{}", &name, 1),
                    ));
                    label.add_css_class(CSS_SUCCESS_LABEL);
                    ab.append(&label);
                }
            });
            show_sgdb_search_dialog(&sc, did, &gn, &dlg, Some(cb));
        });
        action_box.append(&sgdb_btn);
    }
}

pub fn show_sgdb_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    parent: &impl IsA<gtk4::Widget>,
    on_match: Option<Rc<dyn Fn()>>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Match to SteamGridDB"));
    dialog.set_content_width(500);
    dialog.set_content_height(400);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Game name…")));
    entry.set_text(game_name);
    entry.set_hexpand(true);
    search_row.append(&entry);
    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_row.append(&search_btn);
    content.append(&clamped(&search_row, 500, (12, 12, 12, 12)));

    let (scrolled, list) = clamped_boxed_list(500);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let entry_c = entry.clone();
    let dialog_c = dialog.clone();
    let state_c = state.clone();
    let do_search = move || {
        let term = entry_c.text().trim().to_string();
        if term.is_empty() {
            return;
        }
        let steam = state_c.borrow().steam.clone();
        let (tx, rx) = mpsc::channel::<Vec<(String, String)>>();
        std::thread::spawn(move || {
            let _ = tx.send(steam.search_sgdb(&term));
        });
        let list_c2 = list.clone();
        let state_c2 = state_c.clone();
        let on_match_clone = on_match.clone();
        let dialog_c2 = dialog_c.clone();
        poll_channel(rx, move |results| {
            clear_children(&list_c2);
            if results.is_empty() {
                list_c2.append(&status_row(&crate::tr!("No results found")));
                return;
            }
            for (sgdb_id, name) in results {
                list_c2.append(&sgdb_result_row(
                    &state_c2,
                    db_id,
                    &sgdb_id,
                    &name,
                    &on_match_clone,
                    &dialog_c2,
                ));
            }
        });
    };

    let ds = do_search.clone();
    entry.connect_activate(move |_| ds());
    let ds2 = do_search.clone();
    search_btn.connect_clicked(move |_| ds2());

    dialog.present(Some(parent));
    do_search();
}

/// One SGDB hit with a suggested Match button persisting the match.
fn sgdb_result_row(
    state: &SharedState,
    db_id: i64,
    sgdb_id: &str,
    name: &str,
    on_match: &Option<Rc<dyn Fn()>>,
    dialog: &adw::Dialog,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(name));
    row.set_subtitle(&crate::tr!("SGDB ID: {}").replacen("{}", sgdb_id, 1));

    let match_btn = gtk4::Button::with_label(&crate::tr!("Match"));
    match_btn.add_css_class(CSS_SUGGESTED_ACTION);
    match_btn.set_valign(gtk4::Align::Center);
    let sc = state.clone();
    let sid = sgdb_id.to_string();
    let cb = on_match.clone();
    let dlg = dialog.clone();
    match_btn.connect_clicked(move |_| {
        apply_sgdb_match(&sc, db_id, &sid);
        dlg.close();
        if let Some(ref cb) = cb {
            cb();
        }
    });
    row.add_suffix(&match_btn);
    row
}

/// Persist an SGDB match: DB ids, in-memory state, asset download and a
/// refresh of the settings images page when it is showing this game.
fn apply_sgdb_match(state: &SharedState, db_id: i64, sgdb_id: &str) {
    if let Err(e) = ira_db::set_sgdb_id(&state.borrow().db, db_id, sgdb_id) {
        eprintln!("Failed to set SGDB ID: {}", e);
    }
    if let Err(e) = ira_db::set_manual_unmatch(&state.borrow().db, db_id, false) {
        eprintln!("Failed to clear manual unmatch: {}", e);
    }
    if let Some(g) = state
        .borrow_mut()
        .games
        .iter_mut()
        .find(|g| g.db_id == db_id)
    {
        g.sgdb_id = sgdb_id.to_string();
        g.manual_unmatch = false;
    }
    if let Some(ref sd) = state.borrow().settings_data {
        if sd.db_id == db_id {
            sd.pending_copies.borrow_mut().remove("__unmatch__");
        }
    }
    refresh_settings_images_page(state, db_id, |s, game, win, pc, scache| {
        build_image_manager_content_with_drafts(s, game, win, pc, scache).upcast()
    });

    let (steam, sender, save_dir, game_for_dir) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.save_dir.clone(),
            s.games.iter().find(|g| g.db_id == db_id).cloned(),
        )
    };
    let sgdb_id_d = sgdb_id.to_string();
    std::thread::spawn(move || {
        let _s =
            tracing::info_span!("sgdb_search_result_match", db_id = db_id, sgdb_id = %sgdb_id_d)
                .entered();
        let dir = match &game_for_dir {
            Some(g) => ira_parser::game_data_dir(&save_dir, g),
            None => ira_parser::sgdb_data_dir(&save_dir, &sgdb_id_d),
        };
        let (icon, hero, grid, logo, header) = steam.ensure_sgdb_assets_in_dir(&dir, &sgdb_id_d);
        let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
            db_id,
            sgdb_id: sgdb_id_d,
            icon,
            hero,
            grid,
            logo,
            header,
        });
    });
}
