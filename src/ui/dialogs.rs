use gtk4::prelude::*;
use adw::prelude::*;
use crate::config::Config;
use crate::api::SteamClient;
use crate::api::types::SgdbAsset;
use crate::Game;
use crate::strings as S;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use super::state::{SharedState, SAVE_DIR};
use super::sidebar::rebuild_sidebar;
use super::game_display::display_game;
use super::message_handler::apply_game_update;
use super::helpers::confirm_dialog;
use super::mass_match_dialog::show_sgdb_search_dialog;

pub fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamClient>,
    state: &SharedState,
) {
    let win = adw::Window::new();
    win.set_default_width(640);
    win.set_default_height(480);
    win.set_modal(true);
    win.set_transient_for(Some(parent));
    win.set_deletable(false);

    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let sidebar_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_area.add_css_class("settings-sidebar");
    sidebar_area.set_size_request(180, -1);
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

    fn settings_sidebar_row(icon: &str, label: &str) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        let icon = gtk4::Image::from_icon_name(icon);
        let text = gtk4::Label::new(Some(label));
        text.set_halign(gtk4::Align::Start);
        hbox.append(&icon);
        hbox.append(&text);
        row.set_child(Some(&hbox));
        row.set_size_request(-1, 36);
        row
    }

    let general_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title(S::LIVE_UPDATES);

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title(S::NOTIFY_ON_UNLOCKS);
    notif_row.set_subtitle(S::NOTIFY_SUBTITLE);
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title(S::CLOSE_TO_BG_TITLE);
    bg_row.set_subtitle(S::CLOSE_TO_BG_SUBTITLE);
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);

    general_page.append(&notif_group);

    let hidden_group = adw::PreferencesGroup::new();
    let hidden_row = adw::SwitchRow::new();
    hidden_row.set_title(S::SHOW_HIDDEN_GAMES);
    hidden_row.set_active(cfg.show_hidden_games);
    hidden_group.add(&hidden_row);
    general_page.append(&hidden_group);

    let grid_group = adw::PreferencesGroup::new();
    let grid_adj = gtk4::Adjustment::new(cfg.grid_cover_width as f64, 120.0, 320.0, 10.0, 20.0, 0.0);
    let grid_spin = gtk4::SpinButton::new(Some(&grid_adj), 1.0, 0);
    let grid_row = adw::ActionRow::new();
    grid_row.set_title(S::COVER_SIZE);
    grid_row.add_suffix(&grid_spin);
    grid_group.add(&grid_row);
    general_page.append(&grid_group);

    sidebar.append(&settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let api_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let key_group = adw::PreferencesGroup::new();
    key_group.set_title(S::API_KEYS);

    let steam_entry = adw::EntryRow::new();
    steam_entry.set_title(S::STEAM_WEB_API_KEY);
    steam_entry.set_text(&cfg.steam_api_key);
    steam_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_group.add(&steam_entry);

    let sgdb_entry = adw::EntryRow::new();
    sgdb_entry.set_title(S::STEAMGRIDDB_KEY);
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    sgdb_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_group.add(&sgdb_entry);

    api_page.append(&key_group);

    sidebar.append(&settings_sidebar_row("dialog-password-symbolic", "API Keys"));
    stack.add_named(&api_page, Some("api"));

    let ps4_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let ps4_enable_group = adw::PreferencesGroup::new();
    let ps4_enable_row = adw::SwitchRow::new();
    ps4_enable_row.set_title("Enable shadPS4 integration");
    ps4_enable_row.set_subtitle("Scan shadPS4 install directories for PS4 games");
    ps4_enable_row.set_active(cfg.shadps4_enabled);
    ps4_enable_group.add(&ps4_enable_row);
    ps4_page.append(&ps4_enable_group);

    let ps4_exe_group = adw::PreferencesGroup::new();
    ps4_exe_group.set_title("Emulator");

    let ps4_exe_row = adw::EntryRow::new();
    ps4_exe_row.set_title("shadPS4 executable path");
    ps4_exe_row.set_text(&cfg.shadps4_executable);

    let shadps4_versions = crate::platforms::ps4::read_shadps4_versions();
    let detected_path = crate::platforms::ps4::detect_shadps4_version_path();

    if !shadps4_versions.is_empty() {
        let trunc = |s: &str, max: usize| -> String {
            if s.len() > max {
                format!("{}…", &s[..max.saturating_sub(1)])
            } else {
                s.to_string()
            }
        };
        let version_strings: Vec<String> = shadps4_versions.iter()
            .map(|v| {
                let extra = if !v.date.is_empty() { v.date.clone() } else { v.codename.clone() };
                format!("{}  ({})", v.name, trunc(&extra, 14))
            })
            .collect();
        let version_model = gtk4::StringList::new(&version_strings.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let version_dropdown = gtk4::DropDown::new(Some(version_model), None::<&gtk4::PropertyExpression>);

        let current_exe = if cfg.shadps4_executable.is_empty() {
            detected_path.clone().unwrap_or_default()
        } else {
            cfg.shadps4_executable.clone()
        };
        let mut selected_idx: u32 = 0;
        if !current_exe.is_empty() {
            for (i, v) in shadps4_versions.iter().enumerate() {
                let v_path = v.path.trim_matches('"');
                if v_path == current_exe {
                    selected_idx = i as u32;
                    break;
                }
            }
        }
        version_dropdown.set_selected(selected_idx);

        let ps4_exe_row_c = ps4_exe_row.clone();
        version_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected();
            if let Some(versions) = crate::platforms::ps4::read_shadps4_versions().into_iter().nth(idx as usize) {
                let path = versions.path.trim_matches('"').to_string();
                ps4_exe_row_c.set_text(&path);
            }
        });

        let version_row = adw::ActionRow::new();
        version_row.set_title("Version");
        version_row.set_subtitle("Select a shadPS4 version");
        version_dropdown.set_valign(gtk4::Align::Center);
        version_row.add_suffix(&version_dropdown);
        ps4_exe_group.add(&version_row);
    }

    if let Some(ref detected) = detected_path {
        let auto_btn = gtk4::Button::with_label("Auto-detect");
        auto_btn.add_css_class("flat");
        auto_btn.set_valign(gtk4::Align::Center);
        let exe_row = ps4_exe_row.clone();
        let detected_path = detected.clone();
        auto_btn.connect_clicked(move |_| {
            exe_row.set_text(&detected_path);
        });
        ps4_exe_row.add_suffix(&auto_btn);
    }

    let ps4_exe_browse = gtk4::Button::with_label("Browse…");
    ps4_exe_browse.add_css_class("flat");
    ps4_exe_browse.set_valign(gtk4::Align::Center);
    {
        let ps4_exe_row = ps4_exe_row.clone();
        let parent = win.clone();
        ps4_exe_browse.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select shadPS4 executable");
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Executable"));
            filter.add_mime_type("application/x-executable");
            filter.add_pattern("*");
            dialog.set_default_filter(Some(&filter));
            let row = ps4_exe_row.clone();
            dialog.open(Some(&parent), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        row.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }
    ps4_exe_row.add_suffix(&ps4_exe_browse);
    ps4_exe_group.add(&ps4_exe_row);
    ps4_page.append(&ps4_exe_group);

    let ps4_dirs_group = adw::PreferencesGroup::new();
    ps4_dirs_group.set_title("Install directories");
    ps4_dirs_group.set_description(Some("Managed by shadPS4"));
    let install_dirs = crate::platforms::ps4::read_install_dirs();
    if install_dirs.is_empty() {
        let empty_row = adw::ActionRow::new();
        empty_row.set_title("No install directories configured");
        empty_row.set_sensitive(false);
        ps4_dirs_group.add(&empty_row);
    } else {
        for dir in &install_dirs {
            let dir_row = adw::ActionRow::new();
            dir_row.set_title(&dir.display().to_string());
            dir_row.set_sensitive(false);
            ps4_dirs_group.add(&dir_row);
        }
    }
    ps4_page.append(&ps4_dirs_group);

    sidebar.append(&settings_sidebar_row("applications-games-symbolic", "shadPS4"));
    stack.add_named(&ps4_page, Some("ps4"));

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "API Keys" => "api",
                                "shadPS4" => "ps4",
                                _ => "general",
                            };
                            stack_clone.set_visible_child_name(page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    content_area.append(&stack);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(S::CANCEL);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");

    let state_clone = state.clone();
    let win_clone = win.clone();
    let steam_clone = steam.clone();
    save_btn.connect_clicked(move |_| {
        let mut s = state_clone.borrow_mut();
        s.cfg.steam_api_key = steam_entry.text().to_string();
        s.cfg.steam_griddb_api_key = sgdb_entry.text().to_string();
        s.cfg.notifications_enabled = notif_row.is_active();
        s.cfg.close_to_background = bg_row.is_active();
        s.cfg.show_hidden_games = hidden_row.is_active();
        s.cfg.grid_cover_width = grid_spin.value() as i32;
        s.cfg.shadps4_enabled = ps4_enable_row.is_active();
        s.cfg.shadps4_executable = ps4_exe_row.text().to_string();

        steam_clone.update_keys(&s.cfg.steam_api_key, &s.cfg.steam_griddb_api_key);

        let cfg = s.cfg.clone();
        drop(s);

        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        win_clone.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);
    outer.append(&content_area);

    win.set_content(Some(&outer));
    win.present();
}

pub fn show_game_settings_dialog(state: &SharedState, game: &Game) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_default_width(720);
    win.set_default_height(540);
    win.set_transient_for(Some(&parent));
    win.set_modal(true);
    win.set_deletable(false);

    let app_details = crate::parser::read_app_details(SAVE_DIR, &game.app_id);

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
    header.set_title_widget(Some(&gtk4::Label::new(Some(&game.name))));
    content_area.append(&header);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_margin_start(16);
    stack.set_margin_end(16);
    stack.set_margin_top(16);
    stack.set_margin_bottom(16);
    stack.set_hexpand(true);

    fn sidebar_row(icon_name: &str, label: &str) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        let icon = gtk4::Image::from_icon_name(icon_name);
        let text = gtk4::Label::new(Some(label));
        text.set_halign(gtk4::Align::Start);
        hbox.append(&icon);
        hbox.append(&text);
        row.set_child(Some(&hbox));
        row.set_size_request(-1, 36);
        row
    }

    let general_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let title_entry = adw::EntryRow::new();
    title_entry.set_title(S::GAME_TITLE);
    title_entry.set_text(&game.name);
    let general_group = adw::PreferencesGroup::new();
    general_group.set_title("Identity");
    general_group.add(&title_entry);
    general_page.append(&general_group);

    let sort_entry = adw::EntryRow::new();
    sort_entry.set_title("Sort title");
    sort_entry.set_text(&game.sort_title);
    let sort_group = adw::PreferencesGroup::new();
    sort_group.add(&sort_entry);
    general_page.append(&sort_group);

    let pending_version: Rc<RefCell<Option<String>>> = Default::default();
    if game.kind == "ps4" {
        let shadps4_versions = crate::platforms::ps4::read_shadps4_versions();
        if !shadps4_versions.is_empty() {
            let version_group = adw::PreferencesGroup::new();
            version_group.set_title("shadPS4 Version");

            let trunc = |s: &str, max: usize| -> String {
                if s.len() > max {
                    format!("{}…", &s[..max.saturating_sub(1)])
                } else {
                    s.to_string()
                }
            };
            let version_strings: Vec<String> = std::iter::once("Follow global".to_string())
                .chain(shadps4_versions.iter().map(|v| {
                    let extra = if !v.date.is_empty() { v.date.clone() } else { v.codename.clone() };
                    format!("{}  ({})", v.name, trunc(&extra, 14))
                }))
                .collect();
            let str_refs: Vec<&str> = version_strings.iter().map(|s| s.as_str()).collect();
            let version_model = gtk4::StringList::new(&str_refs);
            let version_dropdown = gtk4::DropDown::new(Some(version_model), None::<&gtk4::PropertyExpression>);

            let current_ver = if game.shadps4_version.is_empty() {
                "Follow global".to_string()
            } else {
                let mut found = "".to_string();
                for v in &shadps4_versions {
                    let v_path = v.path.trim_matches('"');
                    if v_path == game.shadps4_version {
                        let extra = if !v.date.is_empty() { v.date.clone() } else { v.codename.clone() };
                        found = format!("{}  ({})", v.name, trunc(&extra, 14));
                        break;
                    }
                }
                if found.is_empty() { "Follow global".to_string() } else { found }
            };
            let mut selected_idx = 0u32;
            for (i, s) in version_strings.iter().enumerate() {
                if s == &current_ver {
                    selected_idx = i as u32;
                    break;
                }
            }
            version_dropdown.set_selected(selected_idx);

            let pending_version_c = pending_version.clone();
            version_dropdown.connect_selected_notify(move |dd| {
                let idx = dd.selected();
                let path = if idx == 0 {
                    String::new()
                } else {
                    match crate::platforms::ps4::read_shadps4_versions().into_iter().nth((idx - 1) as usize) {
                        Some(v) => v.path.trim_matches('"').to_string(),
                        None => return,
                    }
                };
                *pending_version_c.borrow_mut() = Some(path);
            });

            let version_row = adw::ActionRow::new();
            version_row.set_title("Version");
            version_row.set_subtitle("Override the emulator version for this game");
            version_dropdown.set_valign(gtk4::Align::Center);
            version_row.add_suffix(&version_dropdown);
            version_group.add(&version_row);
            general_page.append(&version_group);
        }
    }

    if game.lutris_id != 0 && !game.app_id.is_empty() && game.kind != "ps4" {
        let unmatch_group = adw::PreferencesGroup::new();
        let unmatch_btn = gtk4::Button::with_label("Unmatch game");
        unmatch_btn.add_css_class("destructive-action");
        unmatch_btn.set_halign(gtk4::Align::Start);
        let sc = state.clone();
        let gc = game.clone();
        let win_clone = win.clone();
        unmatch_btn.connect_clicked(move |_| {
            let parent = sc.borrow().window.clone();
            let sc2 = sc.clone();
            let lutris_id = gc.lutris_id;
            let name = gc.name.clone();
            let win2 = win_clone.clone();
            confirm_dialog(
                &parent,
                "Unmatch game?",
                &format!("Unmatch \u{201C}{}\u{201D} from its trophy source? The game will remain in the list but without trophies.", name),
                "Unmatch",
                adw::ResponseAppearance::Destructive,
                move || {
                    if let Err(e) = crate::db::unmatch_game(&sc2.borrow().db, lutris_id) {
                        eprintln!("Failed to unmatch game: {}", e);
                    }
                    if let Some(g) = sc2.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                        g.app_id.clear();
                        g.kind.clear();
                        g.platform_id.clear();
                        g.achievements.clear();
                        g.earned_count = 0;
                        g.total_count = 0;
                        g.icon_path.clear();
                        g.hero_image_path.clear();
                        g.grid_path.clear();
                        g.header_path.clear();
                        g.logo_path.clear();
                        g.manual_unmatch = true;
                        if !g.lutris_name.is_empty() {
                            g.name = g.lutris_name.clone();
                        }
                    }
                    rebuild_sidebar(&sc2);
                    win2.close();
                },
            );
        });
        unmatch_group.add(&unmatch_btn);
        general_page.append(&unmatch_group);
    }

    sidebar.append(&sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let logo_positions = ["top-left", "top-center", "top-right", "center-left", "center", "center-right", "bottom-left", "bottom-center", "bottom-right"];
    let logo_controls: Option<(Rc<RefCell<String>>, gtk4::Adjustment)> = if !game.logo_path.is_empty() {
        let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

        let selected_pos: Rc<RefCell<String>> = Rc::new(RefCell::new(game.logo_position.clone()));

        let size_pct = game.logo_size.clamp(5, 100);
        let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 1.0, 5.0, 0.0);

        let preview_overlay = gtk4::Overlay::new();
        preview_overlay.set_height_request(220);
        preview_overlay.set_overflow(gtk4::Overflow::Hidden);

        let hero_pic = gtk4::Picture::new();
        if let Some(t) = crate::images::texture_for(&game.hero_image_path) {
            hero_pic.set_paintable(Some(&t));
        }
        hero_pic.set_content_fit(gtk4::ContentFit::Cover);
        hero_pic.set_halign(gtk4::Align::Fill);
        hero_pic.set_valign(gtk4::Align::Fill);
        preview_overlay.set_child(Some(&hero_pic));

        let preview_draw = gtk4::DrawingArea::new();
        preview_draw.set_halign(gtk4::Align::Fill);
        preview_draw.set_valign(gtk4::Align::Fill);
        preview_draw.set_hexpand(true);
        preview_draw.set_vexpand(true);

        if let Some(ref pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path).ok() {
            let pb_w = pixbuf.width() as f64;
            let pb_h = pixbuf.height() as f64;
            let pixbuf_clone = pixbuf.clone();
            let pos_for_draw = selected_pos.clone();
            let adj_for_draw = size_adj.clone();

            preview_draw.set_draw_func(move |_area, cr, area_w, area_h| {
                let w = area_w as f64;
                let h = area_h as f64;
                if w <= 0.0 || h <= 0.0 { return; }
                let pct = adj_for_draw.value() as i32;
                let (sw, sh) = super::game_display::logo_scaled_dims(w, h, pb_w, pb_h, pct);
                let lw = sw as f64;
                let lh = sh as f64;
                let pos = pos_for_draw.borrow().clone();
                let (halign, valign) = super::game_display::logo_position_align(&pos);
                let x = match halign {
                    gtk4::Align::Start => 12.0,
                    gtk4::Align::Center => (w - lw) / 2.0,
                    gtk4::Align::End => w - lw - 12.0,
                    _ => 12.0,
                };
                let y = match valign {
                    gtk4::Align::Start => 12.0,
                    gtk4::Align::Center => (h - lh) / 2.0,
                    gtk4::Align::End => h - lh - 12.0,
                    _ => h - lh - 12.0,
                };
                let _ = cr.save();
                cr.translate(x, y);
                cr.scale(lw / pb_w, lh / pb_h);
                cr.set_source_pixbuf(&pixbuf_clone, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            });
        }

        preview_overlay.add_overlay(&preview_draw);

        let pos_grid = gtk4::Grid::new();
        pos_grid.set_column_spacing(2);
        pos_grid.set_row_spacing(2);
        pos_grid.set_halign(gtk4::Align::Fill);
        pos_grid.set_valign(gtk4::Align::Fill);
        pos_grid.set_hexpand(true);
        pos_grid.set_vexpand(true);

        let mut all_btns: Vec<gtk4::Button> = Vec::new();
        for (i, &pos) in logo_positions.iter().enumerate() {
            let btn = gtk4::Button::new();
            btn.add_css_class("logo-pos-overlay-btn");
            if pos == game.logo_position {
                btn.add_css_class("selected");
            }
            btn.set_hexpand(true);
            btn.set_vexpand(true);
            let row = i / 3;
            let col = i % 3;
            pos_grid.attach(&btn, col as i32, row as i32, 1, 1);
            all_btns.push(btn);
        }

        let btns: Rc<Vec<gtk4::Button>> = Rc::new(all_btns);
        for (i, &pos) in logo_positions.iter().enumerate() {
            let btns_c = btns.clone();
            let selected_pos_c = selected_pos.clone();
            let pos_owned = pos.to_string();
            let preview_clone = preview_draw.clone();
            btns[i].connect_clicked(move |btn| {
                for b in btns_c.iter() {
                    b.remove_css_class("selected");
                }
                btn.add_css_class("selected");
                *selected_pos_c.borrow_mut() = pos_owned.clone();
                preview_clone.queue_draw();
            });
        }

        preview_overlay.add_overlay(&pos_grid);

        let preview_frame = gtk4::Frame::new(None::<&str>);
        preview_frame.set_child(Some(&preview_overlay));
        logo_page.append(&preview_frame);

        let size_label = gtk4::Label::new(Some("Size (% of hero height)"));
        size_label.set_halign(gtk4::Align::Start);
        size_label.add_css_class("heading");
        logo_page.append(&size_label);

        let size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        size_row.set_hexpand(true);

        let size_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&size_adj));
        size_scale.set_draw_value(false);
        size_scale.set_hexpand(true);

        let size_spin = gtk4::SpinButton::new(Some(&size_adj), 1.0, 1);
        size_spin.set_numeric(true);
        size_spin.set_digits(1);

        let preview_draw_for_size = preview_draw.clone();
        size_adj.connect_value_changed(move |_| {
            preview_draw_for_size.queue_draw();
        });

        size_row.append(&size_scale);
        size_row.append(&size_spin);
        logo_page.append(&size_row);

        sidebar.append(&sidebar_row("preferences-desktop-wallpaper-symbolic", "Logo"));
        stack.add_named(&logo_page, Some("logo"));
        Some((selected_pos, size_adj))
    } else {
        None
    };

    let pending_copies: Rc<RefCell<HashMap<String, String>>> = Default::default();
    if !game.app_id.is_empty() {
        let images_page = build_image_manager_content_with_drafts(state, game, &win, Some(pending_copies.clone()));
        sidebar.append(&sidebar_row("image-x-generic-symbolic", "Images"));
        stack.add_named(&images_page, Some("images"));
    }

    let dlc_switches: Vec<adw::SwitchRow> = if let Some(ref details) = app_details {
        if !details.dlcs.is_empty() {
            let dlc_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            let dlc_group = adw::PreferencesGroup::new();
            dlc_group.set_title(&format!("DLCs  ·  {}", details.dlcs.len()));

            let mut dlc_list: Vec<(String, crate::api::types::DlcInfo)> = details.dlcs.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            dlc_list.sort_by_key(|(_, d)| d.app_id);

            let mut switches: Vec<adw::SwitchRow> = Vec::new();
            for (_, dlc) in &dlc_list {
                let row = adw::SwitchRow::new();
                row.set_use_markup(false);
                row.set_title(&dlc.name);
                row.set_subtitle(&format!("App ID: {}", dlc.app_id));
                row.set_active(dlc.enabled);
                dlc_group.add(&row);
                switches.push(row);
            }
            dlc_page.append(&dlc_group);

            let dlc_scroll = gtk4::ScrolledWindow::new();
            dlc_scroll.set_child(Some(&dlc_page));
            dlc_scroll.set_vexpand(true);
            dlc_scroll.set_hexpand(true);

            sidebar.append(&sidebar_row("package-x-generic-symbolic", "DLC"));
            stack.add_named(&dlc_scroll, Some("dlc"));
            switches
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    sidebar_area.set_hexpand(false);

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "Logo" => "logo",
                                "Images" => "images",
                                "DLC" => "dlc",
                                _ => "general",
                            };
                            stack_clone.set_visible_child_name(page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    content_area.append(&stack);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(S::CANCEL);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");
    let state_clone = state.clone();
    let db_id = game.db_id;
    let lutris_id = game.lutris_id;
    let app_id = game.app_id.clone();
    let title_entry_c = title_entry.clone();
    let sort_entry_c = sort_entry.clone();
    let logo_controls_c = logo_controls.clone();
    let dlc_switches_c = dlc_switches.clone();
    let app_details_c = Rc::new(RefCell::new(app_details.clone()));
    let win_s = win.clone();
    let pending_version_save = pending_version.clone();
    let pending_copies_save = pending_copies.clone();
    let sgdb_id_save = game.sgdb_id.clone();
    let kind_save = game.kind.clone();
    save_btn.connect_clicked(move |_| {
        let title = title_entry_c.text().to_string();
        let sort_title = sort_entry_c.text().to_string();
        if let Err(e) = crate::db::update_game_title(&state_clone.borrow().db, db_id, &title) {
            eprintln!("Failed to update game: {}", e);
        }
        if let Err(e) = crate::db::update_sort_title(&state_clone.borrow().db, db_id, &sort_title) {
            eprintln!("Failed to update sort title: {}", e);
        }

        if let Some(ver) = pending_version_save.borrow().as_ref() {
            let _ = crate::db::set_shadps4_version(&state_clone.borrow().db, db_id, ver);
        }

        {
            let pc = pending_copies_save.borrow();
            for (asset, src_path) in pc.iter() {
                let cloud_dir = if !sgdb_id_save.is_empty() {
                    crate::parser::sgdb_data_dir(SAVE_DIR, &sgdb_id_save)
                } else if kind_save == "sgdb" {
                    crate::parser::sgdb_data_dir(SAVE_DIR, &app_id)
                } else if kind_save == "ps4" {
                    crate::parser::ps4_data_dir(SAVE_DIR, &app_id)
                } else {
                    crate::parser::data_dir(SAVE_DIR, &app_id)
                };
                let file_name = match asset.as_str() {
                    "icon" => "icon.png",
                    "hero" => "library_hero.jpg",
                    "grid" => "library_600x900.jpg",
                    "header" => "header.jpg",
                    "logo" => "logo.png",
                    _ => continue,
                };
                let dest = cloud_dir.join(file_name);
                let is_ico = src_path.ends_with(".ico");
                if is_ico {
                    let ico_dest = dest.with_extension("ico");
                    if std::fs::copy(&src_path, &ico_dest).is_ok() {
                        let _ = crate::parser::convert_ico_to_png(&ico_dest);
                    }
                } else if let Err(e) = std::fs::copy(&src_path, &dest) {
                    eprintln!("Failed to copy {}: {}", asset, e);
                }
            }
        }

        if pending_copies_save.borrow().contains_key("__unmatch__") {
            let _ = crate::db::set_sgdb_id(&state_clone.borrow().db, db_id, "");
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.sgdb_id.clear();
            }
            pending_copies_save.borrow_mut().remove("__unmatch__");
        }

        if let Some((ref selected_pos, ref size_adj)) = logo_controls_c {
            let pos = selected_pos.borrow().clone();
            let size = size_adj.value() as i32;
            if db_id != 0 {
                if let Err(e) = crate::db::set_logo_settings(&state_clone.borrow().db, db_id, &pos, size) {
                    eprintln!("Failed to update logo settings: {}", e);
                }
            }
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.logo_position = pos;
                g.logo_size = size;
            }
        }

        {
            let mut details_ref = app_details_c.borrow_mut();
            if let Some(ref mut details) = *details_ref {
                if !dlc_switches_c.is_empty() {
                    let dlcs_vec: Vec<_> = details.dlcs.iter_mut().collect();
                    for (i, (_, dlc)) in dlcs_vec.into_iter().enumerate() {
                        if i < dlc_switches_c.len() {
                            dlc.enabled = dlc_switches_c[i].is_active();
                        }
                    }
                    let path = crate::parser::data_dir(SAVE_DIR, &app_id).join("appdetails.json");
                    if let Ok(b) = serde_json::to_vec(&*details) {
                        let _ = std::fs::write(&path, b);
                    }
                }
            }
        }

        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.name = title.clone();
            g.sort_title = sort_title.clone();
            if let Some(ver) = pending_version_save.borrow().as_ref() {
                g.shadps4_version = ver.clone();
            }
        }
        if !app_id.is_empty() {
            state_clone.borrow().game_names.lock().unwrap().insert(app_id.clone(), title);
        }
        rebuild_sidebar(&state_clone);

        let selected = state_clone.borrow().selected_id.clone();
        if selected == lutris_id.to_string() {
            let g = state_clone.borrow().games.iter()
                .find(|g| g.lutris_id == lutris_id)
                .cloned();
            if let Some(g) = g {
                display_game(&g, &state_clone);
            }
        }

        win_s.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);
    outer.append(&content_area);

    win.set_content(Some(&outer));
    win.present();

    {
        let mut s = state.borrow_mut();
        s.settings_data = Some((win.clone(), stack.clone(), game.db_id));
    }
    let state_c = state.clone();
    win.connect_destroy(move |_| {
        state_c.borrow_mut().settings_data = None;
    });
}

