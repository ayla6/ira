use adw::prelude::*;
use ira_api::SteamDataClient;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;

use super::add_game::prompt_for_steam_id;
use super::css::*;
use super::helpers::{clamped, clamped_boxed_list, clear_children, esc, poll_channel, status_row};
use super::matching::{match_game_to_sgdb, match_game_to_steam};
use super::state::SharedState;

type MatchCallback = Rc<dyn Fn(&str, &str)>;

/// A prefilled, hexpanding `gtk::SearchEntry` beside a suggested-action
/// "Search" button — the input strip every search dialog opens with.
/// Returns `(row, entry, search_button)` so callers can wire signals.
pub(crate) fn build_search_row(
    entry_text: &str,
    placeholder: Option<&str>,
) -> (gtk4::Box, gtk4::SearchEntry, gtk4::Button) {
    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let entry = gtk4::SearchEntry::new();
    entry.set_hexpand(true);
    if let Some(placeholder) = placeholder {
        entry.set_placeholder_text(Some(placeholder));
    }
    entry.set_text(entry_text);
    search_row.append(&entry);
    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_row.append(&search_btn);
    (search_row, entry, search_btn)
}

/// The widgets a caller needs from [`build_search_dialog`] to wire its
/// search behavior; toolbar and content column stay internal.
pub(crate) struct SearchDialogWidgets {
    pub dialog: adw::Dialog,
    pub entry: gtk4::SearchEntry,
    pub search_btn: gtk4::Button,
    pub list: gtk4::ListBox,
}

/// Skeleton shared by every modal search dialog: a titled `adw::Dialog`
/// carrying a HeaderBar toolbar, a clamped search row and a clamped boxed
/// result list. Signal wiring and result population stay at the call sites.
pub(crate) fn build_search_dialog(
    title: &str,
    width: i32,
    height: i32,
    max_width: i32,
    entry_text: &str,
    placeholder: Option<&str>,
) -> SearchDialogWidgets {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(width);
    dialog.set_content_height(height);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let (search_row, entry, search_btn) = build_search_row(entry_text, placeholder);
    content.append(&clamped(&search_row, max_width, (12, 12, 12, 12)));

    let (scrolled, list) = clamped_boxed_list(max_width);
    content.append(&scrolled);

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    }
}

#[derive(Clone, Copy)]
pub enum SearchSource {
    Steam,
    Sgdb,
}

pub(super) struct SearchResultsDialogParams<'a, P: IsA<gtk4::Widget>> {
    pub(super) state: &'a SharedState,
    pub(super) steam: Arc<SteamDataClient>,
    pub(super) source_name: &'a str,
    pub(super) game_name: &'a str,
    pub(super) db_id: i64,
    pub(super) source: SearchSource,
    pub(super) on_match: MatchCallback,
    /// Widget the dialog is presented over (main window or another dialog).
    pub(super) parent: &'a P,
    /// When false, the Match button only invokes `on_match` and does not
    /// persist a DB match (used by the auto-add flow where no game exists yet).
    pub(super) match_in_db: bool,
}

pub(super) fn handle_steam_search_result(
    state: &SharedState,
    action_box: &gtk4::Box,
    steam: &Arc<SteamDataClient>,
    game_name: &str,
    db_id: i64,
    matched: Option<(String, String)>,
    parent_dialog: &adw::Dialog,
) {
    clear_children(action_box);

    if let Some((sid, matched_name)) = matched {
        match_game_to_steam(state, db_id, sid.clone(), game_name.to_string());

        let label = gtk4::Label::new(Some(
            &crate::tr!("Matched: {} ({})")
                .replacen("{}", &matched_name, 1)
                .replacen("{}", &sid, 1),
        ));
        label.add_css_class(CSS_SUCCESS_LABEL);
        action_box.append(&label);
    } else {
        let label = gtk4::Label::new(Some(&crate::tr!("Not found")));
        label.add_css_class(CSS_DIM_LABEL);
        action_box.append(&label);

        let ab = action_box.clone();
        let on_match: MatchCallback = Rc::new(move |sid, name| {
            clear_children(&ab);
            let text = if name.is_empty() {
                crate::tr!("Matched: {}").replacen("{}", sid, 1)
            } else {
                crate::tr!("Matched: {} ({})")
                    .replacen("{}", name, 1)
                    .replacen("{}", sid, 1)
            };
            let l = gtk4::Label::new(Some(&text));
            l.add_css_class(CSS_SUCCESS_LABEL);
            ab.append(&l);
        });

        let id_btn = gtk4::Button::with_label(&crate::tr!("Enter ID"));
        let sc = state.clone();
        let name = game_name.to_string();
        let cb = on_match.clone();
        let did = db_id;
        id_btn.connect_clicked(move |_| {
            let sc2 = sc.clone();
            let name2 = name.clone();
            let cb2 = cb.clone();
            let body = crate::tr!("Enter the Steam app ID for \u{201C}{}\u{201D}:")
                .replacen("{}", &name, 1);
            prompt_for_steam_id(&sc, &crate::tr!("Match to Steam"), &body, move |app_id| {
                match_game_to_steam(&sc2, did, app_id.to_string(), name2.clone());
                cb2(app_id, "");
            });
        });
        action_box.append(&id_btn);

        let steam_btn = gtk4::Button::with_label(&crate::tr!("Search Steam"));
        let sc2 = state.clone();
        let name2 = game_name.to_string();
        let steam2 = steam.clone();
        let cb2 = on_match.clone();
        let pd = parent_dialog.clone();
        steam_btn.connect_clicked(move |_| {
            show_search_results_dialog(SearchResultsDialogParams {
                state: &sc2,
                steam: steam2.clone(),
                source_name: &crate::tr!("Steam"),
                game_name: &name2,
                db_id,
                source: SearchSource::Steam,
                on_match: cb2.clone(),
                parent: &pd,
                match_in_db: true,
            });
        });
        action_box.append(&steam_btn);

        let sgdb_btn = gtk4::Button::with_label(&crate::tr!("Search SGDB"));
        let sc3 = state.clone();
        let name3 = game_name.to_string();
        let steam3 = steam.clone();
        let cb3 = on_match.clone();
        let pd = parent_dialog.clone();
        sgdb_btn.connect_clicked(move |_| {
            show_search_results_dialog(SearchResultsDialogParams {
                state: &sc3,
                steam: steam3.clone(),
                source_name: &crate::tr!("SteamGridDB"),
                game_name: &name3,
                db_id,
                source: SearchSource::Sgdb,
                on_match: cb3.clone(),
                parent: &pd,
                match_in_db: true,
            });
        });
        action_box.append(&sgdb_btn);
    }
}

