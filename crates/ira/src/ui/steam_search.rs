use super::css::*;
use super::helpers::{clear_children, esc, poll_channel, status_row};
use super::state::SharedState;
use super::steam_search_dialog::{build_search_dialog, SearchDialogWidgets};
use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

type SelectCallback = Rc<dyn Fn(&str, &str)>;

/// One Steam store hit: escaped title, app-id subtitle and the caller's
/// labelled button writing the picked id back into the entry row.
fn steam_id_result_row(
    app_id: &str,
    name: &str,
    button_label: &str,
    app_id_row: &adw::EntryRow,
    on_select: &SelectCallback,
    dialog: &adw::Dialog,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(name));
    row.set_subtitle(&crate::tr!("App ID: {}").replacen("{}", app_id, 1));
    let pick_btn = gtk4::Button::with_label(button_label);
    pick_btn.add_css_class(CSS_SUGGESTED_ACTION);
    pick_btn.set_valign(gtk4::Align::Center);
    let sid = app_id.to_string();
    let matched_name = name.to_string();
    let on_select = on_select.clone();
    let row_update = app_id_row.clone();
    let dlg = dialog.clone();
    pick_btn.connect_clicked(move |_| {
        on_select(&sid, &matched_name);
        row_update.set_text(&sid);
        dlg.close();
    });
    row.add_suffix(&pick_btn);
    row
}

pub(super) fn show_steam_id_search_popup(
    state: &SharedState,
    game_name: &str,
    parent: &impl IsA<gtk4::Widget>,
    app_id_row: &adw::EntryRow,
    button_label: &str,
    on_select: SelectCallback,
) {
    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(
        &crate::tr!("Search Steam store"),
        500,
        400,
        500,
        game_name,
        Some(&crate::tr!("Game name…")),
    );

    let state_c = state.clone();
    let button_label_s = button_label.to_string();
    let entry_c = entry.clone();
    let dialog_c = dialog.clone();
    let row_c = app_id_row.clone();
    let on_select_c = on_select.clone();

    let do_search = move || {
        let term = entry_c.text().trim().to_string();
        if term.is_empty() {
            return;
        }
        let steam = state_c.borrow().steam.clone();
        let (tx, rx) = mpsc::channel::<Vec<(String, String)>>();
        std::thread::spawn(move || {
            let _ = tx.send(steam.search_steam_store(&term));
        });
        let list_c2 = list.clone();
        let row_c2 = row_c.clone();
        let on_select_c2 = on_select_c.clone();
        let dialog_c2 = dialog_c.clone();
        let btn_label = button_label_s.clone();
        poll_channel(rx, move |results| {
            clear_children(&list_c2);
            if results.is_empty() {
                list_c2.append(&status_row(&crate::tr!("No results found")));
                return;
            }
            for (app_id, name) in results {
                list_c2.append(&steam_id_result_row(
                    &app_id,
                    &name,
                    &btn_label,
                    &row_c2,
                    &on_select_c2,
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
