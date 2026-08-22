use super::css::*;
use super::helpers::dialog_layout;
use super::input_profile_settings::{ConsoleProfileWidgets, ControllerDefaultWidgets};
use super::input_profile_store::ensure_controller_default_profile;
use super::settings_dialog_console::{
    apply_console_settings, apply_emulator_settings, apply_override_states,
    discovery_settings_changed, register_console_pages, SharedConsoleSettingsWidgets,
};
use super::settings_dialog_pages::{build_settings_pages, register_settings_pages};
use super::settings_pages::{AutoReloadWidgets, OverlayPageWidgets, SystemDefaultsWidgets};
use super::state::SharedState;
use super::system_settings::{build_override_switch_row, OverrideState};
use super::wine_config_widget::WineConfigWidgets;
use adw::prelude::*;
use ira_api::SteamDataClient;
use ira_config::{Config, ControllerInputConfig};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

pub(super) fn settings_page_container() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b
}

// Re-exports for backward compatibility with files that use super::settings_dialog::*
pub(super) use super::settings_pages::{settings_sidebar_row, sidebar_separator};

struct SavedSettingsWidgets {
    steam_entry: adw::PasswordEntryRow,
    sgdb_entry: adw::PasswordEntryRow,
    notif_row: adw::SwitchRow,
    bg_row: adw::SwitchRow,
    hidden_row: adw::SwitchRow,
    saves_row: adw::SwitchRow,
    auto_reload_widgets: AutoReloadWidgets,
    steam_enable_row: adw::SwitchRow,
    default_game_folder_row: adw::EntryRow,
    roms_folder_row: adw::EntryRow,
    lang_list: gtk4::ListBox,
    ra_enable_row: adw::SwitchRow,
    ra_username_row: adw::EntryRow,
    ra_web_api_key_row: adw::EntryRow,
    controller_default_widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    overlay_widgets: OverlayPageWidgets,
    system_defaults_widgets: SystemDefaultsWidgets,
    console_pages: SharedConsoleSettingsWidgets,
    wine_widgets: WineConfigWidgets,
    linux_controller_profile: ConsoleProfileWidgets,
    wine_controller_profile: ConsoleProfileWidgets,
    prefix_base_row: adw::EntryRow,
    default_version_row: adw::ComboRow,
    default_version_values: Vec<(String, String)>,
    emu_version_row: adw::ComboRow,
    emu_version_model: gtk4::StringList,
}

struct SettingsDialogParams {
    win: adw::Window,
    sidebar: gtk4::ListBox,
    stack: gtk4::Stack,
    content_area: gtk4::Box,
    cfg: Config,
    steam: Arc<SteamDataClient>,
    state: SharedState,
    rom_platforms_with_games: HashSet<String>,
}

pub fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamDataClient>,
    state: &SharedState,
) {
    let layout = dialog_layout(parent);
    layout.window.set_deletable(false);
    layout.sidebar_area.set_size_request(180, -1);

    let loading = adw::StatusPage::new();
    loading.set_title(&crate::tr!("Loading Settings"));
    loading.set_description(Some(&crate::tr!(
        "Checking installed emulators and controller profiles"
    )));
    let spinner = gtk4::Spinner::new();
    spinner.start();
    loading.set_child(Some(&spinner));
    layout.stack.add_named(&loading, Some("loading"));
    layout.stack.set_visible_child_name("loading");
    layout.window.present();

    let rom_platforms_with_games = {
        let state = state.borrow();
        state
            .games
            .iter()
            .map(|game| game.platform_id.clone())
            .collect()
    };
    let params = SettingsDialogParams {
        win: layout.window,
        sidebar: layout.sidebar,
        stack: layout.stack,
        content_area: layout.content_area,
        cfg,
        steam,
        state: state.clone(),
        rom_platforms_with_games,
    };
    glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
        finish_settings_dialog(params)
    });
}

