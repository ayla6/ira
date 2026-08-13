use super::css::*;
use super::helpers::dialog_layout;
use super::input_profile_settings::{
    add_console_profile_group, add_pc_profile_group, build_input_settings_page,
    ConsoleProfileWidgets, ControllerDefaultWidgets,
};
use super::input_profile_store::ensure_controller_default_profile;
use super::profile_dialog::build_profiles_page;
use super::settings_console::{
    build_cemu_settings_page, build_console_settings_page, build_rpcs3_settings_page,
    build_shadps4_settings_page, build_vita3k_settings_page, ConsolePageWidgets,
};
use super::settings_pages::{
    build_api_emulators_page, build_computer_games_page, build_general_settings_page,
    build_lutris_settings_page, build_overlay_settings_page, build_ra_settings_page,
    build_rom_settings_page, build_steam_settings_page, build_system_defaults_page,
    OverlayPageWidgets, SystemDefaultsWidgets,
};
use super::state::SharedState;
use super::system_settings::{build_override_switch_row, OverrideState};
use super::wine_config_widget::{build_wine_config_pages, WineConfigWidgets, WinePage};
use crate::strings as S;
use adw::prelude::*;
use ira_api::SteamDataClient;
use ira_config::{Config, ConsoleConfig, ControllerInputConfig};
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
pub(super) use super::settings_pages::{
    settings_sidebar_row, sidebar_section_title, sidebar_separator,
};

struct ConsoleSettingsWidgets {
    console_widgets: Vec<(&'static str, ConsolePageWidgets)>,
    console_profile_widgets: Vec<ConsoleProfileWidgets>,
    source_overlay_states: Vec<(String, OverrideState)>,
    source_gamescope_states: Vec<(String, OverrideState)>,
    ps4_enable_row: Option<adw::SwitchRow>,
    ps4_version_dd: Option<adw::ComboRow>,
    ps3_enable_row: Option<adw::SwitchRow>,
    ps3_exe_row: Option<adw::EntryRow>,
    vita3k_enable_row: Option<adw::SwitchRow>,
    vita3k_exe_row: Option<adw::EntryRow>,
    cemu_enable_row: Option<adw::SwitchRow>,
    cemu_exe_row: Option<adw::EntryRow>,
}

struct SavedSettingsWidgets {
    steam_entry: adw::PasswordEntryRow,
    sgdb_entry: adw::PasswordEntryRow,
    notif_row: adw::SwitchRow,
    bg_row: adw::SwitchRow,
    hidden_row: adw::SwitchRow,
    saves_row: adw::SwitchRow,
    steam_enable_row: adw::SwitchRow,
    default_game_folder_row: adw::EntryRow,
    roms_folder_row: adw::EntryRow,
    lang_list: gtk4::ListBox,
    ra_enable_row: adw::SwitchRow,
    ra_username_row: adw::EntryRow,
    ra_password_row: adw::EntryRow,
    controller_default_widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    overlay_widgets: OverlayPageWidgets,
    system_defaults_widgets: SystemDefaultsWidgets,
    console_pages: ConsoleSettingsWidgets,
    wine_widgets: WineConfigWidgets,
    linux_controller_profile: ConsoleProfileWidgets,
    wine_controller_profile: ConsoleProfileWidgets,
    prefix_base_row: adw::EntryRow,
    emu_version_row: adw::ComboRow,
    emu_version_model: gtk4::StringList,
}

struct SettingsPageWidgets {
    general_page: gtk4::Box,
    overlay_page: gtk4::Box,
    input_page: gtk4::Box,
    system_page: gtk4::Box,
    computer_games_page: gtk4::Box,
    steam_page: gtk4::Box,
    emu_page: gtk4::Box,
    lutris_page: gtk4::Box,
    wine_pages: Vec<WinePage>,
    profiles_page: gtk4::ScrolledWindow,
    ra_page: gtk4::Box,
    rom_page: gtk4::Box,
    notif_row: adw::SwitchRow,
    bg_row: adw::SwitchRow,
    hidden_row: adw::SwitchRow,
    steam_entry: adw::PasswordEntryRow,
    sgdb_entry: adw::PasswordEntryRow,
    lang_list: gtk4::ListBox,
    saves_row: adw::SwitchRow,
    controller_default_widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    default_game_folder_row: adw::EntryRow,
    steam_enable_row: adw::SwitchRow,
    emu_version_row: adw::ComboRow,
    emu_version_model: gtk4::StringList,
    wine_widgets: WineConfigWidgets,
    prefix_base_row: adw::EntryRow,
    ra_enable_row: adw::SwitchRow,
    ra_username_row: adw::EntryRow,
    ra_password_row: adw::EntryRow,
    roms_folder_row: adw::EntryRow,
    overlay_widgets: OverlayPageWidgets,
    system_defaults_widgets: SystemDefaultsWidgets,
    linux_controller_profile: ConsoleProfileWidgets,
    wine_controller_profile: ConsoleProfileWidgets,
}

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