pub fn build_image_manager_content(state: &SharedState, game: &Game, parent_win: &adw::Window) -> gtk4::Box {
    build_image_manager_content_with_drafts(state, game, parent_win, None)
}

pub fn build_image_manager_content_with_drafts(
    state: &SharedState,
    game: &Game,
    parent_win: &adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
) -> gtk4::Box {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let is_steam = game.kind == "gbe_steam" || game.kind == "ne_gog";
    let id = game.app_id.clone();

    let cloud_dir = if !game.sgdb_id.is_empty() {
        crate::parser::sgdb_data_dir(SAVE_DIR, &game.sgdb_id)
    } else if game.kind == "sgdb" {
        crate::parser::sgdb_data_dir(SAVE_DIR, &id)
    } else if game.kind == "ps4" {
        crate::parser::ps4_data_dir(SAVE_DIR, &id)
    } else {
        crate::parser::data_dir(SAVE_DIR, &id)
    };
    let cloud_base = cloud_dir.to_string_lossy().into_owned();

    let find_best_path = |game: &Game, field: &str, file: &str| -> String {
        let field_path = match field {
            "icon" if !game.icon_path.is_empty() => game.icon_path.clone(),
            "hero" if !game.hero_image_path.is_empty() => game.hero_image_path.clone(),
            "grid" if !game.grid_path.is_empty() => game.grid_path.clone(),
            "header" if !game.header_path.is_empty() => game.header_path.clone(),
            "logo" if !game.logo_path.is_empty() => game.logo_path.clone(),
            _ => String::new(),
        };
        if !field_path.is_empty() && std::path::Path::new(&field_path).is_file() {
            return field_path;
        }
        if !game.sgdb_id.is_empty() {
            let sgdb = format!("{}/{}", crate::parser::sgdb_data_dir(SAVE_DIR, &game.sgdb_id).to_string_lossy(), file);
            if std::path::Path::new(&sgdb).is_file() {
                return sgdb;
            }
        }
        let native = if game.kind == "ps4" {
            format!("{}/{}", crate::parser::ps4_data_dir(SAVE_DIR, &id).to_string_lossy(), file)
        } else {
            format!("{}/{}", crate::parser::data_dir(SAVE_DIR, &id).to_string_lossy(), file)
        };
        if std::path::Path::new(&native).is_file() {
            return native;
        }
        if field == "icon" && game.kind == "ps4" && !game.icon_path.is_empty() && std::path::Path::new(&game.icon_path).is_file() {
            return game.icon_path.clone();
        }
        String::new()
    };

    let sections: [(&str, &str, &str, i32, i32, &[&str]); 5] = [
        ("Icon", "icon.png", "icon", 48, 48, &[]),
        ("Hero", "library_hero.jpg", "hero", 96, 48, &[]),
        ("Capsule", "library_600x900.jpg", "grid", 32, 48, &["600x900"]),
        ("Header", "header.jpg", "header", 96, 48, &["460x215", "920x430"]),
        ("Logo", "logo.png", "logo", 96, 48, &[]),
    ];
    for &(label, file, asset, thumb_w, thumb_h, dimensions) in &sections {
        let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.add_css_class("heading");
        section.append(&lbl);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_hexpand(true);
        row.set_valign(gtk4::Align::Center);

        let img_path = {
            let draft_path = pending_copies.as_ref()
                .and_then(|pc| pc.borrow().get(asset).cloned());
            if let Some(ref src) = draft_path {
                if std::path::Path::new(src).is_file() {
                    src.clone()
                } else {
                    find_best_path(game, asset, file)
                }
            } else {
                find_best_path(game, asset, file)
            }
        };

        let preview = gtk4::Picture::for_filename(&img_path);
        preview.set_content_fit(gtk4::ContentFit::ScaleDown);
        preview.set_size_request(thumb_w, thumb_h);
        let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        preview_wrapper.set_size_request(thumb_w, 48);
        preview_wrapper.set_valign(gtk4::Align::Center);
        if !img_path.is_empty() && std::path::Path::new(&img_path).is_file() {
            preview_wrapper.append(&preview);
        } else {
            let ph = gtk4::Label::new(Some("—"));
            ph.add_css_class("dim-label");
            preview_wrapper.append(&ph);
        }
        row.append(&preview_wrapper);

        let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        btns.set_hexpand(true);
        btns.set_halign(gtk4::Align::End);

        let dest_path = format!("{}/{}", cloud_base, file);
        let refresh_images: Rc<dyn Fn()> = Rc::new({
            let preview_wrapper = preview_wrapper.clone();
            let dest_path = dest_path.clone();
            let state_clone = state.clone();
            let game_clone = game.clone();
            let pending_copies = pending_copies.clone();
            let asset_c = asset.to_string();
            move || {
                while let Some(child) = preview_wrapper.first_child() {
                    preview_wrapper.remove(&child);
                }
                let preview_src = pending_copies.as_ref()
                    .and_then(|pc| pc.borrow().get(&asset_c).cloned())
                    .filter(|p| std::path::Path::new(p).is_file())
                    .or_else(|| {
                        if std::path::Path::new(&dest_path).exists() {
                            Some(dest_path.clone())
                        } else {
                            None
                        }
                    });
                if let Some(path) = preview_src {
                    let p = gtk4::Picture::for_filename(&path);
                    p.set_content_fit(gtk4::ContentFit::ScaleDown);
                    preview_wrapper.append(&p);
                } else {
                    let ph = gtk4::Label::new(Some("—"));
                    ph.add_css_class("dim-label");
                    preview_wrapper.append(&ph);
                }
                let s = state_clone.borrow();
                if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&s.db, game_clone.lutris_id) {
                    drop(s);
                    if let Ok(updated) = crate::parser::load_game(&entry, SAVE_DIR) {
                        apply_game_update(&state_clone, updated);
                    }
                }
            }
        });

        let browse_btn = gtk4::Button::with_label("Browse…");
        let state_browse = state.clone();
        let refresh = refresh_images.clone();
        let dest_path_btn = dest_path.clone();
        let pending_copies_btn = pending_copies.clone();
        let asset_btn = asset.to_string();
        let label_btn = label.to_string();
        let pending_copies_browse = pending_copies_btn.clone();
        browse_btn.connect_clicked(move |_| {
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            filter.add_mime_type("image/x-icon");
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select image");
            dialog.set_default_filter(Some(&filter));
            let _dest = dest_path_btn.clone();
            let refresh_c = refresh.clone();
            let pc = pending_copies_browse.clone();
            let asset_name = asset_btn.clone();
            let _label_name = label_btn.clone();
            dialog.open(Some(&state_browse.borrow().window), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        if let Some(ref pc_inner) = pc {
                            pc_inner.borrow_mut().insert(asset_name.clone(), path.to_string_lossy().into_owned());
                            refresh_c();
                        }
                    }
                }
            });
        });
        btns.append(&browse_btn);

        if is_steam && asset != "icon" {
            let btn = gtk4::Button::with_label("Steam");
            let steam = state.borrow().steam.clone();
            let id_c = id.clone();
            let asset_c = asset.to_string();
            let refresh = refresh_images.clone();
            btn.connect_clicked(move |_| {
                let _ = steam.force_download_steam(&id_c, &asset_c);
                refresh();
            });
            btns.append(&btn);
        }

        let sgdb_id_for_picker = if !game.sgdb_id.is_empty() {
            game.sgdb_id.clone()
        } else {
            id.clone()
        };
        let sgdb_is_steam_id = is_steam && game.sgdb_id.is_empty();
        if !sgdb_id_for_picker.is_empty() {
            let btn = gtk4::Button::with_label("SGDB…");
            let steam = state.borrow().steam.clone();
            let asset_c = asset.to_string();
            let parent = parent_win.clone();
            let refresh = refresh_images.clone();
            let dims: Vec<&str> = dimensions.to_vec();
            let sgdb_id_c = sgdb_id_for_picker.clone();
            btn.connect_clicked(move |_| {
                show_sgdb_picker(&steam, &sgdb_id_c, &asset_c, sgdb_is_steam_id, &dims, &parent, refresh.clone(), pending_copies_btn.clone());
            });
            btns.append(&btn);
        }

        if asset == "icon" && game.kind == "ps4" {
            let reset_btn = gtk4::Button::with_label("Reset");
            let _sc = state.clone();
            let gc = game.clone();
            let refresh = refresh_images.clone();
            let pending_copies_reset = pending_copies.clone();
            let asset_reset = asset.to_string();
            reset_btn.connect_clicked(move |_| {
                let app_id = gc.app_id.clone();
                let game_path = gc.game_path.clone();
                let image_dir = std::path::Path::new(SAVE_DIR).join("data").join("ps4").join(&app_id);
                let icon_png = image_dir.join("icon.png");
                let default_path = if icon_png.is_file() {
                    Some(icon_png.to_string_lossy().into_owned())
                } else {
                    let game_icon = std::path::Path::new(&game_path).join("sce_sys").join("icon0.png");
                    if game_icon.is_file() {
                        let _ = std::fs::create_dir_all(&image_dir);
                        let _ = std::fs::copy(&game_icon, &icon_png);
                        Some(icon_png.to_string_lossy().into_owned())
                    } else {
                        None
                    }
                };
                if let Some(ref pc) = pending_copies_reset {
                    pc.borrow_mut().remove(&asset_reset);
                    if let Some(path) = default_path {
                        pc.borrow_mut().insert(asset_reset.clone(), path);
                    }
                }
                refresh();
            });
            btns.append(&reset_btn);
        }

        row.append(&btns);
        section.append(&row);
        content.append(&section);
    }

    {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        btn_box.set_halign(gtk4::Align::Center);
        btn_box.set_margin_top(24);

        if game.sgdb_id.is_empty() && !is_steam && game.kind != "sgdb" {
            let match_btn = gtk4::Button::with_label("Match to SteamGridDB…");
            match_btn.add_css_class("suggested-action");
            let sc = state.clone();
            let gn = game.name.clone();
            let did = game.db_id;
            let pw = parent_win.clone();
            match_btn.connect_clicked(move |_| {
                show_sgdb_search_dialog(&sc, did, &gn, &pw, None);
            });
            btn_box.append(&match_btn);
        }

        if !game.sgdb_id.is_empty() {
            let label = gtk4::Label::new(Some(&format!("Matched (SGDB ID: {})", game.sgdb_id)));
            label.add_css_class("success-label");
            btn_box.append(&label);
            let unmatch_btn = gtk4::Button::with_label("Unmatch SGDB");
            unmatch_btn.add_css_class("destructive-action");
            let pending_pc = pending_copies.clone();
            let sc = state.clone();
            let did = game.db_id;
            let pw = parent_win.clone();
            unmatch_btn.connect_clicked(move |_| {
                if let Some(ref pc) = pending_pc {
                    pc.borrow_mut().insert("__unmatch__".to_string(), String::new());
                    if let Some((ref sw, ref ss, sdb_id)) = sc.borrow().settings_data.clone() {
                        if sdb_id == did && sw.is_visible() {
                            if let Some(old) = ss.child_by_name("images") {
                                ss.remove(&old);
                            }
                            if let Some(game) = sc.borrow().games.iter().find(|g| g.db_id == did).cloned() {
                                let mut g2 = game.clone();
                                g2.sgdb_id.clear();
                                let new_page = build_image_manager_content_with_drafts(&sc, &g2, &pw, Some(pc.clone()));
                                ss.add_named(&new_page, Some("images"));
                            }
                        }
                    }
                }
            });
            btn_box.append(&unmatch_btn);
        }

        content.append(&btn_box);
    }

    content
}