fn finish_settings_dialog(params: SettingsDialogParams) {
    let SettingsDialogParams {
        win,
        sidebar,
        stack,
        content_area,
        cfg,
        steam,
        state,
        rom_platforms_with_games,
    } = params;
    let pages = build_settings_pages(&cfg, &win, &state);
    register_settings_pages(&pages, &sidebar, &stack);
    let steam_page = pages.steam_page.clone();
    let ra_page = pages.ra_page.clone();

    let mut source_overlay_states: Vec<(String, OverrideState)> = Vec::new();
    let mut source_gamescope_states: Vec<(String, OverrideState)> = Vec::new();

    {
        let (overlay_row, state) = build_override_switch_row(
            &crate::tr!("In-game overlay"),
            &crate::tr!("Achievements, screenshots, and recording"),
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("steam").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            &crate::tr!("Gamescope"),
            &crate::tr!("Valve Gamescope compositor"),
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
            &crate::tr!("In-game overlay"),
            &crate::tr!("Achievements, screenshots, and recording"),
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("ra").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            &crate::tr!("Gamescope"),
            &crate::tr!("Valve Gamescope compositor"),
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

    let console_pages = register_console_pages(
        &cfg,
        &win,
        &state,
        &sidebar,
        &stack,
        rom_platforms_with_games,
    );
    {
        let mut console_pages = console_pages.borrow_mut();
        console_pages
            .source_overlay_states
            .append(&mut source_overlay_states);
        console_pages
            .source_gamescope_states
            .append(&mut source_gamescope_states);
    }
    let saved_widgets = SavedSettingsWidgets {
        steam_entry: pages.steam_entry,
        sgdb_entry: pages.sgdb_entry,
        notif_row: pages.notif_row,
        bg_row: pages.bg_row,
        hidden_row: pages.hidden_row,
        saves_row: pages.saves_row,
        auto_reload_widgets: pages.auto_reload_widgets,
        steam_enable_row: pages.steam_enable_row,
        default_game_folder_row: pages.default_game_folder_row,
        roms_folder_row: pages.roms_folder_row,
        lang_list: pages.lang_list,
        ra_enable_row: pages.ra_enable_row,
        ra_username_row: pages.ra_username_row,
        ra_web_api_key_row: pages.ra_web_api_key_row,
        controller_default_widgets: pages.controller_default_widgets,
        overlay_widgets: pages.overlay_widgets,
        system_defaults_widgets: pages.system_defaults_widgets,
        console_pages,
        wine_widgets: pages.wine_widgets,
        linux_controller_profile: pages.linux_controller_profile,
        wine_controller_profile: pages.wine_controller_profile,
        prefix_base_row: pages.prefix_base_row,
        default_version_row: pages.default_version_row,
        default_version_values: pages.default_version_values,
        emu_version_row: pages.emu_version_row,
        emu_version_model: pages.emu_version_model,
    };

    let stack_clone = stack;
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let page_id = row.widget_name().to_string();
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

    let cancel_btn = gtk4::Button::with_label(&crate::tr!("Cancel"));
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());
    let win_c = win.clone();
    let cancel_shortcut = gtk4::EventControllerKey::new();
    cancel_shortcut.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            win_c.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    win.add_controller(cancel_shortcut);

    let save_btn = gtk4::Button::with_label(&crate::tr!("Save"));
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);

    let state_clone = state;
    let win_clone = win;
    let steam_clone = steam;
    save_btn.connect_clicked(move |_| {
        let mut s = state_clone.borrow_mut();
        let old_cfg = s.cfg.clone();
        apply_saved_settings(&mut s.cfg, &saved_widgets);

        steam_clone.update_keys(&s.cfg.steam_api_key, &s.cfg.steam_griddb_api_key);

        let reload_games = discovery_settings_changed(&old_cfg, &s.cfg);
        let cfg = s.cfg.clone();
        let sender = s.sender.clone();
        drop(s);

        if let Err(e) = cfg.ensure_rom_folders() {
            eprintln!("Failed to create ROM library folders: {e}");
        }

        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        if old_cfg.shadps4_executable != cfg.shadps4_executable
            || old_cfg.rpcs3_executable != cfg.rpcs3_executable
        {
            crate::activate::refresh_playtime_watchers(&state_clone);
        }
        if reload_games {
            let _ = sender.send(crate::AppMessage::ReloadGames);
        }
        win_clone.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);
    if let Some(row) = sidebar.selected_row().filter(|row| row.parent().is_some()) {
        row.grab_focus();
    }
}

fn apply_saved_settings(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    apply_general_settings(cfg, widgets);
    let console_pages = widgets.console_pages.borrow();
    apply_emulator_settings(cfg, &console_pages);
    apply_controller_defaults(cfg, &widgets.controller_default_widgets);
    apply_overlay_settings(cfg, &widgets.overlay_widgets);
    apply_system_defaults(cfg, &widgets.system_defaults_widgets);
    apply_override_states(
        cfg,
        &console_pages.source_overlay_states,
        &console_pages.source_gamescope_states,
    );
    apply_console_settings(cfg, &console_pages);
    apply_profile_settings(cfg, widgets);
    apply_api_emulator_version(cfg, widgets);
}

fn apply_general_settings(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    cfg.steam_api_key = widgets.steam_entry.text().to_string();
    cfg.steam_griddb_api_key = widgets.sgdb_entry.text().to_string();
    cfg.notifications_enabled = widgets.notif_row.is_active();
    cfg.close_to_background = widgets.bg_row.is_active();
    cfg.show_hidden_games = widgets.hidden_row.is_active();
    cfg.centralize_game_saves = widgets.saves_row.is_active();
    cfg.auto_reload_steam = widgets.auto_reload_widgets.steam.is_active();
    cfg.auto_reload_roms = widgets.auto_reload_widgets.roms.is_active();
    cfg.auto_reload_shadps4 = widgets.auto_reload_widgets.shadps4.is_active();
    cfg.auto_reload_rpcs3 = widgets.auto_reload_widgets.rpcs3.is_active();
    cfg.auto_reload_vita3k = widgets.auto_reload_widgets.vita3k.is_active();
    cfg.auto_reload_cemu = widgets.auto_reload_widgets.cemu.is_active();
    cfg.steam_enabled = widgets.steam_enable_row.is_active();
    cfg.default_game_folder = widgets.default_game_folder_row.text().to_string();
    cfg.roms_folder = widgets.roms_folder_row.text().to_string();
    cfg.language_preferences = super::settings_pages::read_language_preferences(&widgets.lang_list);
    cfg.ra_enabled = widgets.ra_enable_row.is_active();
    cfg.ra_username = widgets.ra_username_row.text().to_string();
    cfg.ra_web_api_key = widgets.ra_web_api_key_row.text().to_string();
}

fn apply_controller_defaults(
    cfg: &mut Config,
    widgets: &Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
) {
    let mut controller_defaults = cfg.controller_defaults.clone();
    for widget in widgets.borrow().iter() {
        let mut profile = widget
            .profile_path
            .borrow()
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mode = super::input_profile_settings::mode_from_selection(widget.mode.selected());
        if mode != ira_models::ControllerInputMode::Disabled && profile.is_empty() {
            match ensure_controller_default_profile(
                &cfg.save_dir,
                &widget.key,
                &widget.device_name,
                &widget.supported_buttons,
                super::helpers::backend_for_mode(mode),
            ) {
                Ok(path) => profile = path.to_string_lossy().into_owned(),
                Err(error) => eprintln!("Failed to create controller mapping: {error}"),
            }
        }
        if mode != ira_models::ControllerInputMode::Disabled || !profile.is_empty() {
            controller_defaults.insert(widget.key.clone(), ControllerInputConfig { mode, profile });
        } else {
            controller_defaults.remove(&widget.key);
        }
    }
    cfg.controller_defaults = controller_defaults;
}

fn apply_overlay_settings(cfg: &mut Config, widgets: &OverlayPageWidgets) {
    cfg.overlay.enabled = widgets.enable_row.is_active();
    cfg.overlay.replay_buffer_enabled = widgets.replay_buffer_row.is_active();
    cfg.overlay.replay_buffer_seconds = ira_overlay_ipc::clamp_replay_buffer_seconds(
        (widgets.replay_duration_row.value().round() as u32).saturating_mul(60),
    );
    cfg.overlay.encoder = match widgets.encoder_row.selected() {
        1 => ira_overlay_ipc::VideoEncoder::Vaapi,
        2 => ira_overlay_ipc::VideoEncoder::Nvenc,
        3 => ira_overlay_ipc::VideoEncoder::Software,
        _ => ira_overlay_ipc::VideoEncoder::Auto,
    };
    cfg.overlay.recording_quality =
        ira_overlay_ipc::RecordingQuality::from_u32(widgets.quality_row.selected());
    cfg.overlay.toggle_hotkey = widgets.toggle_hotkey.kb_value.borrow().clone();
    cfg.overlay.screenshot_hotkey = widgets.screenshot_hotkey.kb_value.borrow().clone();
    cfg.overlay.record_hotkey = widgets.record_hotkey.kb_value.borrow().clone();
    cfg.overlay.toggle_hotkey_gamepad = widgets.toggle_hotkey.gp_value.borrow().clone();
    cfg.overlay.screenshot_hotkey_gamepad = widgets.screenshot_hotkey.gp_value.borrow().clone();
    cfg.overlay.record_hotkey_gamepad = widgets.record_hotkey.gp_value.borrow().clone();
    cfg.overlay.font_family = widgets
        .font_button
        .font_desc()
        .and_then(|desc| desc.family().map(|family| family.to_string()))
        .filter(|family| !family.is_empty());
}

fn apply_system_defaults(cfg: &mut Config, widgets: &SystemDefaultsWidgets) {
    cfg.default_system.gamemode = widgets.gamemode.is_active();
    cfg.default_system.mangohud = widgets.mangohud.is_active();
    cfg.default_system.gamescope = widgets.gamescope.is_active();
    cfg.default_system.gamescope_flags = widgets.gamescope_flags.text().to_string();
    cfg.default_system.gamescope_w = widgets.gamescope_w.value() as u32;
    cfg.default_system.gamescope_h = widgets.gamescope_h.value() as u32;
    cfg.default_system.gamescope_fps = widgets.gamescope_fps.value() as u32;
    cfg.default_system.gamescope_upscaling = {
        let upscale_values = ["linear", "fsr", "nis", "integer", "nearest"];
        let idx = widgets.gamescope_upscaling_row.selected() as usize;
        upscale_values.get(idx).copied().unwrap_or("").to_string()
    };
    cfg.default_system.env_vars =
        super::wine_config_env_dll::collect_env_vars(&widgets.env_vars_box);
    cfg.default_system.ld_preload = widgets.ld_preload.text().to_string();
    cfg.default_system.ld_library_path = widgets.ld_library_path.text().to_string();
    cfg.default_system.gpu = widgets.gpu_row.borrow().as_ref().map_or_else(
        || widgets.gpu_default.borrow().clone(),
        |row| match row.selected() as usize {
            0 => String::new(),
            idx => widgets
                .gpu_options
                .borrow()
                .get(idx - 1)
                .cloned()
                .unwrap_or_default(),
        },
    );
}

fn apply_profile_settings(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    cfg.default_wine_config = widgets.wine_widgets.to_wine_config();
    cfg.default_wine_config.version = widgets
        .default_version_values
        .get(widgets.default_version_row.selected() as usize)
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    cfg.linux_controller_profile = widgets
        .linux_controller_profile
        .profile_path
        .borrow()
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    cfg.linux_controller_mode = *widgets.linux_controller_profile.mode.borrow();
    cfg.wine_controller_profile = widgets
        .wine_controller_profile
        .profile_path
        .borrow()
        .as_ref()
        .cloned()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    cfg.wine_controller_mode = *widgets.wine_controller_profile.mode.borrow();
    cfg.prefix_base_dir = widgets.prefix_base_row.text().to_string();
}

fn apply_api_emulator_version(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    let ver = widgets
        .emu_version_model
        .string(widgets.emu_version_row.selected())
        .map(|value| value.to_string())
        .unwrap_or_default();
    if !ver.is_empty() && !ver.starts_with("(no versions") {
        cfg.default_api_emu_version = ver;
    }
}
