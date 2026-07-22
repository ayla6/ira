use gtk4::prelude::*;
use adw::prelude::*;
use ira_api::SteamDataClient;
use std::rc::Rc;
use std::sync::Arc;

use super::state::SharedState;
use super::matching::{match_game_to_steam, match_game_to_sgdb};
use super::helpers::clear_children;
use super::add_game::prompt_for_steam_id;
use super::css::*;

type MatchCallback = Rc<dyn Fn(&str, &str)>;

#[derive(Clone, Copy)]
pub enum SearchSource {
    Steam,
    Sgdb,
}

pub(super) struct SearchResultsDialogParams<'a> {
    state: &'a SharedState,
    steam: Arc<SteamDataClient>,
    source_name: &'a str,
    game_name: &'a str,
    db_id: i64,
    source: SearchSource,
    on_match: MatchCallback,
    parent: &'a gtk4::Window,
}

pub(super) fn handle_steam_search_result(
    state: &SharedState,
    action_box: &gtk4::Box,
    steam: &Arc<SteamDataClient>,
    game_name: &str,
    db_id: i64,
    matched: Option<(String, String)>,
    parent_dialog: &adw::Window,
) {
    clear_children(action_box);

    if let Some((sid, matched_name)) = matched {
        match_game_to_steam(state, db_id, sid.clone(), game_name.to_string());

        let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sid)));
        label.add_css_class(CSS_SUCCESS_LABEL);
        action_box.append(&label);
    } else {
        let label = gtk4::Label::new(Some("Not found"));
        label.add_css_class(CSS_DIM_LABEL);
        action_box.append(&label);

        let ab = action_box.clone();
        let on_match: MatchCallback = Rc::new(move |sid, name| {
            clear_children(&ab);
            let text = if name.is_empty() {
                format!("Matched: {}", sid)
            } else {
                format!("Matched: {} ({})", name, sid)
            };
            let l = gtk4::Label::new(Some(&text));
            l.add_css_class(CSS_SUCCESS_LABEL);
            ab.append(&l);
        });

        let id_btn = gtk4::Button::with_label("Enter ID");
        let sc = state.clone();
        let name = game_name.to_string();
        let cb = on_match.clone();
        let did = db_id;
        id_btn.connect_clicked(move |_| {
            let sc2 = sc.clone();
            let name2 = name.clone();
            let cb2 = cb.clone();
            let body = format!("Enter the Steam app ID for \u{201C}{}\u{201D}:", name);
            prompt_for_steam_id(&sc, "Match to Steam", &body, move |app_id| {
                match_game_to_steam(&sc2, did, app_id.to_string(), name2.clone());
                cb2(app_id, "");
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
            show_search_results_dialog(SearchResultsDialogParams {
                state: &sc2, steam: steam2.clone(), source_name: "Steam",
                game_name: &name2, db_id, source: SearchSource::Steam,
                on_match: cb2.clone(), parent: pd.upcast_ref(),
            });
        });
        action_box.append(&steam_btn);

        let sgdb_btn = gtk4::Button::with_label("Search SGDB");
        let sc3 = state.clone();
        let name3 = game_name.to_string();
        let steam3 = steam.clone();
        let cb3 = on_match.clone();
        let pd = parent_dialog.clone();
        sgdb_btn.connect_clicked(move |_| {
            show_search_results_dialog(SearchResultsDialogParams {
                state: &sc3, steam: steam3.clone(), source_name: "SteamGridDB",
                game_name: &name3, db_id, source: SearchSource::Sgdb,
                on_match: cb3.clone(), parent: pd.upcast_ref(),
            });
        });
        action_box.append(&sgdb_btn);
    }
}

pub fn show_search_results_dialog(params: SearchResultsDialogParams) {
    let SearchResultsDialogParams { state, steam, source_name, game_name, db_id, source, on_match, parent } = params;

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
    placeholder.add_css_class(CSS_DIM_LABEL);
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
        searching.add_css_class(CSS_DIM_LABEL);
        results_clone.append(&searching);

        let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
        let rx = std::cell::RefCell::new(rx);

        let steam = steam_clone.clone();
        let src = source;
        std::thread::spawn(move || {
            let search_results = match src {
                SearchSource::Steam => steam.search_steam_store(&term),
                SearchSource::Sgdb => steam.search_sgdb(&term),
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
                    none.add_css_class(CSS_DIM_LABEL);
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
                    match_btn.add_css_class(CSS_SUGGESTED_ACTION);
                    let sc2 = sc.clone();
                    let name2 = name.clone();
                    let sid = app_id.clone();
                    let matched_name = result_name.clone();
                    let did = db_id;
                    let dialog_clone = dlg.clone();
                    let callback = cb.clone();
                    let src_type = source;
                    match_btn.connect_clicked(move |_| {
                        match src_type {
                            SearchSource::Steam => match_game_to_steam(&sc2, did, sid.clone(), name2.clone()),
                            SearchSource::Sgdb => match_game_to_sgdb(&sc2, did, sid.clone()),
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
