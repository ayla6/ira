use adw::prelude::{AdwDialogExt, AdwWindowExt, AlertDialogExt};
use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use std::collections::HashMap;

use super::state::SharedState;

pub struct DialogLayout {
    pub window: adw::Window,
    pub sidebar: gtk4::ListBox,
    pub stack: gtk4::Stack,
    pub header: adw::HeaderBar,
    pub content_area: gtk4::Box,
    pub sidebar_area: gtk4::Box,
}

pub fn dialog_layout(parent: &impl IsA<gtk4::Window>) -> DialogLayout {
    let win = adw::Window::new();
    win.set_default_size(720, 540);
    win.set_modal(true);
    win.set_transient_for(Some(parent));

    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let sidebar_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_area.add_css_class("settings-sidebar");
    sidebar_area.set_size_request(200, -1);
    sidebar_area.set_vexpand(true);

    let sidebar = gtk4::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_margin_top(6);
    sidebar.set_margin_bottom(6);
    sidebar_area.append(&sidebar);

    let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    outer.append(&sidebar_area);
    outer.append(&sep);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.set_hexpand(true);

    let header = adw::HeaderBar::new();
    header.add_css_class("settings-header");
    content_area.append(&header);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_margin_start(16);
    stack.set_margin_end(16);
    stack.set_margin_top(16);
    stack.set_margin_bottom(16);

    content_area.append(&stack);
    outer.append(&content_area);
    win.set_content(Some(&outer));

    DialogLayout {
        window: win,
        sidebar,
        stack,
        header,
        content_area,
        sidebar_area,
    }
}

pub fn make_browse_button(
    parent: Option<&adw::Window>,
    title: &str,
    select_folder: bool,
    filter: Option<(&str, &[&str])>,
    on_select: impl Fn(&std::path::Path) + 'static,
) -> gtk4::Button {
    let browse = gtk4::Button::with_label("Browse…");
    browse.add_css_class("flat");
    browse.set_valign(gtk4::Align::Center);
    let parent = parent.cloned();
    let title = title.to_string();
    let filter = filter.map(|(name, mimes)| (name.to_string(), mimes.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
    let on_select = std::rc::Rc::new(on_select);
    browse.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&title);
        if let Some((name, mimes)) = &filter {
            let f = gtk4::FileFilter::new();
            f.set_name(Some(name));
            for mime in mimes {
                f.add_mime_type(mime);
            }
            f.add_pattern("*");
            dialog.set_default_filter(Some(&f));
        }
        let on_select = on_select.clone();
        let cb = move |result: Result<gio::File, glib::Error>| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    on_select(&path);
                }
            }
        };
        if select_folder {
            dialog.select_folder(parent.as_ref(), None::<&gio::Cancellable>, cb);
        } else {
            dialog.open(parent.as_ref(), None::<&gio::Cancellable>, cb);
        }
    });
    browse
}

