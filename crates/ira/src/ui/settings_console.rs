use super::css::*;
use super::helpers::{entry_path_closure, make_browse_button, string_list_from};
use super::settings_dialog::settings_page_container;
use adw::prelude::*;
use ira_config::{Config, ConsoleConfig};
use ira_models::ConsoleDef;

pub(super) fn build_emulator_dropdown(
    current_path: &str,
    include_global: bool,
    follow_label: &str,
    emulators: &[ira_platforms::emulator_detect::DetectedEmulator],
) -> gtk4::DropDown {
    let mut version_strings: Vec<String> = Vec::new();
    if include_global {
        version_strings.push(follow_label.to_string());
    }
    version_strings.extend(emulators.iter().map(|e| e.display_name.clone()));
    let version_model = string_list_from(&version_strings);
    let version_dropdown =
        gtk4::DropDown::new(Some(version_model), None::<&gtk4::PropertyExpression>);

    let mut selected_idx: u32 = 0;
    if !current_path.is_empty() {
        for (i, emulator) in emulators.iter().enumerate() {
            if emulator.launch_command == current_path {
                selected_idx = if include_global {
                    (i + 1) as u32
                } else {
                    i as u32
                };
                break;
            }
        }
    }
    version_dropdown.set_selected(selected_idx);
    version_dropdown
}

pub(super) fn build_shadps4_settings_page(
    cfg: &Config,
) -> (gtk4::Box, adw::SwitchRow, Option<gtk4::DropDown>) {
    let page = settings_page_container();

    let ps4_enable_group = adw::PreferencesGroup::new();
    let ps4_enable_row = adw::SwitchRow::new();
    ps4_enable_row.set_title("Enable PS4 integration");
    ps4_enable_row.set_subtitle("Scan shadPS4 install directories for PS4 games");
    ps4_enable_row.set_active(cfg.shadps4_enabled);
    ps4_enable_group.add(&ps4_enable_row);
    page.append(&ps4_enable_group);

    let mut version_dropdown: Option<gtk4::DropDown> = None;
    let emulators = ira_platforms::ps4::read_shadps4_launch_options();
    if !emulators.is_empty() {
        let emu_group = adw::PreferencesGroup::new();
        emu_group.set_title("shadPS4 build");

        let dd = build_emulator_dropdown(
            &cfg.shadps4_executable,
            true,
            "Qt Launcher default",
            &emulators,
        );

        let version_row = adw::ActionRow::new();
        version_row.set_title("Launch build");
        dd.set_valign(gtk4::Align::Center);
        version_row.add_suffix(&dd);
        emu_group.add(&version_row);
        page.append(&emu_group);

        version_dropdown = Some(dd);
    }

    let ps4_dirs_group = adw::PreferencesGroup::new();
    ps4_dirs_group.set_title("Install directories");
    ps4_dirs_group.set_description(Some("Managed by shadPS4"));
    let install_dirs =
        ira_platforms::ps4::read_install_dirs_for_executable(&cfg.shadps4_executable);
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

    (page, ps4_enable_row, version_dropdown)
}