    let loading = gtk4::Label::new(Some("Loading settings..."));
    loading.set_margin_top(24);
    loading.set_margin_bottom(24);
    layout.stack.add_named(&loading, Some("loading"));
    layout.stack.set_visible_child_name("loading");
    layout.window.present();

    let db = state.borrow().db.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _s = tracing::info_span!("load_settings_game_platforms").entered();
        let result = ira_db::load_all_games(&db).map(|games| {
            games
                .into_iter()
                .map(|game| game.platform_id)
                .collect::<HashSet<_>>()
        });
        let _ = tx.send(result);
    });

    let rx = std::cell::RefCell::new(rx);
    let win = layout.window;
    let sidebar = layout.sidebar;
    let stack = layout.stack;
    let content_area = layout.content_area;
    let state = state.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(Ok(platforms)) => {
                finish_settings_dialog(
                    win.clone(),
                    sidebar.clone(),
                    stack.clone(),
                    content_area.clone(),
                    cfg.clone(),
                    steam.clone(),
                    &state,
                    platforms,
                );
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                eprintln!("Failed to load ROM platforms for settings: {error}");
                finish_settings_dialog(
                    win.clone(),
                    sidebar.clone(),
                    stack.clone(),
                    content_area.clone(),
                    cfg.clone(),
                    steam.clone(),
                    &state,
                    HashSet::new(),
                );
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                eprintln!("Settings game-platform loader disconnected");
                finish_settings_dialog(
                    win.clone(),
                    sidebar.clone(),
                    stack.clone(),
                    content_area.clone(),
                    cfg.clone(),
                    steam.clone(),
                    &state,
                    HashSet::new(),
                );
                glib::ControlFlow::Break
            }
        }
    });
}

