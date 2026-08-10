use super::css::*;
use super::helpers::clear_children;
use super::state::SharedState;
use adw::prelude::*;
use std::rc::Rc;
use std::sync::Arc;

pub(super) fn show_steam_id_search_popup(
    state: &SharedState,
    game_name: &str,
    parent: &adw::Window,
    app_id_row: &adw::EntryRow,
    button_label: &str,
    on_select: Rc<dyn Fn(&str)>,
) {
    let dialog = adw::Window::new();
    dialog.set_default_width(500);
    dialog.set_default_height(400);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));
    dialog.set_title(Some("Search Steam store"));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
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
    list.add_css_class(CSS_BOXED_LIST);
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win_c = dialog.clone();
    close_btn.connect_clicked(move |_| win_c.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));

    let state_c = state.clone();
    let list_c = list.clone();
    let dialog_c = dialog.clone();
    let row_c = app_id_row.clone();
    let on_select_c = on_select.clone();

    let entry_s = entry.clone();
    let button_label_s = button_label.to_string();
    let do_search = move || {
        let term = entry_s.text().to_string();
        if term.is_empty() {
            return;
        }
        let steam = state_c.borrow().steam.clone();
        let results_shared = Arc::new(std::sync::Mutex::new(None::<Vec<(String, String)>>));
        let results_thread = results_shared.clone();
        std::thread::spawn(move || {
            let r = steam.search_steam_store(&term);
            *results_thread.lock().unwrap() = Some(r);
        });
        let results_poll = results_shared.clone();
        let list_c2 = list_c.clone();
        let dialog_c2 = dialog_c.clone();
        let row_c2 = row_c.clone();
        let on_select_c2 = on_select_c.clone();
        let btn_label = button_label_s.clone();
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
                    for (app_id, name) in &results {
                        let row = adw::ActionRow::new();
                        row.set_title(name);
                        row.set_subtitle(&format!("App ID: {}", app_id));
                        let match_btn = gtk4::Button::with_label(&btn_label);
                        match_btn.add_css_class(CSS_SUGGESTED_ACTION);
                        match_btn.set_valign(gtk4::Align::Center);
                        let sid = app_id.clone();
                        let on_select_c3 = on_select_c2.clone();
                        let row_update = row_c2.clone();
                        let dlg = dialog_c2.clone();
                        match_btn.connect_clicked(move |_| {
                            on_select_c3(&sid);
                            row_update.set_text(&sid);
                            dlg.close();
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

    dialog.present();
    do_search();
}
