use super::css::*;
use super::helpers::{clamped, clamped_boxed_list, clear_children, esc, poll_channel, status_row};
use super::state::SharedState;
use adw::prelude::*;
use std::rc::Rc;
use std::sync::mpsc;

type SelectCallback = Rc<dyn Fn(&str, &str)>;

pub(super) fn show_steam_id_search_popup(
    state: &SharedState,
    game_name: &str,
    parent: &impl IsA<gtk4::Widget>,
    app_id_row: &adw::EntryRow,
    button_label: &str,
    on_select: SelectCallback,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Search Steam store"));
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
                let row = adw::ActionRow::new();
                row.set_title(&esc(&name));
                row.set_subtitle(&crate::tr!("App ID: {}").replacen("{}", &app_id, 1));
                let match_btn = gtk4::Button::with_label(&btn_label);
                match_btn.add_css_class(CSS_SUGGESTED_ACTION);
                match_btn.set_valign(gtk4::Align::Center);
                let sid = app_id.clone();
                let matched_name = name.clone();
                let on_select_c3 = on_select_c2.clone();
                let row_update = row_c2.clone();
                let dlg = dialog_c2.clone();
                match_btn.connect_clicked(move |_| {
                    on_select_c3(&sid, &matched_name);
                    row_update.set_text(&sid);
                    dlg.close();
                });
                row.add_suffix(&match_btn);
                list_c2.append(&row);
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
