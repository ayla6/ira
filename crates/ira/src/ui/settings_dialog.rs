use gtk4::prelude::*;
use adw::prelude::*;
use ira_api::SteamDataClient;
use ira_config::Config;
use crate::strings as S;
use std::sync::Arc;
use super::helpers::dialog_layout;
use super::profile_dialog::build_profiles_page;
use super::state::SharedState;
use super::wine_config_widget::build_wine_config_pages;
use super::settings_pages::{
    build_general_settings_page, build_api_keys_page, build_lutris_settings_page,
    build_steam_settings_page, build_ra_settings_page, build_api_emulators_page,
};
use super::settings_console::{
    build_shadps4_settings_page, build_console_settings_page, ConsolePageWidgets,
};

pub(super) fn settings_page_container() -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Vertical, 16)
}

// Re-exports for backward compatibility with files that use super::settings_dialog::*
pub(super) use super::settings_pages::{settings_sidebar_row, sidebar_separator};
pub(super) use super::settings_console::build_shadps4_version_dropdown;

pub fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamDataClient>,
    state: &SharedState,
) {
    let layout = dialog_layout(parent);
    layout.window.set_default_size(640, 480);
    layout.window.set_deletable(false);
    layout.sidebar_area.set_size_request(180, -1);

    let win = layout.window;
    let sidebar = layout.sidebar;
    let stack = layout.stack;
    let content_area = layout.content_area;

    let (general_page, notif_row, bg_row, hidden_row, grid_spin) = build_general_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let (api_page, steam_entry, sgdb_entry) = build_api_keys_page(&cfg);
    sidebar.append(&settings_sidebar_row("dialog-password-symbolic", "API Keys"));
    stack.add_named(&api_page, Some("api"));

    sidebar.append(&sidebar_separator());

    let (steam_page, steam_enable_row) = build_steam_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("application-x-executable-symbolic", "Steam"));
    stack.add_named(&steam_page, Some("steam"));

    let (ra_page, ra_enable_row, ra_username_row, ra_password_row, ra_token_row) = build_ra_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-science-symbolic", "RetroAchievements"));
    stack.add_named(&ra_page, Some("ra"));

    let mut console_widgets: Vec<(&'static str, ConsolePageWidgets)> = Vec::new();
    let mut ps4_enable_row: Option<adw::SwitchRow> = None;
    let mut ps4_exe_row: Option<adw::EntryRow> = None;
    for def in ira_models::CONSOLES {
        let cc = cfg.console(def.id);
        let (page, widgets) = build_console_settings_page(&win, def, cc);
        sidebar.append(&settings_sidebar_row("applications-games-symbolic", def.display_name));
        stack.add_named(&page, Some(def.display_name.to_lowercase().as_str()));
        console_widgets.push((def.id, widgets));

        if def.id == "ps2" {
            let (ps4_page, ps4_en, ps4_exe) = build_shadps4_settings_page(&cfg, &win);
            sidebar.append(&settings_sidebar_row("applications-games-symbolic", "PS4"));
            stack.add_named(&ps4_page, Some("ps4"));
            ps4_enable_row = Some(ps4_en);
            ps4_exe_row = Some(ps4_exe);
        }
    }

    let (wine_pages, wine_widgets) = build_wine_config_pages(&cfg.default_wine_config, None);
    sidebar.append(&sidebar_separator());
    for wp in &wine_pages {
        sidebar.append(&settings_sidebar_row(wp.icon, wp.label));
        stack.add_named(&wp.page, Some(wp.label));
    }

    let profiles_page = build_profiles_page(state, &win);
    sidebar.append(&settings_sidebar_row("system-users-symbolic", "Wine Profiles"));
    stack.add_named(&profiles_page, Some("profiles"));

    sidebar.append(&sidebar_separator());
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-engineering-symbolic", "API Emulators"));
    stack.add_named(&emu_page, Some("api_emulators"));

    let lutris_page = build_lutris_settings_page(state, &win);
    sidebar.append(&settings_sidebar_row("system-software-install-symbolic", "Migration"));
    stack.add_named(&lutris_page, Some("migration"));

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let text = label.text().to_string();
                            let page_id = match text.as_str() {
                                "API Keys" => "api".to_string(),
                                "RetroAchievements" => "ra".to_string(),
                                "Wine Profiles" => "profiles".to_string(),
                                "API Emulators" => "api_emulators".to_string(),
                                _ => {
                                    if stack_clone.child_by_name(&text).is_some() {
                                        text
                                    } else {
                                        let lower = text.to_lowercase();
                                        if stack_clone.child_by_name(&lower).is_some() {
                                            lower
                                        } else {
                                            "general".to_string()
                                        }
                                    }
                                }
                            };
                            stack_clone.set_visible_child_name(&page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

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
        if let Some(row) = &ps4_enable_row {
            s.cfg.shadps4_enabled = row.is_active();
        }
        if let Some(row) = &ps4_exe_row {
            s.cfg.shadps4_executable = row.text().to_string();
        }
        s.cfg.steam_enabled = steam_enable_row.is_active();
        s.cfg.ra_enabled = ra_enable_row.is_active();
        s.cfg.ra_username = ra_username_row.text().to_string();
        s.cfg.ra_password = ra_password_row.text().to_string();
        s.cfg.ra_token = ra_token_row.text().to_string();
        for (console_id, widgets) in &console_widgets {
            let cc = s.cfg.console_mut(console_id);
            cc.enabled = widgets.enable_row.is_active();
            cc.folder = widgets.folder_row.text().to_string();
            cc.executable = widgets.exe_row.text().to_string();
            cc.fullscreen = widgets.fullscreen_row.is_active();
            if let Some(ref dd) = widgets.core_dropdown {
                if dd.selected() > 0 {
                    let cores = ira_platforms::emulator_detect::detect_ra_cores();
                    if let Some(c) = cores.get((dd.selected() - 1) as usize) {
                        cc.ra_core = c.path.clone();
                    }
                } else {
                    cc.ra_core = String::new();
                }
            }
        }
        s.cfg.default_wine_config = wine_widgets.to_wine_config();

        let idx = emu_version_row.selected();
        let ver = emu_version_model.string(idx).map(|s| s.to_string()).unwrap_or_default();
        if !ver.is_empty() && !ver.starts_with("(no versions") {
            s.cfg.default_api_emu_version = ver.to_string();
        }

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
    win.present();
}