pub fn show_search_results_dialog<P: IsA<gtk4::Widget>>(params: SearchResultsDialogParams<'_, P>) {
    let SearchResultsDialogParams {
        state,
        steam,
        source_name,
        game_name,
        db_id,
        source,
        on_match,
        parent,
        match_in_db,
    } = params;

    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Search {}").replacen("{}", source_name, 1),
        450,
        400,
        400,
        game_name,
        None,
    );

    let ctx = MatchContext {
        state: state.clone(),
        db_id,
        source,
        match_in_db,
        on_match,
        dialog: dialog.clone(),
    };

    let run_search = {
        let entry = entry.clone();
        let list = list.clone();
        let steam = steam.clone();
        let ctx = ctx.clone();
        move || {
            let term = entry.text().trim().to_string();
            if term.is_empty() {
                return;
            }
            clear_children(&list);
            list.append(&status_row(&crate::tr!("Searching...")));

            let (tx, rx) = mpsc::channel::<Vec<(String, String)>>();
            let steam = steam.clone();
            std::thread::spawn(move || {
                let results = match source {
                    SearchSource::Steam => steam.search_steam_store(&term),
                    SearchSource::Sgdb => steam.search_sgdb(&term),
                };
                let _ = tx.send(results);
            });

            let list = list.clone();
            let ctx = ctx.clone();
            poll_channel(rx, move |results| {
                clear_children(&list);
                if results.is_empty() {
                    list.append(&status_row(&crate::tr!("No results found")));
                    return;
                }
                for (app_id, result_name) in results {
                    list.append(&search_result_row(&ctx, &app_id, &result_name));
                }
            });
        }
    };

    let rs = run_search.clone();
    entry.connect_activate(move |_| rs());
    let rs = run_search.clone();
    search_btn.connect_clicked(move |_| rs());

    dialog.present(Some(parent));
    run_search();
}

/// Everything a result row's Match button needs to persist and report a match.
#[derive(Clone)]
struct MatchContext {
    state: SharedState,
    db_id: i64,
    source: SearchSource,
    match_in_db: bool,
    on_match: MatchCallback,
    dialog: adw::Dialog,
}

/// One search hit rendered as an ActionRow: escaped title, id subtitle and a
/// suggested Match button (centered) that runs `on_match`. Persisting,
/// closing and reporting stay with the caller's closure.
pub(crate) fn match_result_row(
    title: &str,
    subtitle: &str,
    on_match: impl Fn() + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(title));
    row.set_subtitle(subtitle);

    let match_btn = gtk4::Button::with_label(&crate::tr!("Match"));
    match_btn.add_css_class(CSS_SUGGESTED_ACTION);
    match_btn.set_valign(gtk4::Align::Center);
    match_btn.connect_clicked(move |_| on_match());
    row.add_suffix(&match_btn);
    row
}

/// One store hit: name + app id with a suggested Match button.
fn search_result_row(ctx: &MatchContext, app_id: &str, result_name: &str) -> adw::ActionRow {
    let ctx = ctx.clone();
    let sid = app_id.to_string();
    let name = result_name.to_string();
    match_result_row(
        result_name,
        &crate::tr!("App ID: {}").replacen("{}", app_id, 1),
        move || {
            if ctx.match_in_db {
                match ctx.source {
                    SearchSource::Steam => {
                        match_game_to_steam(&ctx.state, ctx.db_id, sid.clone(), name.clone())
                    }
                    SearchSource::Sgdb => match_game_to_sgdb(&ctx.state, ctx.db_id, sid.clone()),
                }
            }
            (ctx.on_match)(&sid, &name);
            ctx.dialog.close();
        },
    )
}
