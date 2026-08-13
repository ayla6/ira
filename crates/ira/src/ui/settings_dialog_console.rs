use super::input_profile_settings::{add_console_profile_group, ConsoleProfileWidgets};
use super::settings_console::{
    build_cemu_settings_page, build_console_settings_page, build_rpcs3_settings_page,
    build_shadps4_settings_page, build_vita3k_settings_page, ConsolePageWidgets,
};
use super::settings_pages::{settings_sidebar_row, sidebar_section_title};
use super::state::SharedState;
use super::system_settings::{build_override_switch_row, OverrideState};
use adw::prelude::*;
use ira_config::Config;
use std::collections::HashSet;
use std::sync::Arc;

pub(super) struct ConsoleSettingsWidgets {
    pub(super) console_widgets: Vec<(&'static str, ConsolePageWidgets)>,
    pub(super) console_profile_widgets: Vec<ConsoleProfileWidgets>,
    pub(super) source_overlay_states: Vec<(String, OverrideState)>,
    pub(super) source_gamescope_states: Vec<(String, OverrideState)>,
    pub(super) ps4_enable_row: Option<adw::SwitchRow>,
    pub(super) ps4_version_dd: Option<adw::ComboRow>,
    pub(super) ps3_enable_row: Option<adw::SwitchRow>,
    pub(super) ps3_exe_row: Option<adw::EntryRow>,
    pub(super) vita3k_enable_row: Option<adw::SwitchRow>,
    pub(super) vita3k_exe_row: Option<adw::EntryRow>,
    pub(super) cemu_enable_row: Option<adw::SwitchRow>,
    pub(super) cemu_exe_row: Option<adw::EntryRow>,
}

pub(super) fn apply_emulator_settings(cfg: &mut Config, pages: &ConsoleSettingsWidgets) {
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

pub(super) fn apply_override_states(
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

pub(super) fn apply_console_settings(cfg: &mut Config, pages: &ConsoleSettingsWidgets) {
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

pub(super) fn register_console_pages(
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

pub(super) fn discovery_settings_changed(before: &Config, after: &Config) -> bool {
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
