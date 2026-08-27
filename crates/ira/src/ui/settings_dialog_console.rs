use super::input_profile_settings::{add_console_profile_group, ConsoleProfileWidgets};
use super::settings_console::{
    build_azahar_settings_page, build_cemu_settings_page, build_console_settings_page,
    build_rpcs3_settings_page, build_shadps4_settings_page, build_vita3k_settings_page,
    ConsolePageWidgets,
};
use super::settings_pages::{settings_sidebar_row, sidebar_section_title};
use super::state::SharedState;
use super::system_settings::{build_override_switch_row, OverrideState};
use adw::prelude::*;
use ira_config::Config;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
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
    pub(super) azahar_enable_row: Option<adw::SwitchRow>,
    pub(super) azahar_exe_row: Option<adw::EntryRow>,
}

pub(super) type SharedConsoleSettingsWidgets = Rc<RefCell<ConsoleSettingsWidgets>>;

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
    if let Some(row) = &pages.azahar_enable_row {
        cfg.azahar_enabled = row.is_active();
    }
    if let Some(row) = &pages.azahar_exe_row {
        cfg.azahar_executable = row.text().to_string();
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
    win: &adw::Dialog,
    state: &SharedState,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    rom_platforms_with_games: HashSet<String>,
) -> SharedConsoleSettingsWidgets {
    let result = ConsoleSettingsWidgets {
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
        azahar_enable_row: None,
        azahar_exe_row: None,
    };
    let registry = {
        let state = state.borrow();
        state.controller_registry.clone()
    };
    let mut empty_platforms = Vec::new();

    for def in ira_models::all_consoles() {
        if def.id == "psp" {
            register_lazy_console_page(sidebar, stack, &crate::tr!("PS3"), "ps3");
            register_lazy_console_page(sidebar, stack, &crate::tr!("PS4"), "ps4");
            register_lazy_console_page(sidebar, stack, &crate::tr!("PS Vita"), "psvita");
        }
        if def.id == "wii" {
            register_lazy_console_page(sidebar, stack, &crate::tr!("Wii U"), "wiiu");
        }
        if def.id == "gc" {
            register_lazy_console_page(sidebar, stack, &crate::tr!("Nintendo 3DS"), "3ds");
        }
        if !def.uses_rom_folder() {
            continue;
        }
        let page_id = def.display_name.to_lowercase();
        if rom_platforms_with_games.contains(def.id) {
            sidebar.append(&settings_sidebar_row(
                "games-symbolic",
                def.display_name,
                &page_id,
            ));
        } else {
            empty_platforms.push((def.display_name.to_string(), page_id.clone()));
        }
        add_console_loading_page(stack, &page_id);
    }
    if !empty_platforms.is_empty() {
        sidebar.append(&sidebar_section_title(&crate::tr!("Empty platforms")));
        for (label, page_id) in empty_platforms {
            sidebar.append(&settings_sidebar_row("games-symbolic", &label, &page_id));
        }
    }
    let result = Rc::new(RefCell::new(result));
    connect_lazy_console_pages(&result, cfg, win, sidebar, stack, registry, state);
    result
}

fn register_lazy_console_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    label: &str,
    page_id: &str,
) {
    sidebar.append(&settings_sidebar_row("games-symbolic", label, page_id));
    add_console_loading_page(stack, page_id);
}

fn add_console_loading_page(stack: &gtk4::Stack, page_id: &str) {
    let loading = adw::StatusPage::new();
    loading.set_title(&crate::tr!("Loading emulator settings"));
    let spinner = gtk4::Spinner::new();
    spinner.start();
    loading.set_child(Some(&spinner));
    stack.add_named(&loading, Some(page_id));
}