pub(super) fn build_rpcs3_settings_page(
    cfg: &Config,
    win: &adw::Window,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable PS3 integration");
    enable_row.set_subtitle("Scan RPCS3's dev_hdd0 for installed PS3 games");
    enable_row.set_active(cfg.rpcs3_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");

    let detected = ira_platforms::emulator_detect::detect_emulator_choices(
        &["rpcs3", "rpcs3-emu"],
        &[(ira_platforms::ps3::RPCS3_FLATPAK_ID, "RPCS3")],
        "RPCS3",
    );

    let exe_row = adw::EntryRow::new();
    exe_row.set_title("RPCS3 executable path");

    let initial_exe = if cfg.rpcs3_executable.is_empty() {
        detected
            .first()
            .map(|e| e.launch_command.clone())
            .unwrap_or_default()
    } else {
        cfg.rpcs3_executable.clone()
    };
    exe_row.set_text(&initial_exe);

    if let Some(emu) = detected.first() {
        let auto_btn = gtk4::Button::with_label("Auto-detect");
        auto_btn.add_css_class(CSS_FLAT);
        auto_btn.set_valign(gtk4::Align::Center);
        let exe_row_c = exe_row.clone();
        let path = emu.launch_command.clone();
        auto_btn.connect_clicked(move |_| {
            exe_row_c.set_text(&path);
        });
        exe_row.add_suffix(&auto_btn);
    }

    if !detected.is_empty() {
        let dropdown = build_emulator_dropdown(&initial_exe, false, "", &detected);
        let row = adw::ActionRow::new();
        row.set_title("Detected emulators");
        dropdown.set_valign(gtk4::Align::Center);
        row.add_suffix(&dropdown);
        emu_group.add(&row);
        let exe_row_c = exe_row.clone();
        let detected_c = detected.clone();
        dropdown.connect_selected_notify(move |dd| {
            if let Some(emu) = detected_c.get(dd.selected() as usize) {
                exe_row_c.set_text(&emu.launch_command);
            }
        });
    }

    let exe_browse = make_browse_button(
        Some(win),
        "Select RPCS3 executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        entry_path_closure(&exe_row),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&exe_browse);
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title("Install directories");
    dirs_group.set_description(Some("Managed by RPCS3 (dev_hdd0/game)"));
    let games_dir = ira_platforms::ps3::games_dir_for(&initial_exe);
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&games_dir.display().to_string());
    dir_row.set_sensitive(false);
    dirs_group.add(&dir_row);
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

pub(super) fn build_vita3k_settings_page(
    cfg: &Config,
    win: &adw::Window,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable PS Vita integration");
    enable_row.set_subtitle("Scan Vita3K's installed applications for PS Vita games");
    enable_row.set_active(cfg.vita3k_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");
    let detected = ira_platforms::emulator_detect::detect_emulator_choices(
        &["vita3k", "Vita3K"],
        &[],
        "Vita3K",
    );
    let exe_row = adw::EntryRow::new();
    exe_row.set_title("Vita3K executable path");
    let initial_exe = if cfg.vita3k_executable.is_empty() {
        detected
            .first()
            .map(|emu| emu.launch_command.clone())
            .unwrap_or_default()
    } else {
        cfg.vita3k_executable.clone()
    };
    exe_row.set_text(&initial_exe);
    add_detected_emulator_dropdown(&emu_group, &exe_row, &detected);
    let browse = make_browse_button(
        Some(win),
        "Select Vita3K executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        entry_path_closure(&exe_row),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&browse);
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title("Install directory");
    dirs_group.set_description(Some("Vita3K stores installed applications below ux0/app"));
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(
        &ira_platforms::vita3k::vita_fs_path_for(&initial_exe)
            .join("ux0/app")
            .display()
            .to_string(),
    );
    dir_row.set_sensitive(false);
    dirs_group.add(&dir_row);
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

pub(super) fn build_cemu_settings_page(
    cfg: &Config,
    win: &adw::Window,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable Wii U integration");
    enable_row.set_subtitle("Scan Cemu's configured game paths and installed titles");
    enable_row.set_active(cfg.cemu_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");
    let detected = ira_platforms::emulator_detect::detect_emulator_choices(
        &["cemu", "Cemu"],
        &[(ira_platforms::cemu::CEMU_FLATPAK_ID, "Cemu")],
        "Cemu",
    );
    let exe_row = adw::EntryRow::new();
    exe_row.set_title("Cemu executable path");
    let initial_exe = if cfg.cemu_executable.is_empty() {
        detected
            .first()
            .map(|emu| emu.launch_command.clone())
            .unwrap_or_default()
    } else {
        cfg.cemu_executable.clone()
    };
    exe_row.set_text(&initial_exe);
    add_detected_emulator_dropdown(&emu_group, &exe_row, &detected);
    let browse = make_browse_button(
        Some(win),
        "Select Cemu executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        entry_path_closure(&exe_row),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&browse);
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title("Install directories");
    dirs_group.set_description(Some("Managed by Cemu"));
    let mlc_row = adw::ActionRow::new();
    mlc_row.set_title(
        &ira_platforms::cemu::mlc_path_for(&initial_exe)
            .display()
            .to_string(),
    );
    mlc_row.set_subtitle("MLC path");
    mlc_row.set_sensitive(false);
    dirs_group.add(&mlc_row);
    for path in ira_platforms::cemu::configured_game_paths_for(&initial_exe) {
        let row = adw::ActionRow::new();
        row.set_title(&path.display().to_string());
        row.set_subtitle("Configured game path");
        row.set_sensitive(false);
        dirs_group.add(&row);
    }
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

fn add_detected_emulator_dropdown(
    group: &adw::PreferencesGroup,
    exe_row: &adw::EntryRow,
    detected: &[ira_platforms::emulator_detect::DetectedEmulator],
) {
    if detected.is_empty() {
        return;
    }
    let current = exe_row.text();
    let dropdown = build_emulator_dropdown(current.as_str(), false, "", detected);
    let row = adw::ActionRow::new();
    row.set_title("Detected emulators");
    dropdown.set_valign(gtk4::Align::Center);
    row.add_suffix(&dropdown);
    group.add(&row);
    let exe_row = exe_row.clone();
    let detected = detected.to_vec();
    dropdown.connect_selected_notify(move |dropdown| {
        if let Some(emu) = detected.get(dropdown.selected() as usize) {
            exe_row.set_text(&emu.launch_command);
        }
    });
}

pub(super) struct ConsolePageWidgets {
    pub(super) enable_row: adw::SwitchRow,
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
    let display_name = gtk4::glib::markup_escape_text(def.display_name);
    enable_row.set_title(&format!("Enable {display_name} ROM discovery"));
    enable_row.set_subtitle(&format!(
        "Scan for {display_name} ROM files in the configured folder"
    ));
    enable_row.set_active(cc.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");

    let detected_emulators = ira_platforms::emulator_detect::detect_emulators(def.id);

    let exe_row = adw::EntryRow::new();
    exe_row.set_title("Emulator executable");

    let initial_exe = if cc.executable.is_empty() {
        detected_emulators
            .first()
            .map(|e| e.launch_command.clone())
            .unwrap_or_default()
    } else {
        cc.executable.clone()
    };
    exe_row.set_text(&initial_exe);

    if !detected_emulators.is_empty() {
        let current_exe = exe_row.text().to_string();
        let emu_dropdown = build_emulator_dropdown(&current_exe, false, "", &detected_emulators);

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
    auto_btn.add_css_class(CSS_FLAT);
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
        entry_path_closure(&exe_row),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&auto_btn);
    exe_row.add_suffix(&exe_browse);
    emu_group.add(&exe_row);

    let cores = ira_platforms::emulator_detect::detect_ra_cores_for_console(def.id);
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
        core_row.set_visible(ira_platforms::emulator_detect::is_retroarch(
            exe_row.text().as_ref(),
        ));
        emu_group.add(&core_row);

        core_row_opt = Some(core_row);
        core_dropdown = Some(dropdown);
    }

    auto_btn.set_visible(
        !detected_emulators
            .iter()
            .any(|e| e.launch_command == exe_row.text().as_str()),
    );

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
    (
        page,
        ConsolePageWidgets {
            enable_row,
            exe_row,
            core_dropdown,
            fullscreen_row,
        },
    )
}
