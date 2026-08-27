use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

use super::css::*;
use super::game_display::display_game;
use super::grid_view::show_grid_view;
use super::helpers::{clear_children, poll_channel, refresh_settings_images_page, status_row};
use super::image_manager::build_image_manager_content_with_drafts;
use super::matching::{fetch_and_report_sgdb_assets, persist_sgdb_match};
use super::state::SharedState;
use super::steam_search_dialog::{build_search_dialog, match_result_row, SearchDialogWidgets};

/// A "Search SGDB…" button that reopens the SGDB search for this game and,
/// once a new match is accepted, repaints the action box with the green
/// "Matched to SGDB" label.
fn manual_sgdb_search_button(
    state: &SharedState,
    action_box: &gtk4::Box,
    parent_dialog: &adw::Dialog,
    db_id: i64,
    game_name: &str,
) -> gtk4::Button {
    let state = state.clone();
    let action_box = action_box.clone();
    let parent_dialog = parent_dialog.clone();
    let game_name = game_name.to_string();
    let btn = gtk4::Button::with_label(&crate::tr!("Search SGDB…"));
    btn.connect_clicked(move |_| {
        let cb: Rc<dyn Fn()> = Rc::new({
            let action_box = action_box.clone();
            let game_name = game_name.clone();
            move || {
                clear_children(&action_box);
                let label = gtk4::Label::new(Some(
                    &crate::tr!("Matched to SGDB: {}").replacen("{}", &game_name, 1),
                ));
                label.add_css_class(CSS_SUCCESS_LABEL);
                action_box.append(&label);
            }
        });
        show_sgdb_search_dialog(&state, db_id, &game_name, &parent_dialog, Some(cb));
    });
    btn
}

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
        persist_sgdb_match(&state.borrow().db, db_id, &sgdb_id, false);
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
        let save_dir = state.borrow().save_dir.clone();
        let game_for_dir = state
            .borrow()
            .games
            .iter()
            .find(|g| g.db_id == db_id)
            .cloned();
        std::thread::spawn(move || {
            let _s = tracing::info_span!("handle_unified_sgdb_result", db_id = db_id, sgdb_id = %sgdb_id).entered();
            std::thread::sleep(std::time::Duration::from_millis(100));
            fetch_and_report_sgdb_assets(
                &steam_dl,
                &sender,
                &save_dir,
                game_for_dir.as_ref(),
                db_id,
                sgdb_id,
            );
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
            action_box_c.append(&manual_sgdb_search_button(
                &sc,
                &action_box_c,
                &parent_c,
                db_id,
                &game_name_c,
            ));
        });
        action_box.append(&undo_btn);
    } else {
        let label = gtk4::Label::new(Some(&crate::tr!("SGDB: not found")));
        label.add_css_class(CSS_DIM_LABEL);
        action_box.append(&label);

        action_box.append(&manual_sgdb_search_button(
            state,
            action_box,
            parent_dialog,
            db_id,
            game_name,
        ));
    }
}

pub fn show_sgdb_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    parent: &impl IsA<gtk4::Widget>,
    on_match: Option<Rc<dyn Fn()>>,
) {
    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Match to SteamGridDB"),
        500,
        400,
        500,
        game_name,
        Some(&crate::tr!("Game name…")),
    );

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
    let sc = state.clone();
    let sid = sgdb_id.to_string();
    let cb = on_match.clone();
    let dlg = dialog.clone();
    match_result_row(
        name,
        &crate::tr!("SGDB ID: {}").replacen("{}", sgdb_id, 1),
        move || {
            apply_sgdb_match(&sc, db_id, &sid);
            dlg.close();
            if let Some(ref cb) = cb {
                cb();
            }
        },
    )
}

/// Persist an SGDB match: DB ids, in-memory state, asset download and a
/// refresh of the settings images page when it is showing this game.
fn apply_sgdb_match(state: &SharedState, db_id: i64, sgdb_id: &str) {
    persist_sgdb_match(&state.borrow().db, db_id, sgdb_id, true);
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
        fetch_and_report_sgdb_assets(
            &steam,
            &sender,
            &save_dir,
            game_for_dir.as_ref(),
            db_id,
            sgdb_id_d,
        );
    });
}
