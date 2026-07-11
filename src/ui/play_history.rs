use gtk4::prelude::*;
use adw::prelude::*;
use crate::strings as S;
use super::state::SharedState;

fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let mins = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{} {} {} {}", hours, S::HOURS, mins, S::MINUTES)
    } else {
        format!("{} {}", mins, S::MINUTES)
    }
}

fn format_datetime(timestamp: i64) -> String {
    let secs = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
    let naive = chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    naive
}

pub fn show_play_history_dialog(state: &SharedState, game_id: i64) -> adw::Dialog {
    let game_name = state.borrow().games.iter()
        .find(|g| g.db_id == game_id)
        .map(|g| g.name.clone())
        .unwrap_or_default();

    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("{}: {}", S::SESSION_HISTORY_FOR, game_name));
    dialog.set_content_width(500);
    dialog.set_content_height(400);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    let total = crate::db::get_total_playtime_for_game(&state.borrow().db, game_id)
        .unwrap_or(0);
    let total_label = gtk4::Label::new(Some(&format!("{}: {}", S::TOTAL_PLAYTIME, format_duration(total))));
    total_label.add_css_class("heading");
    total_label.set_xalign(0.0);
    box_.append(&total_label);

    let sessions = crate::db::get_sessions_for_game(&state.borrow().db, game_id)
        .unwrap_or_default();

    if sessions.is_empty() {
        let empty_label = gtk4::Label::new(Some(S::NO_SESSIONS));
        empty_label.set_xalign(0.0);
        empty_label.set_opacity(0.6);
        box_.append(&empty_label);
    } else {
        let list = gtk4::ListBox::new();
        list.add_css_class("boxed-list");

        for session in &sessions {
            let row = gtk4::ListBoxRow::new();
            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            hbox.set_margin_start(8);
            hbox.set_margin_end(8);
            hbox.set_margin_top(6);
            hbox.set_margin_bottom(6);

            let date_label = gtk4::Label::new(Some(&format_datetime(session.started_at)));
            date_label.set_xalign(0.0);
            date_label.set_hexpand(true);
            hbox.append(&date_label);

            let dur_label = gtk4::Label::new(Some(&format_duration(session.duration_seconds)));
            dur_label.add_css_class("dim-label");
            hbox.append(&dur_label);

            row.set_child(Some(&hbox));
            list.append(&row);
        }

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_vexpand(true);
        box_.append(&scroll);
    }

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));
    dialog
}

pub fn show_daily_history_dialog(state: &SharedState) {
    let dialog = adw::Dialog::new();
    dialog.set_title(S::DAILY_HISTORY);
    dialog.set_content_width(600);
    dialog.set_content_height(400);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let from = now - 30 * 86400;

    let days = crate::db::get_playtime_by_day(&state.borrow().db, from, now)
        .unwrap_or_default();

    if days.is_empty() {
        let empty_label = gtk4::Label::new(Some(S::NO_SESSIONS));
        empty_label.set_xalign(0.0);
        empty_label.set_opacity(0.6);
        box_.append(&empty_label);
    } else {
        let list = gtk4::ListBox::new();
        list.add_css_class("boxed-list");

        for (date, total_secs) in &days {
            let row = gtk4::ListBoxRow::new();
            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            hbox.set_margin_start(8);
            hbox.set_margin_end(8);
            hbox.set_margin_top(6);
            hbox.set_margin_bottom(6);

            let date_label = gtk4::Label::new(Some(&date.format("%Y-%m-%d").to_string()));
            date_label.set_xalign(0.0);
            date_label.set_hexpand(true);
            hbox.append(&date_label);

            let dur_label = gtk4::Label::new(Some(&format_duration(*total_secs)));
            dur_label.add_css_class("dim-label");
            hbox.append(&dur_label);

            row.set_child(Some(&hbox));
            list.append(&row);
        }

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_vexpand(true);
        box_.append(&scroll);
    }

    toolbar_view.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&state.borrow().window));
}