fn connect_lazy_console_pages(
    result: &SharedConsoleSettingsWidgets,
    cfg: &Config,
    win: &adw::Dialog,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    registry: Arc<ira_input::ControllerRegistry>,
    state: &SharedState,
) {
    let result = result.clone();
    let cfg = cfg.clone();
    let win = win.clone();
    let stack = stack.clone();
    let state = state.clone();
    sidebar.connect_row_selected(move |_, row| {
        let Some(page_id) = row.map(|row| row.widget_name().to_string()) else {
            return;
        };
        if load_special_console_page(&page_id, &cfg, &win, &stack, &registry, &result, &state) {
            return;
        }
        let Some(def) = ira_models::all_consoles()
            .find(|def| def.uses_rom_folder() && def.display_name.to_lowercase() == page_id)
        else {
            return;
        };
        if result
            .borrow()
            .console_widgets
            .iter()
            .any(|(console_id, _)| *console_id == def.id)
        {
            return;
        }

        if let Some(loading) = stack.child_by_name(&page_id) {
            stack.remove(&loading);
        }
        let (page, widgets) = build_console_settings_page(&win, def, cfg.console(def.id));
        let profile = add_console_page_overrides(
            &page,
            &cfg,
            &win,
            def.id,
            def.display_name,
            registry.clone(),
            &mut result.borrow_mut(),
        );
        super::open_emulator_row::add_open_emulator_row(
            &page,
            &state,
            def.display_name,
            def.id.to_string(),
            {
                let exe_row = widgets.exe_row.clone();
                move || exe_row.text().to_string()
            },
            &profile,
        );
        let mut result = result.borrow_mut();
        result.console_profile_widgets.push(profile);
        result.console_widgets.push((def.id, widgets));
        wrap_console_page(&stack, &page, &page_id);
        stack.set_visible_child_name(&page_id);
    });
}

fn load_special_console_page(
    page_id: &str,
    cfg: &Config,
    win: &adw::Dialog,
    stack: &gtk4::Stack,
    registry: &Arc<ira_input::ControllerRegistry>,
    result: &SharedConsoleSettingsWidgets,
    state: &SharedState,
) -> bool {
    if special_console_loaded(&result.borrow(), page_id) {
        return true;
    }
    let Some((console_id, label, page, widgets)) = build_special_console_page(page_id, cfg, win)
    else {
        return false;
    };
    let exe_source = special_exe_source(&widgets, cfg);

    let mut result = result.borrow_mut();
    let profile = add_console_page_overrides(
        &page,
        cfg,
        win,
        console_id,
        &label,
        registry.clone(),
        &mut result,
    );
    super::open_emulator_row::add_open_emulator_row(
        &page,
        state,
        &label,
        console_id.to_string(),
        exe_source,
        &profile,
    );
    result.console_profile_widgets.push(profile);
    store_special_console_widgets(&mut result, widgets);
    drop(result);
    replace_loading_page(stack, &page, console_id);
    true
}

/// Resolves the executable shown on the special page's widgets at click time.
/// For shadPS4 the version dropdown decides: index 0 means the saved global
/// default (resolved like `apply_emulator_settings` would save it).
fn special_exe_source(widgets: &SpecialConsoleWidgets, cfg: &Config) -> Box<dyn Fn() -> String> {
    match widgets {
        SpecialConsoleWidgets::Ps4(_, version_dd) => {
            let dd = version_dd.clone();
            let saved_default = cfg.shadps4_executable.clone();
            Box::new(move || {
                let selected = dd.as_ref().map(|row| row.selected()).unwrap_or(0);
                let chosen = if selected == 0 {
                    saved_default.clone()
                } else {
                    ira_platforms::ps4::read_shadps4_launch_options()
                        .into_iter()
                        .nth(selected as usize - 1)
                        .map(|version| version.launch_command)
                        .unwrap_or_default()
                };
                ira_platforms::ps4::resolve_shadps4_executable("", &chosen)
            })
        }
        SpecialConsoleWidgets::Ps3(_, exe_row)
        | SpecialConsoleWidgets::Vita3k(_, exe_row)
        | SpecialConsoleWidgets::Cemu(_, exe_row)
        | SpecialConsoleWidgets::Azahar(_, exe_row) => {
            let exe_row = exe_row.clone();
            Box::new(move || exe_row.text().to_string())
        }
    }
}

enum SpecialConsoleWidgets {
    Ps3(adw::SwitchRow, adw::EntryRow),
    Ps4(adw::SwitchRow, Option<adw::ComboRow>),
    Vita3k(adw::SwitchRow, adw::EntryRow),
    Cemu(adw::SwitchRow, adw::EntryRow),
    Azahar(adw::SwitchRow, adw::EntryRow),
}

