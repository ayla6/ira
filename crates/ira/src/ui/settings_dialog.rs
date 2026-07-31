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
    build_general_settings_page, build_lutris_settings_page,
    build_steam_settings_page, build_ra_settings_page, build_api_emulators_page,
    build_overlay_settings_page,
    build_system_defaults_page, build_computer_games_page,
};
use super::system_settings::{build_override_switch_row, OverrideState};
use super::settings_console::{
    build_shadps4_settings_page, build_rpcs3_settings_page, build_console_settings_page, ConsolePageWidgets,
};
use super::css::*;

pub(super) fn settings_page_container() -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Vertical, 16)
}

// Re-exports for backward compatibility with files that use super::settings_dialog::*
pub(super) use super::settings_pages::{settings_sidebar_row, sidebar_separator, sidebar_section_title};
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

    let (general_page, notif_row, bg_row, hidden_row, steam_entry, sgdb_entry, lang_list, saves_row) = build_general_settings_page(&cfg);
    let general_scroll = gtk4::ScrolledWindow::new();
    general_scroll.set_hexpand(true);
    general_scroll.set_vexpand(true);
    general_scroll.set_child(Some(&general_page));
    sidebar.append(&settings_sidebar_row("preferences-system-symbolic", "General", "general"));
    stack.add_named(&general_scroll, Some("general"));

    let (overlay_page, overlay_widgets) = build_overlay_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("view-grid-symbolic", "Overlay", "overlay"));
    stack.add_named(&overlay_page, Some("overlay"));

    let (system_page, system_defaults_widgets) = build_system_defaults_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-science-symbolic", "Game system", "system"));
    stack.add_named(&system_page, Some("system"));

    sidebar.append(&sidebar_section_title("PC games"));
    let (computer_games_page, default_game_folder_row) = build_computer_games_page(&win, &cfg);
    sidebar.append(&settings_sidebar_row("applications-games-symbolic", "PC games", "computer_games"));
    stack.add_named(&computer_games_page, Some("computer_games"));

    let (steam_page, steam_enable_row) = build_steam_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("application-x-executable-symbolic", "Steam", "steam"));
    stack.add_named(&steam_page, Some("steam"));

    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-engineering-symbolic", "API emulators", "api_emulators"));
    stack.add_named(&emu_page, Some("api_emulators"));

    let lutris_page = build_lutris_settings_page(state, &win);
    sidebar.append(&settings_sidebar_row("system-software-install-symbolic", "Lutris migration", "migration"));
    stack.add_named(&lutris_page, Some("migration"));

    sidebar.append(&sidebar_section_title("Wine"));
    let (wine_pages, wine_widgets) = build_wine_config_pages(&cfg.default_wine_config, None);
    let (profiles_page, prefix_base_row) = build_profiles_page(state, &win);
    sidebar.append(&settings_sidebar_row("system-users-symbolic", "Profiles", "profiles"));
    stack.add_named(&profiles_page, Some("profiles"));
    for wp in &wine_pages {
        sidebar.append(&settings_sidebar_row(wp.icon, wp.label, wp.label));
        stack.add_named(&wp.page, Some(wp.label));
    }

    sidebar.append(&sidebar_section_title("Emulation"));
    let (ra_page, ra_enable_row, ra_username_row, ra_password_row) = build_ra_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-science-symbolic", "RetroAchievements", "ra"));
    stack.add_named(&ra_page, Some("ra"));

    let mut source_overlay_states: Vec<(String, OverrideState)> = Vec::new();
    let mut source_gamescope_states: Vec<(String, OverrideState)> = Vec::new();

    {
        let (overlay_row, state) = build_override_switch_row(
            "In-game overlay", "Achievements, screenshots, and recording",
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("steam").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            "Gamescope", "Valve Gamescope compositor",
            cfg.default_system.gamescope,
            cfg.overlay.source_gamescope.get("steam").copied(),
        );
        let g = adw::PreferencesGroup::new();
        g.add(&overlay_row);
        g.add(&gs_row);
        steam_page.append(&g);
        source_overlay_states.push(("steam".to_string(), state));
        source_gamescope_states.push(("steam".to_string(), gs_state));
    }

    {
        let (overlay_row, state) = build_override_switch_row(
            "In-game overlay", "Achievements, screenshots, and recording",
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("ra").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            "Gamescope", "Valve Gamescope compositor",
            cfg.default_system.gamescope,
            cfg.overlay.source_gamescope.get("ra").copied(),
        );
        let g = adw::PreferencesGroup::new();
        g.add(&overlay_row);
        g.add(&gs_row);
        ra_page.append(&g);
        source_overlay_states.push(("ra".to_string(), state));
        source_gamescope_states.push(("ra".to_string(), gs_state));
    }

    let mut console_widgets: Vec<(&'static str, ConsolePageWidgets)> = Vec::new();
    let mut ps4_enable_row: Option<adw::SwitchRow> = None;
    let mut ps4_exe_row: Option<adw::EntryRow> = None;
    let mut ps3_enable_row: Option<adw::SwitchRow> = None;
    let mut ps3_exe_row: Option<adw::EntryRow> = None;
    for def in ira_models::CONSOLES {
        let cc = cfg.console(def.id);
        let (page, widgets) = build_console_settings_page(&win, def, cc);

        let (overlay_row, overlay_state) = build_override_switch_row(
            "In-game overlay", "Achievements, screenshots, and recording",
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get(def.id).copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            "Gamescope", "Valve Gamescope compositor",
            cfg.default_system.gamescope,
            cfg.overlay.source_gamescope.get(def.id).copied(),
        );
        let overlay_group = adw::PreferencesGroup::new();
        overlay_group.add(&overlay_row);
        overlay_group.add(&gs_row);
        page.append(&overlay_group);
        source_overlay_states.push((def.id.to_string(), overlay_state));
        source_gamescope_states.push((def.id.to_string(), gs_state));

        let page_id = def.display_name.to_lowercase();
        sidebar.append(&settings_sidebar_row("applications-games-symbolic", def.display_name, &page_id));
        stack.add_named(&page, Some(page_id.as_str()));
        console_widgets.push((def.id, widgets));

        if def.id == "ps2" {
            let (ps3_page, ps3_en, ps3_exe) = build_rpcs3_settings_page(&cfg, &win);

            let (ps3_ov_row, ps3_ov_state) = build_override_switch_row(
                "In-game overlay", "Achievements, screenshots, and recording",
                cfg.overlay.enabled,
                cfg.overlay.source_overrides.get("ps3").copied(),
            );
            let (ps3_gs_row, ps3_gs_state) = build_override_switch_row(
                "Gamescope", "Valve Gamescope compositor",
                cfg.default_system.gamescope,
                cfg.overlay.source_gamescope.get("ps3").copied(),
            );
            let ps3_ov_group = adw::PreferencesGroup::new();
            ps3_ov_group.add(&ps3_ov_row);
            ps3_ov_group.add(&ps3_gs_row);
            ps3_page.append(&ps3_ov_group);
            source_overlay_states.push(("ps3".to_string(), ps3_ov_state));
            source_gamescope_states.push(("ps3".to_string(), ps3_gs_state));

            sidebar.append(&settings_sidebar_row("applications-games-symbolic", "PS3", "ps3"));
            stack.add_named(&ps3_page, Some("ps3"));
            ps3_enable_row = Some(ps3_en);
            ps3_exe_row = Some(ps3_exe);

            let (ps4_page, ps4_en, ps4_exe) = build_shadps4_settings_page(&cfg, &win);

            let (ps4_ov_row, ps4_ov_state) = build_override_switch_row(
                "In-game overlay", "Achievements, screenshots, and recording",
                cfg.overlay.enabled,
                cfg.overlay.source_overrides.get("ps4").copied(),
            );
            let (ps4_gs_row, ps4_gs_state) = build_override_switch_row(
                "Gamescope", "Valve Gamescope compositor",
                cfg.default_system.gamescope,
                cfg.overlay.source_gamescope.get("ps4").copied(),
            );
            let ps4_ov_group = adw::PreferencesGroup::new();
            ps4_ov_group.add(&ps4_ov_row);
            ps4_ov_group.add(&ps4_gs_row);
            ps4_page.append(&ps4_ov_group);
            source_overlay_states.push(("ps4".to_string(), ps4_ov_state));
            source_gamescope_states.push(("ps4".to_string(), ps4_gs_state));

            sidebar.append(&settings_sidebar_row("applications-games-symbolic", "PS4", "ps4"));
            stack.add_named(&ps4_page, Some("ps4"));
            ps4_enable_row = Some(ps4_en);
            ps4_exe_row = Some(ps4_exe);
        }
    }

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let page_id = row.widget_name().to_string().to_string();
            stack_clone.set_visible_child_name(&page_id);
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
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);

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
        s.cfg.centralize_game_saves = saves_row.is_active();
        if let Some(row) = &ps4_enable_row {
            s.cfg.shadps4_enabled = row.is_active();
        }
        if let Some(row) = &ps4_exe_row {
            s.cfg.shadps4_executable = row.text().to_string();
        }
        if let Some(row) = &ps3_enable_row {
            s.cfg.rpcs3_enabled = row.is_active();
        }
        if let Some(row) = &ps3_exe_row {
            s.cfg.rpcs3_executable = row.text().to_string();
        }
        s.cfg.steam_enabled = steam_enable_row.is_active();
        s.cfg.default_game_folder = default_game_folder_row.text().to_string();
        s.cfg.language_preferences = super::settings_pages::read_language_preferences(&lang_list);
        s.cfg.ra_enabled = ra_enable_row.is_active();
        s.cfg.ra_username = ra_username_row.text().to_string();
        s.cfg.ra_password = ra_password_row.text().to_string();

        s.cfg.overlay.enabled = overlay_widgets.enable_row.is_active();
        s.cfg.overlay.encoder = match overlay_widgets.encoder_row.selected() {
            1 => ira_overlay_ipc::VideoEncoder::Vaapi,
            2 => ira_overlay_ipc::VideoEncoder::Nvenc,
            3 => ira_overlay_ipc::VideoEncoder::Software,
            _ => ira_overlay_ipc::VideoEncoder::Auto,
        };
        s.cfg.overlay.recording_quality = ira_overlay_ipc::RecordingQuality::from_u32(overlay_widgets.quality_row.selected());
        s.cfg.overlay.toggle_hotkey = overlay_widgets.toggle_hotkey.kb_value.borrow().clone();
        s.cfg.overlay.screenshot_hotkey = overlay_widgets.screenshot_hotkey.kb_value.borrow().clone();
        s.cfg.overlay.record_hotkey = overlay_widgets.record_hotkey.kb_value.borrow().clone();
        s.cfg.overlay.toggle_hotkey_gamepad = overlay_widgets.toggle_hotkey.gp_value.borrow().clone();
        s.cfg.overlay.screenshot_hotkey_gamepad = overlay_widgets.screenshot_hotkey.gp_value.borrow().clone();
        s.cfg.overlay.record_hotkey_gamepad = overlay_widgets.record_hotkey.gp_value.borrow().clone();
        if let Some(desc) = overlay_widgets.font_button.font_desc() {
            let family_str: String = desc.family().map(|s| s.to_string()).unwrap_or_default();
            if family_str.is_empty() {
                s.cfg.overlay.font_family = None;
            } else {
                s.cfg.overlay.font_family = Some(family_str);
            }
        } else {
            s.cfg.overlay.font_family = None;
        }

        s.cfg.default_system.gamemode = system_defaults_widgets.gamemode.is_active();
        s.cfg.default_system.mangohud = system_defaults_widgets.mangohud.is_active();
        s.cfg.default_system.gamescope = system_defaults_widgets.gamescope.is_active();
        s.cfg.default_system.gamescope_flags = system_defaults_widgets.gamescope_flags.text().to_string();
        s.cfg.default_system.gamescope_w = system_defaults_widgets.gamescope_w.value() as u32;
        s.cfg.default_system.gamescope_h = system_defaults_widgets.gamescope_h.value() as u32;
        s.cfg.default_system.gamescope_fps = system_defaults_widgets.gamescope_fps.value() as u32;
        s.cfg.default_system.gamescope_upscaling = {
            let upscale_values = ["linear", "fsr", "nis", "integer", "nearest"];
            let idx = system_defaults_widgets.gamescope_upscaling_row.selected() as usize;
            upscale_values.get(idx).copied().unwrap_or("").to_string()
        };
        s.cfg.default_system.env_vars = super::wine_config_env_dll::collect_env_vars(&system_defaults_widgets.env_vars_box);
        s.cfg.default_system.ld_preload = system_defaults_widgets.ld_preload.text().to_string();
        s.cfg.default_system.ld_library_path = system_defaults_widgets.ld_library_path.text().to_string();

        for (source_id, state) in &source_overlay_states {
            match *state.borrow() {
                Some(v) => { s.cfg.overlay.source_overrides.insert(source_id.clone(), v); }
                None => { s.cfg.overlay.source_overrides.remove(source_id); }
            }
        }
        for (source_id, state) in &source_gamescope_states {
            match *state.borrow() {
                Some(v) => { s.cfg.overlay.source_gamescope.insert(source_id.clone(), v); }
                None => { s.cfg.overlay.source_gamescope.remove(source_id); }
            }
        }

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
        s.cfg.prefix_base_dir = prefix_base_row.text().to_string();

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