pub fn merge_game_enrichment(existing: &Game, updated: &mut Game) {
    updated.kind = existing.kind;
    updated.game_path = existing.game_path.clone();

    if !existing.name.is_empty() && !existing.name.starts_with("App ID:") {
        updated.name = existing.name.clone();
    }

    updated.hidden = existing.hidden;
    updated.slug = existing.slug.clone();
    updated.playtime = existing.playtime;
    updated.last_played = existing.last_played;
    updated.manual_unmatch = existing.manual_unmatch;
    updated.sort_title = existing.sort_title.clone();
    if updated.icon_path.is_empty() {
        updated.icon_path = existing.icon_path.clone();
    }
    if updated.hero_image_path.is_empty() {
        updated.hero_image_path = existing.hero_image_path.clone();
    }
    if updated.grid_path.is_empty() {
        updated.grid_path = existing.grid_path.clone();
    }
    if updated.header_path.is_empty() {
        updated.header_path = existing.header_path.clone();
    }
    if updated.logo_path.is_empty() {
        updated.logo_path = existing.logo_path.clone();
    }
    if updated.logo_position.is_empty() {
        updated.logo_position = existing.logo_position.clone();
    }
    if updated.logo_size == 0 {
        updated.logo_size = existing.logo_size;
    }

    if updated.release_date.is_empty() {
        updated.release_date = existing.release_date.clone();
    }
    if updated.release_timestamp == 0 {
        updated.release_timestamp = existing.release_timestamp;
    }
    if updated.metacritic_score < 0 {
        updated.metacritic_score = existing.metacritic_score;
    }
    if updated.steam_review_score < 0 {
        updated.steam_review_score = existing.steam_review_score;
    }
    if updated.steam_review_count == 0 {
        updated.steam_review_count = existing.steam_review_count;
    }

    if !existing.achievements.is_empty() {
        let existing_pcts: HashMap<String, f64> = existing
            .achievements
            .iter()
            .map(|a| (a.name.clone(), a.global_percent))
            .collect();
        for a in &mut updated.achievements {
            if a.global_percent == 0.0 {
                if let Some(&pct) = existing_pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }
}

pub fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

pub fn open_file_location(file_path: &str) {
    let path = std::path::Path::new(file_path);
    let dir = path.parent().map(|p| p.to_string_lossy().to_string());
    let uri = format!("file://{}", file_path);
    let dbus_result = std::process::Command::new("dbus-send")
        .args([
            "--session", "--print-reply", "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems",
            &format!("array:string:{}", uri),
            "string:",
        ])
        .output();
    match dbus_result {
        Ok(o) if o.status.success() => return,
        _ => {}
    }
    if let Some(dir) = dir {
        open_folder(&dir);
    }
}

pub fn confirm_dialog(
    parent: &adw::ApplicationWindow,
    title: &str,
    body: &str,
    confirm_label: &str,
    appearance: adw::ResponseAppearance,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", appearance);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}

pub trait Clearable {
    fn clear_all_children(&self);
}

impl Clearable for gtk4::Box {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::ListBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

impl Clearable for gtk4::FlowBox {
    fn clear_all_children(&self) {
        while let Some(child) = self.first_child() {
            self.remove(&child);
        }
    }
}

pub fn clear_children(w: &impl Clearable) {
    w.clear_all_children();
}

pub fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

pub fn refresh_settings_images_page(
    state: &SharedState,
    db_id: i64,
    build_page: impl Fn(&SharedState, &Game, &adw::Window) -> gtk4::Widget,
) {
    if let Some((ref sw, ref ss, sdb_id)) = state.borrow().settings_data.clone() {
        if sdb_id == db_id && sw.is_visible() {
            if let Some(old) = ss.child_by_name("images") {
                ss.remove(&old);
            }
            if let Some(game) = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned() {
                let new_page = build_page(state, &game, sw);
                ss.add_named(&new_page, Some("images"));
            }
        }
    }
}

pub fn string_list_from(strings: &[String]) -> gtk4::StringList {
    let str_refs: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
    gtk4::StringList::new(&str_refs)
}

pub fn format_duration(seconds: i64) -> String {
    let total_mins = ((seconds.max(0) as f64) / 60.0).round() as u64;
    let h = total_mins / 60;
    let m = total_mins % 60;
    match (h, m) {
        (0, 0) => "0min".to_string(),
        (0, m) => format!("{}min", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{:02}min", h, m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "0min");
    }

    #[test]
    fn test_format_duration_negative() {
        assert_eq!(format_duration(-10), "0min");
    }

    #[test]
    fn test_format_duration_sub_minute_rounds_up() {
        assert_eq!(format_duration(30), "1min");
        assert_eq!(format_duration(45), "1min");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(300), "5min");
        assert_eq!(format_duration(600), "10min");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(7200), "2h");
    }

    #[test]
    fn test_format_duration_hours_minutes() {
        assert_eq!(format_duration(7500), "2h05min");
        assert_eq!(format_duration(9000), "2h30min");
    }

    #[test]
    fn test_format_duration_rounds_near_hour() {
        assert_eq!(format_duration(3570), "1h");
        assert_eq!(format_duration(3630), "1h01min");
    }
}