fn build_special_console_page(
    page_id: &str,
    cfg: &Config,
    win: &adw::Dialog,
) -> Option<(&'static str, String, gtk4::Box, SpecialConsoleWidgets)> {
    match page_id {
        "ps3" => {
            let (page, enable_row, exe_row) = build_rpcs3_settings_page(cfg, win);
            Some((
                "ps3",
                crate::tr!("PS3"),
                page,
                SpecialConsoleWidgets::Ps3(enable_row, exe_row),
            ))
        }
        "ps4" => {
            let (page, enable_row, version_dd) = build_shadps4_settings_page(cfg);
            Some((
                "ps4",
                crate::tr!("PS4"),
                page,
                SpecialConsoleWidgets::Ps4(enable_row, version_dd),
            ))
        }
        "psvita" => {
            let (page, enable_row, exe_row) = build_vita3k_settings_page(cfg, win);
            Some((
                "psvita",
                crate::tr!("PS Vita"),
                page,
                SpecialConsoleWidgets::Vita3k(enable_row, exe_row),
            ))
        }
        "wiiu" => {
            let (page, enable_row, exe_row) = build_cemu_settings_page(cfg, win);
            Some((
                "wiiu",
                crate::tr!("Wii U"),
                page,
                SpecialConsoleWidgets::Cemu(enable_row, exe_row),
            ))
        }
        "3ds" => {
            let (page, enable_row, exe_row) = build_azahar_settings_page(cfg, win);
            Some((
                "3ds",
                crate::tr!("Nintendo 3DS"),
                page,
                SpecialConsoleWidgets::Azahar(enable_row, exe_row),
            ))
        }
        _ => None,
    }
}

fn special_console_loaded(widgets: &ConsoleSettingsWidgets, console_id: &str) -> bool {
    match console_id {
        "ps3" => widgets.ps3_enable_row.is_some(),
        "ps4" => widgets.ps4_enable_row.is_some(),
        "psvita" => widgets.vita3k_enable_row.is_some(),
        "wiiu" => widgets.cemu_enable_row.is_some(),
        "3ds" => widgets.azahar_enable_row.is_some(),
        _ => false,
    }
}

fn store_special_console_widgets(
    settings: &mut ConsoleSettingsWidgets,
    widgets: SpecialConsoleWidgets,
) {
    match widgets {
        SpecialConsoleWidgets::Ps3(enable_row, exe_row) => {
            settings.ps3_enable_row = Some(enable_row);
            settings.ps3_exe_row = Some(exe_row);
        }
        SpecialConsoleWidgets::Ps4(enable_row, version_dd) => {
            settings.ps4_enable_row = Some(enable_row);
            settings.ps4_version_dd = version_dd;
        }
        SpecialConsoleWidgets::Vita3k(enable_row, exe_row) => {
            settings.vita3k_enable_row = Some(enable_row);
            settings.vita3k_exe_row = Some(exe_row);
        }
        SpecialConsoleWidgets::Cemu(enable_row, exe_row) => {
            settings.cemu_enable_row = Some(enable_row);
            settings.cemu_exe_row = Some(exe_row);
        }
        SpecialConsoleWidgets::Azahar(enable_row, exe_row) => {
            settings.azahar_enable_row = Some(enable_row);
            settings.azahar_exe_row = Some(exe_row);
        }
    }
}

fn replace_loading_page(stack: &gtk4::Stack, page: &gtk4::Box, page_id: &str) {
    if let Some(loading) = stack.child_by_name(page_id) {
        stack.remove(&loading);
    }
    wrap_console_page(stack, page, page_id);
    stack.set_visible_child_name(page_id);
}

fn wrap_console_page(stack: &gtk4::Stack, page: &gtk4::Box, page_id: &str) {
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_child(Some(page));
    stack.add_named(&scroll, Some(page_id));
}

fn add_console_page_overrides(
    page: &gtk4::Box,
    cfg: &Config,
    win: &adw::Dialog,
    console_id: &str,
    label: &str,
    registry: Arc<ira_input::ControllerRegistry>,
    result: &mut ConsoleSettingsWidgets,
) -> ConsoleProfileWidgets {
    let (overlay_row, overlay_state) = build_override_switch_row(
        &crate::tr!("In-game overlay"),
        &crate::tr!("Achievements, screenshots, and recording"),
        cfg.overlay.enabled,
        cfg.overlay.source_overrides.get(console_id).copied(),
    );
    let (gamescope_row, gamescope_state) = build_override_switch_row(
        &crate::tr!("Gamescope"),
        &crate::tr!("Valve Gamescope compositor"),
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
        || before.azahar_enabled != after.azahar_enabled
        || before.azahar_executable != after.azahar_executable
        || before.roms_folder != after.roms_folder
        || before.extra_roms_folders != after.extra_roms_folders
        || ira_models::all_consoles().any(|def| {
            let before_console = before.console(def.id);
            let after_console = after.console(def.id);
            before_console.enabled != after_console.enabled
                || before_console.executable != after_console.executable
                || before_console.ra_core != after_console.ra_core
        })
}