fn finish_settings_dialog(
    win: adw::Window,
    sidebar: gtk4::ListBox,
    stack: gtk4::Stack,
    content_area: gtk4::Box,
    cfg: Config,
    steam: Arc<SteamDataClient>,
    state: &SharedState,
    rom_platforms_with_games: HashSet<String>,
) {
    let pages = build_settings_pages(&cfg, &win, state);
    register_settings_pages(&pages, &sidebar, &stack);
    let steam_page = pages.steam_page.clone();
    let ra_page = pages.ra_page.clone();

    let mut source_overlay_states: Vec<(String, OverrideState)> = Vec::new();
    let mut source_gamescope_states: Vec<(String, OverrideState)> = Vec::new();

    {
        let (overlay_row, state) = build_override_switch_row(
            "In-game overlay",
            "Achievements, screenshots, and recording",
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("steam").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            "Gamescope",
            "Valve Gamescope compositor",
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
            "In-game overlay",
            "Achievements, screenshots, and recording",
            cfg.overlay.enabled,
            cfg.overlay.source_overrides.get("ra").copied(),
        );
        let (gs_row, gs_state) = build_override_switch_row(
            "Gamescope",
            "Valve Gamescope compositor",
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

    let mut console_pages = register_console_pages(
        &cfg,
        &win,
        state,
        &sidebar,
        &stack,
        rom_platforms_with_games,
    );
    source_overlay_states.append(&mut console_pages.source_overlay_states);
    source_gamescope_states.append(&mut console_pages.source_gamescope_states);
    let saved_widgets = SavedSettingsWidgets {
        steam_entry: pages.steam_entry,
        sgdb_entry: pages.sgdb_entry,
        notif_row: pages.notif_row,
        bg_row: pages.bg_row,
        hidden_row: pages.hidden_row,
        saves_row: pages.saves_row,
        steam_enable_row: pages.steam_enable_row,
        default_game_folder_row: pages.default_game_folder_row,
        roms_folder_row: pages.roms_folder_row,
        lang_list: pages.lang_list,
        ra_enable_row: pages.ra_enable_row,
        ra_username_row: pages.ra_username_row,
        ra_password_row: pages.ra_password_row,
        controller_default_widgets: pages.controller_default_widgets,
        overlay_widgets: pages.overlay_widgets,
        system_defaults_widgets: pages.system_defaults_widgets,
        console_pages,
        wine_widgets: pages.wine_widgets,
        linux_controller_profile: pages.linux_controller_profile,
        wine_controller_profile: pages.wine_controller_profile,
        prefix_base_row: pages.prefix_base_row,
        emu_version_row: pages.emu_version_row,
        emu_version_model: pages.emu_version_model,
    };

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
    apply_emulator_settings(cfg, &widgets.console_pages);
    apply_controller_defaults(cfg, &widgets.controller_default_widgets);
    apply_overlay_settings(cfg, &widgets.overlay_widgets);
    apply_system_defaults(cfg, &widgets.system_defaults_widgets);
    apply_override_states(
        cfg,
        &widgets.console_pages.source_overlay_states,
        &widgets.console_pages.source_gamescope_states,
    );
    apply_console_settings(cfg, &widgets.console_pages);
    apply_profile_settings(cfg, widgets);
    apply_api_emulator_version(cfg, widgets);
}

fn build_settings_pages(
    cfg: &Config,
    win: &adw::Window,
    state: &SharedState,
) -> SettingsPageWidgets {
    let (
        general_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        sgdb_entry,
        lang_list,
        saves_row,
    ) = build_general_settings_page(cfg);
    let (overlay_page, overlay_widgets) = build_overlay_settings_page(cfg);
    let registry = state.borrow().controller_registry.clone();
    let (input_page, input_widgets) = build_input_settings_page(win, &cfg.save_dir, cfg, registry);
    let controller_default_widgets = input_widgets.controller_defaults.clone();
    let (system_page, system_defaults_widgets) = build_system_defaults_page(cfg);
    let (computer_games_page, default_game_folder_row) = build_computer_games_page(win, cfg);
    let (linux_controller_profile, wine_controller_profile) =
        build_pc_controller_profiles(&computer_games_page, cfg, win, state);
    let (steam_page, steam_enable_row) = build_steam_settings_page(cfg);
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(cfg);
    let lutris_page = build_lutris_settings_page(state, win);
    let (wine_pages, wine_widgets) = build_wine_config_pages(&cfg.default_wine_config, None);
    let (profiles_page, prefix_base_row) = build_profiles_page(state, win);
    let (ra_page, ra_enable_row, ra_username_row, ra_password_row) = build_ra_settings_page(cfg);
    let (rom_page, roms_folder_row) = build_rom_settings_page(win, cfg);
    SettingsPageWidgets {
        general_page,
        overlay_page,
        input_page,
        system_page,
        computer_games_page,
        steam_page,
        emu_page,
        lutris_page,
        wine_pages,
        profiles_page,
        ra_page,
        rom_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        sgdb_entry,
        lang_list,
        saves_row,
        controller_default_widgets,
        default_game_folder_row,
        steam_enable_row,
        emu_version_row,
        emu_version_model,
        wine_widgets,
        prefix_base_row,
        ra_enable_row,
        ra_username_row,
        ra_password_row,
        roms_folder_row,
        overlay_widgets,
        system_defaults_widgets,
        linux_controller_profile,
        wine_controller_profile,
    }
}

fn register_settings_pages(
    pages: &SettingsPageWidgets,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) {
    register_scrolled_page(
        sidebar,
        stack,
        &pages.general_page,
        "preferences-system-symbolic",
        "General",
        "general",
    );
    register_page(
        sidebar,
        stack,
        &pages.overlay_page,
        "view-grid-symbolic",
        "Overlay",
        "overlay",
    );
    register_scrolled_page(
        sidebar,
        stack,
        &pages.input_page,
        "input-gaming-symbolic",
        "Controller",
        "input",
    );
    register_scrolled_page(
        sidebar,
        stack,
        &pages.system_page,
        "applications-science-symbolic",
        "Game system",
        "system",
    );
    sidebar.append(&sidebar_section_title("PC games"));
    register_page(
        sidebar,
        stack,
        &pages.computer_games_page,
        "applications-games-symbolic",
        "PC games",
        "computer_games",
    );
    register_page(
        sidebar,
        stack,
        &pages.steam_page,
        "application-x-executable-symbolic",
        "Steam",
        "steam",
    );
    register_page(
        sidebar,
        stack,
        &pages.emu_page,
        "applications-engineering-symbolic",
        "API emulators",
        "api_emulators",
    );
    register_page(
        sidebar,
        stack,
        &pages.lutris_page,
        "system-software-install-symbolic",
        "Lutris migration",
        "migration",
    );
    sidebar.append(&sidebar_section_title("Wine"));
    register_page(
        sidebar,
        stack,
        &pages.profiles_page,
        "system-users-symbolic",
        "Profiles",
        "profiles",
    );
    for page in &pages.wine_pages {
        register_page(
            sidebar, stack, &page.page, page.icon, page.label, page.label,
        );
    }
    sidebar.append(&sidebar_section_title("Emulation"));
    register_page(
        sidebar,
        stack,
        &pages.ra_page,
        "applications-science-symbolic",
        "RetroAchievements",
        "ra",
    );
    register_page(
        sidebar,
        stack,
        &pages.rom_page,
        "drive-harddisk-symbolic",
        "ROM library",
        "roms",
    );
}

fn register_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    page: &impl IsA<gtk4::Widget>,
    icon: &str,
    label: &str,
    page_id: &str,
) {
    sidebar.append(&settings_sidebar_row(icon, label, page_id));
    stack.add_named(page, Some(page_id));
}

fn register_scrolled_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    page: &gtk4::Box,
    icon: &str,
    label: &str,
    page_id: &str,
) {
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_child(Some(page));
    register_page(sidebar, stack, &scroll, icon, label, page_id);
}

