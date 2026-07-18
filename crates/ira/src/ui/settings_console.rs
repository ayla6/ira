use gtk4::prelude::*;
use adw::prelude::*;
use ira_config::{Config, ConsoleConfig};
use ira_models::ConsoleDef;
use super::helpers::{make_browse_button, string_list_from};
use super::settings_dialog::settings_page_container;

pub(super) fn build_shadps4_version_dropdown(current_path: &str, include_global: bool) -> gtk4::DropDown {
    let shadps4_versions = ira_platforms::ps4::read_shadps4_versions();
    let trunc = |s: &str, max: usize| -> String {
        if s.len() > max {
            format!("{}…", &s[..max.saturating_sub(1)])
        } else {
            s.to_string()
        }
    };
    let mut version_strings: Vec<String> = Vec::new();
    if include_global {
        version_strings.push("Follow global".to_string());
    }
    for v in &shadps4_versions {
        let extra = if !v.date.is_empty() { v.date.clone() } else { v.codename.clone() };
        version_strings.push(format!("{}  ({})", v.name, trunc(&extra, 14)));
    }
    let version_model = string_list_from(&version_strings);
    let version_dropdown = gtk4::DropDown::new(Some(version_model), None::<&gtk4::PropertyExpression>);

    let mut selected_idx: u32 = 0;
    if !current_path.is_empty() {
        for (i, v) in shadps4_versions.iter().enumerate() {
            let v_path = v.path.trim_matches('"');
            if v_path == current_path {
                selected_idx = if include_global { (i + 1) as u32 } else { i as u32 };
                break;
            }
        }
    }
    version_dropdown.set_selected(selected_idx);
    version_dropdown
}