fn show_sgdb_picker(steam: &Arc<SteamClient>, id: &str, asset: &str, is_steam_id: bool, dimensions: &[&str], parent: &adw::Window, on_done: Rc<dyn Fn()>, pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>) {
    let picker = adw::Window::new();
    picker.set_default_width(600);
    picker.set_default_height(500);
    picker.set_transient_for(Some(parent));
    picker.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&format!("Pick {}", asset)))));

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name("view-list-symbolic");
    toggle_btn.set_tooltip_text(Some("Switch to list view"));
    toggle_btn.add_css_class("flat");
    header_bar.pack_end(&toggle_btn);

    outer.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);

    let stack = gtk4::Stack::new();

    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(8);
    flow.set_row_spacing(8);
    flow.set_column_spacing(8);
    flow.set_margin_start(12);
    flow.set_margin_end(12);
    flow.set_margin_top(8);
    flow.set_margin_bottom(8);
    flow.set_halign(gtk4::Align::Fill);

    let list_view = gtk4::ListBox::new();
    list_view.set_selection_mode(gtk4::SelectionMode::None);
    list_view.set_margin_start(12);
    list_view.set_margin_end(12);
    list_view.set_margin_top(8);
    list_view.set_margin_bottom(8);

    stack.add_named(&flow, Some("grid"));
    stack.add_named(&list_view, Some("list"));
    stack.set_visible_child_name("grid");

    let loading = gtk4::Label::new(Some("Loading\u{2026}"));
    loading.add_css_class("dim-label");
    flow.append(&loading);
    list_view.append(&gtk4::Label::new(Some("Loading\u{2026}")));

    scrolled.set_child(Some(&stack));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    picker.set_content(Some(&outer));
    picker.present();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<SgdbAsset>>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = steam.clone();
    let id_c = id.to_string();
    let asset_c = asset.to_string();
    let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let results = steam_c.list_sgdb_assets(&id_c, &asset_c, is_steam_id, &dims_refs);
        let _ = tx.send(results);
    });

    let steam_clone = steam.clone();
    let id_clone = id.to_string();
    let asset_clone = asset.to_string();
    let picker_clone = picker.clone();
    let on_done = on_done.clone();

    let stack_toggle = stack.clone();
    toggle_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            stack_toggle.set_visible_child_name("list");
            btn.set_icon_name("view-grid-symbolic");
            btn.set_tooltip_text(Some("Switch to grid view"));
        } else {
            stack_toggle.set_visible_child_name("grid");
            btn.set_icon_name("view-list-symbolic");
            btn.set_tooltip_text(Some("Switch to list view"));
        }
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(assets) = rx.borrow_mut().try_recv() {
            while let Some(child) = flow.first_child() { flow.remove(&child); }
            while let Some(child) = list_view.first_child() { list_view.remove(&child); }

            if assets.is_empty() {
                let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
                none.add_css_class("dim-label");
                flow.append(&none);
                list_view.append(&gtk4::Label::new(Some("No images found on SteamGridDB")));
                return glib::ControlFlow::Break;
            }

            for a in assets {
                let thumb_size = if asset_clone == "header" { 138 } else { 90 };

                let mut info = String::new();
                if a.width > 0 && a.height > 0 {
                    info = format!("{}\u{d7}{}", a.width, a.height);
                }
                if !a.style.is_empty() {
                    if !info.is_empty() { info = format!("{} \u{b7} {}", info, a.style); }
                    else { info = a.style.clone(); }
                }
                if !a.author.is_empty() {
                    if !info.is_empty() { info = format!("{} \u{b7} by {}", info, a.author); }
                    else { info = format!("by {}", a.author); }
                }

                let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
                card.set_halign(gtk4::Align::Center);
                card.set_valign(gtk4::Align::Start);
                card.set_margin_top(4);
                card.set_margin_bottom(4);

                let grid_pic = gtk4::Picture::new();
                grid_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
                grid_pic.set_size_request(thumb_size, thumb_size);
                card.append(&grid_pic);

                let ilbl = gtk4::Label::new(Some(&info));
                ilbl.set_xalign(0.5);
                ilbl.set_max_width_chars(20);
                ilbl.set_wrap(true);
                ilbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                ilbl.add_css_class("dim-label");
                card.append(&ilbl);

                let gdl = gtk4::Button::with_label("Download");
                gdl.add_css_class("suggested-action");
                gdl.set_halign(gtk4::Align::Center);
                card.append(&gdl);
                flow.append(&card);

                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                row.set_margin_top(4);
                row.set_margin_bottom(4);

                let list_pic = gtk4::Picture::new();
                list_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
                list_pic.set_size_request(48, 48);
                row.append(&list_pic);

                let rlbl = gtk4::Label::new(Some(&info));
                rlbl.set_xalign(0.0);
                rlbl.set_hexpand(true);
                rlbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                row.append(&rlbl);

                let ldl = gtk4::Button::with_label("Download");
                ldl.add_css_class("suggested-action");
                row.append(&ldl);
                list_view.append(&row);

                let data_subdir = if is_steam_id { "steam".to_string() } else { "steamgriddb".to_string() };
                let dest_dir = format!("{}/data/{}/{}", SAVE_DIR, data_subdir, id_clone);
                let file_name = match asset_clone.as_str() {
                    "icon" => {
                        let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") { "ico" }
                        else if a.mime.contains("png") { "png" }
                        else if a.mime.contains("jpeg") || a.mime.contains("jpg") { "jpg" }
                        else if a.mime.contains("webp") { "webp" }
                        else { std::path::Path::new(&a.url).extension().and_then(|e| e.to_str()).unwrap_or("png") };
                        format!("icon.{}", ext)
                    }
                    "hero" => "library_hero.jpg".to_string(),
                    "grid" => "library_600x900.jpg".to_string(),
                    "header" => "header.jpg".to_string(),
                    "logo" => "logo.png".to_string(),
                    _ => continue,
                };
                let dest = format!("{}/{}", dest_dir, file_name);
                let dl_url = a.url.clone();
                let steam_dl = steam_clone.clone();
                let picker_dl = picker_clone.clone();
                let on_done_dl = on_done.clone();
                let _dest_dl = dest.clone();
                let _fn_dl = file_name.clone();
                let _dir_dl = dest_dir.clone();
                let asset_dl = asset_clone.clone();
                let pending_dl = pending_copies.clone();
                let cb: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(ref pc) = pending_dl {
                        let tmp = std::env::temp_dir().join(format!("sgdb_{}", asset_dl));
                        if steam_dl.download_file(&dl_url, &tmp).is_ok() {
                            pc.borrow_mut().insert(asset_dl.clone(), tmp.to_string_lossy().into_owned());
                            on_done_dl();
                            picker_dl.close();
                        } else {
                            eprintln!("Download failed for {}", dl_url);
                        }
                    }
                });
                let cb_g = cb.clone();
                gdl.connect_clicked(move |_| cb_g());
                ldl.connect_clicked(move |_| cb());

                let url_clone = a.url.clone();
                let steam_thumb = steam_clone.clone();
                let thumb_dir = format!("{}/data/.thumbnails", SAVE_DIR);
                let _ = std::fs::create_dir_all(&thumb_dir);
                let thumb_name = format!("{}/{}", thumb_dir, url_clone.rsplit('/').next().unwrap_or("thumb"));
                let tsize = thumb_size;
                let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<String>>();
                let rx_thumb = std::cell::RefCell::new(rx_thumb);
                std::thread::spawn(move || {
                    let final_path = if std::path::Path::new(&thumb_name).exists() {
                        Some(thumb_name.clone())
                    } else if steam_thumb.download_file(&url_clone, std::path::Path::new(&thumb_name)).is_ok() {
                        let mut path = thumb_name.clone();
                        if std::path::Path::new(&thumb_name).extension().and_then(|e| e.to_str()) == Some("ico") {
                            if let Ok(img) = image::open(&thumb_name) {
                                let png_path = std::path::Path::new(&thumb_name).with_extension("png");
                                if img.save(&png_path).is_ok() {
                                    let _ = std::fs::remove_file(&thumb_name);
                                    path = png_path.to_string_lossy().into_owned();
                                }
                            }
                        }
                        if let Ok(img) = image::open(&path) {
                            let (w, h) = (img.width(), img.height());
                            if w > tsize as u32 || h > tsize as u32 {
                                let resized = img.resize(tsize as u32, tsize as u32, image::imageops::FilterType::Lanczos3);
                                let _ = resized.save(&path);
                            }
                        }
                        Some(path)
                    } else {
                        None
                    };
                    let _ = tx_thumb.send(final_path);
                });
                let tp_g = grid_pic.clone();
                let tp_l = list_pic.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    if let Ok(path) = rx_thumb.borrow_mut().try_recv() {
                        if let Some(p) = path {
                            tp_g.set_filename(Some(&p));
                            tp_l.set_filename(Some(&p));
                        }
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