fn apply_general_settings(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    cfg.steam_api_key = widgets.steam_entry.text().to_string();
    cfg.steam_griddb_api_key = widgets.sgdb_entry.text().to_string();
    cfg.notifications_enabled = widgets.notif_row.is_active();
    cfg.close_to_background = widgets.bg_row.is_active();
    cfg.show_hidden_games = widgets.hidden_row.is_active();
    cfg.centralize_game_saves = widgets.saves_row.is_active();
    cfg.steam_enabled = widgets.steam_enable_row.is_active();
    cfg.default_game_folder = widgets.default_game_folder_row.text().to_string();
    cfg.roms_folder = widgets.roms_folder_row.text().to_string();
    cfg.language_preferences = super::settings_pages::read_language_preferences(&widgets.lang_list);
    cfg.ra_enabled = widgets.ra_enable_row.is_active();
    cfg.ra_username = widgets.ra_username_row.text().to_string();
    cfg.ra_password = widgets.ra_password_row.text().to_string();
}

fn apply_emulator_settings(cfg: &mut Config, pages: &ConsoleSettingsWidgets) {
    if let Some(row) = &pages.ps4_enable_row {
        cfg.shadps4_enabled = row.is_active();
    }
    if let Some(dd) = &pages.ps4_version_dd {
        let idx = dd.selected();
        cfg.shadps4_executable = if idx == 0 {
            String::new()
        } else {
            ira_platforms::ps4::read_shadps4_launch_options()
                .into_iter()
                .nth((idx - 1) as usize)
                .map(|v| v.launch_command)
                .unwrap_or_default()
        };
    }
    if let Some(row) = &pages.ps3_enable_row {
        cfg.rpcs3_enabled = row.is_active();
    }
    if let Some(row) = &pages.ps3_exe_row {
        cfg.rpcs3_executable = row.text().to_string();
    }
    if let Some(row) = &pages.vita3k_enable_row {
        cfg.vita3k_enabled = row.is_active();
    }
    if let Some(row) = &pages.vita3k_exe_row {
        cfg.vita3k_executable = row.text().to_string();
    }
    if let Some(row) = &pages.cemu_enable_row {
        cfg.cemu_enabled = row.is_active();
    }
    if let Some(row) = &pages.cemu_exe_row {
        cfg.cemu_executable = row.text().to_string();
    }
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
                match mode {
                    ira_models::ControllerInputMode::VirtualDirectInput => {
                        ira_input::VirtualGamepadBackend::DirectInput
                    }
                    ira_models::ControllerInputMode::Disabled
                    | ira_models::ControllerInputMode::VirtualXInput => {
                        ira_input::VirtualGamepadBackend::XInput
                    }
                },
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

fn apply_override_states(
    cfg: &mut Config,
    overlay_states: &[(String, OverrideState)],
    gamescope_states: &[(String, OverrideState)],
) {
    for (source_id, state) in overlay_states {
        match *state.borrow() {
            Some(value) => {
                cfg.overlay
                    .source_overrides
                    .insert(source_id.clone(), value);
            }
            None => {
                cfg.overlay.source_overrides.remove(source_id);
            }
        }
    }
    for (source_id, state) in gamescope_states {
        match *state.borrow() {
            Some(value) => {
                cfg.overlay
                    .source_gamescope
                    .insert(source_id.clone(), value);
            }
            None => {
                cfg.overlay.source_gamescope.remove(source_id);
            }
        }
    }
}

fn apply_console_settings(cfg: &mut Config, pages: &ConsoleSettingsWidgets) {
    for (console_id, widgets) in &pages.console_widgets {
        let console = cfg.console_mut(console_id);
        console.enabled = widgets.enable_row.is_active();
        console.executable = widgets.exe_row.text().to_string();
        console.fullscreen = widgets.fullscreen_row.is_active();
        if let Some(ref core_path) = widgets.core_path_row {
            console.ra_core = ira_platforms::emulator_detect::resolve_ra_core_for_console(
                console_id,
                &core_path.text(),
            )
            .unwrap_or_default();
        }
    }
    for widget in &pages.console_profile_widgets {
        let console = cfg.console_mut(&widget.console_id);
        console.controller_mode = *widget.mode.borrow();
        console.controller_profile = widget
            .profile_path
            .borrow()
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
}

fn apply_profile_settings(cfg: &mut Config, widgets: &SavedSettingsWidgets) {
    cfg.default_wine_config = widgets.wine_widgets.to_wine_config();
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

fn discovery_settings_changed(before: &Config, after: &Config) -> bool {
    before.steam_enabled != after.steam_enabled
        || before.shadps4_enabled != after.shadps4_enabled
        || before.shadps4_executable != after.shadps4_executable
        || before.rpcs3_enabled != after.rpcs3_enabled
        || before.rpcs3_executable != after.rpcs3_executable
        || before.vita3k_enabled != after.vita3k_enabled
        || before.vita3k_executable != after.vita3k_executable
        || before.cemu_enabled != after.cemu_enabled
        || before.cemu_executable != after.cemu_executable
        || before.roms_folder != after.roms_folder
        || ira_models::all_consoles().any(|def| {
            let before_console = before.console(def.id);
            let after_console = after.console(def.id);
            before_console.enabled != after_console.enabled
                || before_console.executable != after_console.executable
                || before_console.ra_core != after_console.ra_core
        })
}

fn build_pc_controller_profiles(
    page: &gtk4::Box,
    cfg: &Config,
    win: &adw::Window,
    state: &SharedState,
) -> (ConsoleProfileWidgets, ConsoleProfileWidgets) {
    let mut pc_controller_cfg = cfg.clone();
    pc_controller_cfg.consoles.insert(
        "linux".to_string(),
        ConsoleConfig {
            controller_mode: cfg.linux_controller_mode,
            controller_profile: cfg.linux_controller_profile.clone(),
            ..Default::default()
        },
    );
    pc_controller_cfg.consoles.insert(
        "wine".to_string(),
        ConsoleConfig {
            controller_mode: cfg.wine_controller_mode,
            controller_profile: cfg.wine_controller_profile.clone(),
            ..Default::default()
        },
    );
    let registry = state.borrow().controller_registry.clone();
    let linux = add_pc_profile_group(
        page,
        &pc_controller_cfg,
        "linux",
        "Linux controller",
        win,
        registry.clone(),
    );
    let wine = add_pc_profile_group(
        page,
        &pc_controller_cfg,
        "wine",
        "Wine controller",
        win,
        registry,
    );
    (linux, wine)
}

fn register_console_pages(
    cfg: &Config,
    win: &adw::Window,
    state: &SharedState,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    rom_platforms_with_games: HashSet<String>,
) -> ConsoleSettingsWidgets {
    let mut result = ConsoleSettingsWidgets {
        console_widgets: Vec::new(),
        console_profile_widgets: Vec::new(),
        source_overlay_states: Vec::new(),
        source_gamescope_states: Vec::new(),
        ps4_enable_row: None,
        ps4_version_dd: None,
        ps3_enable_row: None,
        ps3_exe_row: None,
        vita3k_enable_row: None,
        vita3k_exe_row: None,
        cemu_enable_row: None,
        cemu_exe_row: None,
    };
    let registry = {
        let state = state.borrow();
        state.controller_registry.clone()
    };
    let mut empty_platforms = Vec::new();

    for def in ira_models::all_consoles() {
        if def.id == "psp" {
            let (page, enable_row, exe_row) = build_rpcs3_settings_page(cfg, win);
            let profile = add_console_page_overrides(
                &page,
                cfg,
                win,
                "ps3",
                "PS3",
                registry.clone(),
                &mut result,
            );
            result.console_profile_widgets.push(profile);
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                "PS3",
                "ps3",
            ));
            stack.add_named(&page, Some("ps3"));
            result.ps3_enable_row = Some(enable_row);
            result.ps3_exe_row = Some(exe_row);

            let (page, enable_row, version_dd) = build_shadps4_settings_page(cfg);
            let profile = add_console_page_overrides(
                &page,
                cfg,
                win,
                "ps4",
                "PS4",
                registry.clone(),
                &mut result,
            );
            result.console_profile_widgets.push(profile);
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                "PS4",
                "ps4",
            ));
            stack.add_named(&page, Some("ps4"));
            result.ps4_enable_row = Some(enable_row);
            result.ps4_version_dd = version_dd;

            let (page, enable_row, exe_row) = build_vita3k_settings_page(cfg, win);
            let profile = add_console_page_overrides(
                &page,
                cfg,
                win,
                "psvita",
                "PS Vita",
                registry.clone(),
                &mut result,
            );
            result.console_profile_widgets.push(profile);
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                "PS Vita",
                "psvita",
            ));
            stack.add_named(&page, Some("psvita"));
            result.vita3k_enable_row = Some(enable_row);
            result.vita3k_exe_row = Some(exe_row);
        }
        if def.id == "wii" {
            let (page, enable_row, exe_row) = build_cemu_settings_page(cfg, win);
            let profile = add_console_page_overrides(
                &page,
                cfg,
                win,
                "wiiu",
                "Wii U",
                registry.clone(),
                &mut result,
            );
            result.console_profile_widgets.push(profile);
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                "Wii U",
                "wiiu",
            ));
            stack.add_named(&page, Some("wiiu"));
            result.cemu_enable_row = Some(enable_row);
            result.cemu_exe_row = Some(exe_row);
        }
        if !def.uses_rom_folder() {
            continue;
        }
        let (page, widgets) = build_console_settings_page(win, def, cfg.console(def.id));
        let profile = add_console_page_overrides(
            &page,
            cfg,
            win,
            def.id,
            def.display_name,
            registry.clone(),
            &mut result,
        );
        result.console_profile_widgets.push(profile);
        let page_id = def.display_name.to_lowercase();
        if rom_platforms_with_games.contains(def.id) {
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                def.display_name,
                &page_id,
            ));
        } else {
            empty_platforms.push((def.display_name.to_string(), page_id.clone()));
        }
        stack.add_named(&page, Some(&page_id));
        result.console_widgets.push((def.id, widgets));
    }
    if !empty_platforms.is_empty() {
        sidebar.append(&sidebar_section_title("Empty platforms"));
        for (label, page_id) in empty_platforms {
            sidebar.append(&settings_sidebar_row(
                "applications-games-symbolic",
                &label,
                &page_id,
            ));
        }
    }
    result
}

fn add_console_page_overrides(
    page: &gtk4::Box,
    cfg: &Config,
    win: &adw::Window,
    console_id: &str,
    label: &str,
    registry: Arc<ira_input::ControllerRegistry>,
    result: &mut ConsoleSettingsWidgets,
) -> ConsoleProfileWidgets {
    let (overlay_row, overlay_state) = build_override_switch_row(
        "In-game overlay",
        "Achievements, screenshots, and recording",
        cfg.overlay.enabled,
        cfg.overlay.source_overrides.get(console_id).copied(),
    );
    let (gamescope_row, gamescope_state) = build_override_switch_row(
        "Gamescope",
        "Valve Gamescope compositor",
        cfg.default_system.gamescope,
        cfg.overlay.source_gamescope.get(console_id).copied(),
    );
    let group = adw::PreferencesGroup::new();
    group.add(&overlay_row);
    group.add(&gamescope_row);
    page.prepend(&group);
    result
        .source_overlay_states
        .push((console_id.to_string(), overlay_state));
    result
        .source_gamescope_states
        .push((console_id.to_string(), gamescope_state));
    add_console_profile_group(page, win, cfg, &cfg.save_dir, console_id, label, registry)
}