pub(super) fn build_shadps4_settings_page(cfg: &Config, win: &adw::Window) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let ps4_enable_group = adw::PreferencesGroup::new();
    let ps4_enable_row = adw::SwitchRow::new();
    ps4_enable_row.set_title("Enable PS4 integration");
    ps4_enable_row.set_subtitle("Scan shadPS4 install directories for PS4 games");
    ps4_enable_row.set_active(cfg.shadps4_enabled);
    ps4_enable_group.add(&ps4_enable_row);
    page.append(&ps4_enable_group);

    let ps4_exe_group = adw::PreferencesGroup::new();
    ps4_exe_group.set_title("Emulator");

    let ps4_exe_row = adw::EntryRow::new();
    ps4_exe_row.set_title("shadPS4 executable path");
    ps4_exe_row.set_text(&cfg.shadps4_executable);

    let shadps4_versions = ira_platforms::ps4::read_shadps4_versions();
    let detected_path = ira_platforms::ps4::detect_shadps4_version_path();

    if !shadps4_versions.is_empty() {
        let current_exe = if cfg.shadps4_executable.is_empty() {
            detected_path.clone().unwrap_or_default()
        } else {
            cfg.shadps4_executable.clone()
        };
        let version_dropdown = build_shadps4_version_dropdown(&current_exe, false);

        let ps4_exe_row_c = ps4_exe_row.clone();
        version_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected();
            if let Some(versions) = ira_platforms::ps4::read_shadps4_versions().into_iter().nth(idx as usize) {
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

    let ps4_exe_browse = make_browse_button(
        Some(win),
        "Select shadPS4 executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let row = ps4_exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    ps4_exe_row.add_suffix(&ps4_exe_browse);
    ps4_exe_group.add(&ps4_exe_row);
    page.append(&ps4_exe_group);

    let ps4_dirs_group = adw::PreferencesGroup::new();
    ps4_dirs_group.set_title("Install directories");
    ps4_dirs_group.set_description(Some("Managed by shadPS4"));
    let install_dirs = ira_platforms::ps4::read_install_dirs();
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
    page.append(&ps4_dirs_group);

    (page, ps4_enable_row, ps4_exe_row)
}

pub(super) struct ConsolePageWidgets {
    pub(super) enable_row: adw::SwitchRow,
    pub(super) folder_row: adw::EntryRow,
    pub(super) exe_row: adw::EntryRow,
    pub(super) core_dropdown: Option<gtk4::DropDown>,
    pub(super) fullscreen_row: adw::SwitchRow,
}

pub(super) fn build_console_settings_page(
    win: &adw::Window,
    def: &ConsoleDef,
    cc: &ConsoleConfig,
) -> (gtk4::Box, ConsolePageWidgets) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&format!("Enable {} ROM discovery", def.display_name));
    enable_row.set_subtitle(&format!("Scan for {} ROM files in the configured folder", def.display_name));
    enable_row.set_active(cc.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let rom_group = adw::PreferencesGroup::new();
    rom_group.set_title("ROMs");

    let folder_row = adw::EntryRow::new();
    folder_row.set_title("ROM folder");
    folder_row.set_text(&cc.folder);

    let folder_browse = make_browse_button(
        Some(win),
        "Select ROM folder",
        true,
        None,
        {
            let row = folder_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    folder_row.add_suffix(&folder_browse);
    rom_group.add(&folder_row);
    page.append(&rom_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");

    let detected_emulators = ira_platforms::emulator_detect::detect_emulators(def.id);

    let exe_row = adw::EntryRow::new();
    exe_row.set_title("Emulator executable");

    let initial_exe = if cc.executable.is_empty() {
        detected_emulators.first().map(|e| e.launch_command.clone()).unwrap_or_default()
    } else {
        cc.executable.clone()
    };
    exe_row.set_text(&initial_exe);

    if !detected_emulators.is_empty() {
        let emu_names: Vec<String> = detected_emulators.iter()
            .map(|e| e.display_name.clone())
            .collect();
        let emu_model = string_list_from(&emu_names);
        let emu_dropdown = gtk4::DropDown::new(Some(emu_model), None::<&gtk4::PropertyExpression>);

        let mut selected_idx: u32 = 0;
        let current_exe = exe_row.text().to_string();
        if !current_exe.is_empty() {
            for (i, e) in detected_emulators.iter().enumerate() {
                if e.launch_command == current_exe {
                    selected_idx = i as u32;
                    break;
                }
            }
        }
        emu_dropdown.set_selected(selected_idx);

        let exe_row_c = exe_row.clone();
        let emus_clone = detected_emulators.clone();
        emu_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Some(e) = emus_clone.get(idx) {
                exe_row_c.set_text(&e.launch_command);
            }
        });

        let emu_select_row = adw::ActionRow::new();
        emu_select_row.set_title("Detected emulators");
        emu_dropdown.set_valign(gtk4::Align::Center);
        emu_select_row.add_suffix(&emu_dropdown);
        emu_group.add(&emu_select_row);
    }

    let auto_btn = gtk4::Button::with_label("Auto-detect");
    auto_btn.add_css_class("flat");
    auto_btn.set_valign(gtk4::Align::Center);
    {
        let exe_row_c = exe_row.clone();
        let emus_clone = detected_emulators.clone();
        auto_btn.connect_clicked(move |_| {
            if let Some(e) = emus_clone.first() {
                exe_row_c.set_text(&e.launch_command);
            }
        });
    }

    let exe_browse = make_browse_button(
        Some(win),
        "Select emulator executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&auto_btn);
    exe_row.add_suffix(&exe_browse);
    emu_group.add(&exe_row);

    let cores = ira_platforms::emulator_detect::detect_ra_cores();
    let mut core_dropdown: Option<gtk4::DropDown> = None;
    let mut core_row_opt: Option<adw::ActionRow> = None;

    if !cores.is_empty() {
        let mut core_names: Vec<String> = vec!["None (auto-detect)".to_string()];
        core_names.extend(cores.iter().map(|c| c.display_name.clone()));
        let core_model = string_list_from(&core_names);
        let dropdown = gtk4::DropDown::new(Some(core_model), None::<&gtk4::PropertyExpression>);

        let mut selected_idx: u32 = 0;
        if !cc.ra_core.is_empty() {
            for (i, c) in cores.iter().enumerate() {
                if c.path == cc.ra_core {
                    selected_idx = (i + 1) as u32;
                    break;
                }
            }
        }
        dropdown.set_selected(selected_idx);

        let core_row = adw::ActionRow::new();
        core_row.set_title("RetroArch core");
        core_row.set_subtitle("Select a core for this console");
        dropdown.set_valign(gtk4::Align::Center);
        core_row.add_suffix(&dropdown);
        core_row.set_visible(ira_platforms::emulator_detect::is_retroarch(exe_row.text().as_ref()));
        emu_group.add(&core_row);

        core_row_opt = Some(core_row);
        core_dropdown = Some(dropdown);
    }

    auto_btn.set_visible(!detected_emulators.iter().any(|e| e.launch_command == exe_row.text().as_str()));

    let core_row_c = core_row_opt;
    let auto_btn_c = auto_btn.clone();
    let emus_for_changed = detected_emulators.clone();
    exe_row.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if let Some(ref cr) = core_row_c {
            cr.set_visible(ira_platforms::emulator_detect::is_retroarch(&text));
        }
        auto_btn_c.set_visible(!emus_for_changed.iter().any(|e| e.launch_command == text));
    });

    let fullscreen_row = adw::SwitchRow::new();
    fullscreen_row.set_title("Start games in fullscreen");
    fullscreen_row.set_subtitle("Launch the emulator in fullscreen mode");
    fullscreen_row.set_active(cc.fullscreen);
    emu_group.add(&fullscreen_row);

    page.append(&emu_group);
    (page, ConsolePageWidgets { enable_row, folder_row, exe_row, core_dropdown, fullscreen_row })
}
