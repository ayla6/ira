use adw::prelude::{AdwDialogExt, AdwWindowExt, AlertDialogExt};
use gtk4::prelude::*;
use crate::Game;
use crate::strings as S;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::state::{PendingImage, SharedState};
use super::css::*;

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
    sidebar_area.add_css_class(CSS_SETTINGS_SIDEBAR);
    sidebar_area.set_size_request(200, -1);
    sidebar_area.set_vexpand(true);

    let sidebar = gtk4::ListBox::new();
    sidebar.add_css_class(CSS_NAVIGATION_SIDEBAR);
    sidebar.set_margin_top(6);
    sidebar.set_margin_bottom(6);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.set_vexpand(true);
    sidebar_scroll.set_child(Some(&sidebar));
    sidebar_area.append(&sidebar_scroll);

    let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    outer.append(&sidebar_area);
    outer.append(&sep);

    let content_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_area.set_hexpand(true);

    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_SETTINGS_HEADER);
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
    browse.add_css_class(CSS_FLAT);
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

/// Merge enrichment results into the existing in-memory game.
///
/// Starts from `existing` (which has the user's current edits) and applies only
/// the fields enrichment is responsible for (achievements, images, metadata).
/// All other fields are preserved from `existing` by default, so stale
/// enrichment data (read from DB before a save) can never revert user edits.
pub fn merge_game_enrichment(existing: &Game, enriched: &Game) -> Game {
    let mut result = existing.clone();

    // Achievements — enrichment loads these from disk.
    result.achievements = enriched.achievements.clone();
    if !existing.achievements.is_empty() {
        let existing_pcts: HashMap<String, f64> = existing
            .achievements
            .iter()
            .map(|a| (a.name.clone(), a.global_percent))
            .collect();
        for a in &mut result.achievements {
            if a.global_percent == 0.0 {
                if let Some(&pct) = existing_pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }
    result.earned_count = enriched.earned_count;
    result.total_count = enriched.total_count;

    // Images — apply only if enrichment found something.
    // Skip image paths from variant pseudo-games — their images are variant-specific
    // and must not overwrite the base game's images.
    if enriched.variant_id.is_none() {
        if !enriched.icon_path.is_empty() {
            result.icon_path = enriched.icon_path.clone();
        }
        if !enriched.hero_image_path.is_empty() {
            result.hero_image_path = enriched.hero_image_path.clone();
        }
        if !enriched.grid_path.is_empty() {
            result.grid_path = enriched.grid_path.clone();
        }
        if !enriched.header_path.is_empty() {
            result.header_path = enriched.header_path.clone();
        }
        if !enriched.logo_path.is_empty() {
            result.logo_path = enriched.logo_path.clone();
        }
    }

    // Name — apply only if existing is a placeholder.
    if existing.name.is_empty() || existing.name.starts_with("App ID:") {
        result.set_name(&enriched.name);
    }

    // Metadata — apply only if enrichment fetched something.
    if !enriched.release_date.is_empty() {
        result.release_date = enriched.release_date.clone();
    }
    if enriched.release_timestamp != 0 {
        result.release_timestamp = enriched.release_timestamp;
    }
    if enriched.metacritic_score >= 0 {
        result.metacritic_score = enriched.metacritic_score;
    }
    if enriched.steam_review_score >= 0 {
        result.steam_review_score = enriched.steam_review_score;
    }
    if enriched.steam_review_count != 0 {
        result.steam_review_count = enriched.steam_review_count;
    }

    result
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
    build_page: impl Fn(&SharedState, &Game, &adw::Window, Option<Rc<RefCell<HashMap<String, PendingImage>>>>) -> gtk4::Widget,
) {
    let sd = match state.borrow().settings_data.clone() {
        Some(d) => d,
        None => return,
    };
    if sd.db_id == db_id && sd.window.is_visible() {
        if let Some(old) = sd.stack.child_by_name("images") {
            sd.stack.remove(&old);
        }
        if let Some(game) = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned() {
            let new_page = build_page(state, &game, &sd.window, Some(sd.pending_copies.clone()));
            sd.stack.add_named(&new_page, Some("images"));
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

/// Spawn a terminal running bash, with the given env vars set.
/// Tries common terminal emulators in order since `-e` isn't universal.
pub fn spawn_terminal(env: &[(String, String)]) {
    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "bash"]),
        ("konsole", &["-e", "bash"]),
        ("xfce4-terminal", &["-e", "bash"]),
        ("mate-terminal", &["--", "bash"]),
        ("alacritty", &["-e", "bash"]),
        ("kitty", &["-e", "bash"]),
        ("foot", &["-e", "bash"]),
        ("wezterm", &["start", "--", "bash"]),
        ("tilix", &["-e", "bash"]),
        ("qterminal", &["-e", "bash"]),
        ("lxterminal", &["-e", "bash"]),
        ("terminator", &["-e", "bash"]),
        ("xterm", &["-e", "bash"]),
    ];

    if let Ok(term) = std::env::var("TERMINAL") {
        let mut cmd = std::process::Command::new(&term);
        cmd.arg("-e").arg("bash");
        for (k, v) in env { cmd.env(k, v); }
        if cmd.spawn().is_ok() { return; }
    }

    for (term, args) in terminals {
        let mut cmd = std::process::Command::new(term);
        cmd.args(*args);
        for (k, v) in env { cmd.env(k, v); }
        if cmd.spawn().is_ok() { return; }
    }
    eprintln!("No terminal emulator found. Set $TERMINAL or install gnome-terminal/konsole/xterm.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_models::LogoPosition;

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

    #[test]
    fn test_merge_game_enrichment_preserves_user_edits() {
        let existing = Game {
            shadps4_version: "/new/shadps4".to_string(),
            ra_core: "snes9x".to_string(),
            emulator_override: "/new/emu".to_string(),
            platform_id: "NEWPID".to_string(),
            trophy_source: ira_models::TrophySource::Ra,
            sgdb_id: "12345".to_string(),
            rom_path: "/new/rom.sfc".to_string(),
            sort_title: "My Sort".to_string(),
            logo_position: "center".to_string(),
            logo_size: 75,
            ..Default::default()
        };

        let enriched = Game {
            platform_id: "OLDPID".to_string(),
            trophy_source: ira_models::TrophySource::SteamNative,
            logo_position: LogoPosition::BottomLeft.to_string(),
            logo_size: 50,
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.shadps4_version, "/new/shadps4");
        assert_eq!(result.ra_core, "snes9x");
        assert_eq!(result.emulator_override, "/new/emu");
        assert_eq!(result.platform_id, "NEWPID");
        assert_eq!(result.trophy_source, ira_models::TrophySource::Ra);
        assert_eq!(result.sgdb_id, "12345");
        assert_eq!(result.rom_path, "/new/rom.sfc");
        assert_eq!(result.sort_title, "My Sort");
        assert_eq!(result.logo_position, "center");
        assert_eq!(result.logo_size, 75);
    }

    #[test]
    fn test_merge_game_enrichment_preserves_name_and_sort_title() {
        let existing = Game {
            name: "My Game".to_string(),
            sort_title: "Sort Key".to_string(),
            ..Default::default()
        };

        let enriched = Game {
            name: "App ID: 123".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.name, "My Game");
        assert_eq!(result.sort_title, "Sort Key");
    }

    #[test]
    fn test_merge_game_enrichment_applies_enrichment_achievements() {
        let existing = Game::default();

        let enriched = Game {
            earned_count: 5,
            total_count: 10,
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.earned_count, 5);
        assert_eq!(result.total_count, 10);
    }

    #[test]
    fn test_merge_game_enrichment_applies_enrichment_images() {
        let existing = Game::default();

        let enriched = Game {
            icon_path: "/new/icon.webp".to_string(),
            hero_image_path: "/new/hero.webp".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.icon_path, "/new/icon.webp");
        assert_eq!(result.hero_image_path, "/new/hero.webp");
    }

    #[test]
    fn test_merge_game_enrichment_applies_name_for_placeholder() {
        let existing = Game {
            name: "App ID: 123".to_string(),
            ..Default::default()
        };

        let enriched = Game {
            name: "Real Game Name".to_string(),
            ..Default::default()
        };

        let result = merge_game_enrichment(&existing, &enriched);

        assert_eq!(result.name, "Real Game Name");
    }
}
