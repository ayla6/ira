use super::css::*;
use super::helpers::{entry_path_closure, esc, make_browse_button, string_list_from};
use super::settings_dialog::settings_page_container;
use adw::prelude::*;
use ira_config::{Config, ConsoleConfig};
use ira_models::ConsoleDef;
use std::cell::Cell;
use std::rc::Rc;

pub(super) fn build_emulator_dropdown(
    current_path: &str,
    include_global: bool,
    follow_label: &str,
    emulators: &[ira_platforms::emulator_detect::DetectedEmulator],
) -> adw::ComboRow {
    let mut version_strings: Vec<String> = Vec::new();
    if include_global {
        version_strings.push(follow_label.to_string());
    }
    version_strings.extend(emulators.iter().map(|e| e.display_name.clone()));
    let version_dropdown = adw::ComboRow::new();
    version_dropdown.set_title(&crate::tr!("Version"));
    version_dropdown.set_model(Some(&string_list_from(&version_strings)));

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
) -> (gtk4::Box, adw::SwitchRow, Option<adw::ComboRow>) {
    let page = settings_page_container();

    let ps4_enable_group = adw::PreferencesGroup::new();
    let ps4_enable_row = adw::SwitchRow::new();
    ps4_enable_row.set_title(&crate::tr!("Enable PS4 integration"));
    ps4_enable_row.set_subtitle(&crate::tr!(
        "Scan shadPS4 install directories for PS4 games"
    ));
    ps4_enable_row.set_active(cfg.shadps4_enabled);
    ps4_enable_group.add(&ps4_enable_row);
    page.append(&ps4_enable_group);

    let mut version_dropdown: Option<adw::ComboRow> = None;
    let emulators = ira_platforms::ps4::read_shadps4_launch_options();
    if !emulators.is_empty() {
        let emu_group = adw::PreferencesGroup::new();
        emu_group.set_title(&crate::tr!("Emulator"));

        let dd = build_emulator_dropdown(
            &cfg.shadps4_executable,
            true,
            &crate::tr!("Qt Launcher default"),
            &emulators,
        );

        emu_group.add(&dd);
        page.append(&emu_group);

        version_dropdown = Some(dd);
    }

    let ps4_dirs_group = adw::PreferencesGroup::new();
    ps4_dirs_group.set_title(&crate::tr!("Install directories"));
    ps4_dirs_group.set_description(Some(&crate::tr!("Managed by shadPS4")));
    let install_dirs =
        ira_platforms::ps4::read_install_dirs_for_executable(&cfg.shadps4_executable);
    if install_dirs.is_empty() {
        let empty_row = adw::ActionRow::new();
        empty_row.set_title(&crate::tr!("No install directories configured"));
        empty_row.set_sensitive(false);
        ps4_dirs_group.add(&empty_row);
    } else {
        for dir in &install_dirs {
            let dir_row = adw::ActionRow::new();
            dir_row.set_title(&esc(&dir.display().to_string()));
            dir_row.set_sensitive(false);
            ps4_dirs_group.add(&dir_row);
        }
    }
    page.append(&ps4_dirs_group);

    (page, ps4_enable_row, version_dropdown)
}

pub(super) fn build_rpcs3_settings_page(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable PS3 integration"));
    enable_row.set_subtitle(&crate::tr!("Scan RPCS3's dev_hdd0 for installed PS3 games"));
    enable_row.set_active(cfg.rpcs3_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));

    let detected = ira_platforms::emulator_detect::detect_emulator_choices(
        &["rpcs3", "rpcs3-emu"],
        &[(ira_platforms::ps3::RPCS3_FLATPAK_ID, "RPCS3")],
        "RPCS3",
    );

    let exe_row = adw::EntryRow::new();
    exe_row.set_title(&crate::tr!("RPCS3 executable path"));

    let initial_exe = if cfg.rpcs3_executable.is_empty() {
        detected
            .first()
            .map(|e| e.launch_command.clone())
            .unwrap_or_default()
    } else {
        cfg.rpcs3_executable.clone()
    };
    exe_row.set_text(&initial_exe);

    add_detected_emulator_dropdown(&emu_group, &exe_row, &detected);
    add_executable_actions(&exe_row, win, &detected, &crate::tr!("Select executable"));
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title(&crate::tr!("Install directories"));
    dirs_group.set_description(Some(&crate::tr!("Managed by RPCS3 (dev_hdd0/game)")));
    let games_dir = ira_platforms::ps3::games_dir_for(&initial_exe);
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&esc(&games_dir.display().to_string()));
    dir_row.set_sensitive(false);
    dirs_group.add(&dir_row);
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

pub(super) fn build_vita3k_settings_page(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable PS Vita integration"));
    enable_row.set_subtitle(&crate::tr!(
        "Scan Vita3K's installed applications for PS Vita games"
    ));
    enable_row.set_active(cfg.vita3k_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));
    let detected = ira_platforms::emulator_detect::detect_emulator_choices(
        &["vita3k", "Vita3K"],
        &[],
        "Vita3K",
    );
    let exe_row = adw::EntryRow::new();
    exe_row.set_title(&crate::tr!("Vita3K executable path"));
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
    add_executable_actions(&exe_row, win, &detected, &crate::tr!("Select executable"));
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title(&crate::tr!("Install directory"));
    dirs_group.set_description(Some(&crate::tr!(
        "Vita3K stores installed applications below ux0/app"
    )));
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&esc(&ira_platforms::vita3k::vita_fs_path_for(&initial_exe)
        .join("ux0/app")
        .display()
        .to_string()));
    dir_row.set_sensitive(false);
    dirs_group.add(&dir_row);
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

pub(super) fn build_cemu_settings_page(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable Wii U integration"));
    enable_row.set_subtitle(&crate::tr!(
        "Scan Cemu's configured game paths and installed titles"
    ));
    enable_row.set_active(cfg.cemu_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));
    let detected = ira_platforms::emulator_detect::cemu_choices();
    let exe_row = adw::EntryRow::new();
    exe_row.set_title(&crate::tr!("Cemu executable path"));
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
    add_executable_actions(&exe_row, win, &detected, &crate::tr!("Select executable"));
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title(&crate::tr!("Install directories"));
    dirs_group.set_description(Some(&crate::tr!("Managed by Cemu")));
    let mlc_row = adw::ActionRow::new();
    mlc_row.set_title(&esc(&ira_platforms::cemu::mlc_path_for(&initial_exe)
        .display()
        .to_string()));
    mlc_row.set_subtitle(&crate::tr!("MLC path"));
    mlc_row.set_sensitive(false);
    dirs_group.add(&mlc_row);
    for path in ira_platforms::cemu::configured_game_paths_for(&initial_exe) {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&path.display().to_string()));
        row.set_subtitle(&crate::tr!("Configured game path"));
        row.set_sensitive(false);
        dirs_group.add(&row);
    }
    page.append(&dirs_group);

    (page, enable_row, exe_row)
}

pub(super) fn build_azahar_settings_page(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable Nintendo 3DS integration"));
    enable_row.set_subtitle(&crate::tr!(
        "Scan Azahar's game folders and installed 3DS titles"
    ));
    enable_row.set_active(cfg.azahar_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));
    let detected = ira_platforms::emulator_detect::azahar_choices();
    let exe_row = adw::EntryRow::new();
    exe_row.set_title(&crate::tr!("Azahar executable path"));
    let initial_exe = if cfg.azahar_executable.is_empty() {
        detected
            .first()
            .map(|emu| emu.launch_command.clone())
            .unwrap_or_default()
    } else {
        cfg.azahar_executable.clone()
    };
    exe_row.set_text(&initial_exe);
    add_detected_emulator_dropdown(&emu_group, &exe_row, &detected);
    add_executable_actions(&exe_row, win, &detected, &crate::tr!("Select executable"));
    emu_group.add(&exe_row);
    page.append(&emu_group);

    let dirs_group = adw::PreferencesGroup::new();
    dirs_group.set_title(&crate::tr!("Game locations"));
    dirs_group.set_description(Some(&crate::tr!("Managed by Azahar")));
    let paths = ira_platforms::azahar::read_paths_for_executable(&initial_exe);
    for (path, deep_scan) in paths.game_dirs {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&path.display().to_string()));
        let subtitle = if deep_scan {
            crate::tr!("Game folder (deep scan)")
        } else {
            crate::tr!("Game folder")
        };
        row.set_subtitle(&subtitle);
        row.set_sensitive(false);
        dirs_group.add(&row);
    }
    for (path, label) in [
        (paths.nand_dir, crate::tr!("NAND")),
        (paths.sdmc_dir, crate::tr!("SD card")),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&path.display().to_string()));
        row.set_subtitle(&label);
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
    let mut labels: Vec<String> = detected
        .iter()
        .map(|emulator| emulator.display_name.clone())
        .collect();
    labels.push(crate::tr!("Custom…"));

    let dropdown = adw::ComboRow::new();
    dropdown.set_title(&crate::tr!("Emulator"));
    dropdown.set_model(Some(&string_list_from(&labels)));
    // Selecting an unknown or empty path lands on "Custom…" instead of
    // silently pointing at the first detection.
    dropdown.set_selected(selection_index_for_text(detected, &exe_row.text()));
    group.add(&dropdown);

    // Guards against feedback loops: programmatic entry updates triggered by
    // a dropdown change must not re-enter the dropdown update.
    let syncing = Rc::new(Cell::new(false));

    let detected_for_entry = detected.to_vec();
    // Weak refs: each handler holding the other widget strongly would form
    // a retain cycle and leak both widgets per settings-dialog open.
    let dropdown_for_entry = dropdown.downgrade();
    let syncing_for_entry = syncing.clone();
    exe_row.connect_changed(move |row| {
        if syncing_for_entry.get() {
            return;
        }
        syncing_for_entry.set(true);
        if let Some(dropdown) = dropdown_for_entry.upgrade() {
            dropdown.set_selected(selection_index_for_text(&detected_for_entry, &row.text()));
        }
        syncing_for_entry.set(false);
    });

    let exe_row_for_dropdown = exe_row.downgrade();
    let detected_for_dropdown = detected.to_vec();
    let syncing_for_dropdown = syncing;
    dropdown.connect_selected_notify(move |dropdown| {
        syncing_for_dropdown.set(true);
        if let Some(exe_row) = exe_row_for_dropdown.upgrade() {
            if let Some(emulator) = detected_for_dropdown.get(dropdown.selected() as usize) {
                exe_row.set_text(&emulator.launch_command);
            }
        }
        syncing_for_dropdown.set(false);
    });
}

/// Index into the dropdown model for `text`: the matching detection, or the
/// trailing "Custom…" entry.
fn selection_index_for_text(
    detected: &[ira_platforms::emulator_detect::DetectedEmulator],
    text: &str,
) -> u32 {
    match detected
        .iter()
        .position(|emulator| emulator.launch_command == text)
    {
        Some(index) => index as u32,
        None => detected.len() as u32,
    }
}

fn add_executable_actions(
    row: &adw::EntryRow,
    parent: &impl IsA<gtk4::Widget>,
    detected: &[ira_platforms::emulator_detect::DetectedEmulator],
    browse_title: &str,
) {
    if let Some(emulator) = detected.first() {
        let auto_detect = gtk4::Button::from_icon_name("system-search-symbolic");
        auto_detect.add_css_class(CSS_FLAT);
        auto_detect.add_css_class(CSS_SQUARE_BUTTON);
        auto_detect.set_tooltip_text(Some(&crate::tr!("Auto-detect emulator")));
        auto_detect.set_valign(gtk4::Align::Center);
        let row_for_detect = row.clone();
        let path = emulator.launch_command.clone();
        auto_detect.connect_clicked(move |_| row_for_detect.set_text(&path));
        set_auto_detect_visible(&auto_detect, row, detected);
        let auto_detect_for_changed = auto_detect.clone();
        let detected_for_changed = detected.to_vec();
        row.connect_changed(move |row| {
            set_auto_detect_visible(&auto_detect_for_changed, row, &detected_for_changed);
        });
        row.add_suffix(&auto_detect);
    }
        let browse = make_browse_button(
            Some(parent),
            browse_title,
        false,
        Some(("Executable", &["application/x-executable"])),
        entry_path_closure(row),
        {
            let row = row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    row.add_suffix(&browse);
}

fn set_auto_detect_visible(
    button: &gtk4::Button,
    row: &adw::EntryRow,
    detected: &[ira_platforms::emulator_detect::DetectedEmulator],
) {
    button.set_visible(
        !detected
            .iter()
            .any(|emulator| emulator.launch_command == row.text().as_str()),
    );
}

pub(super) struct ConsolePageWidgets {
    pub(super) enable_row: adw::SwitchRow,
    pub(super) exe_row: adw::EntryRow,
    pub(super) core_path_row: Option<adw::EntryRow>,
    pub(super) fullscreen_row: adw::SwitchRow,
}

pub(super) fn build_console_settings_page(
    win: &impl IsA<gtk4::Widget>,
    def: &ConsoleDef,
    cc: &ConsoleConfig,
) -> (gtk4::Box, ConsolePageWidgets) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    let display_name = gtk4::glib::markup_escape_text(def.display_name);
    enable_row.set_title(
        &crate::tr!("Enable {display_name} ROM discovery").replace("{display_name}", &display_name),
    );
    enable_row.set_subtitle(
        &crate::tr!("Scan for {display_name} ROM files in the configured folder")
            .replace("{display_name}", &display_name),
    );
    enable_row.set_active(cc.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));

    let detected_emulators = ira_platforms::emulator_detect::detect_emulators(def.id);

    let exe_row = adw::EntryRow::new();
    exe_row.set_title(&crate::tr!("Emulator executable"));

    let initial_exe = if cc.executable.is_empty() {
        detected_emulators
            .first()
            .map(|e| e.launch_command.clone())
            .unwrap_or_default()
    } else {
        cc.executable.clone()
    };
    exe_row.set_text(&initial_exe);

    add_detected_emulator_dropdown(&emu_group, &exe_row, &detected_emulators);

    add_executable_actions(
        &exe_row,
        win,
        &detected_emulators,
        &crate::tr!("Select executable"),
    );
    emu_group.add(&exe_row);

    let mut core_path_row: Option<adw::EntryRow> = None;
    let mut core_row_opt: Option<adw::ActionRow> = None;
    let mut core_selector_opt: Option<adw::ComboRow> = None;
    let mut custom_core_selected: Option<Rc<Cell<bool>>> = None;

    if ira_platforms::emulator_detect::supports_retroarch_cores(def.id) {
        let cores = ira_platforms::emulator_detect::detect_ra_cores_for_console(def.id);
        let configured_core = (!cc.ra_core.is_empty()
            && std::path::Path::new(&cc.ra_core).is_file())
        .then(|| cc.ra_core.clone());
        let selected_core = configured_core.or_else(|| cores.first().map(|core| core.path.clone()));
        let core_path = adw::EntryRow::new();
        core_path.set_title(&crate::tr!("Custom core file"));
        core_path.set_text(selected_core.as_deref().unwrap_or_default());
        let browse = make_browse_button(
            Some(win),
            &crate::tr!("Select RetroArch core"),
            false,
            None,
            entry_path_closure(&core_path),
            {
                let row = core_path.clone();
                move |path| row.set_text(&path.to_string_lossy())
            },
        );
        core_path.add_suffix(&browse);
        core_path.set_visible(ira_platforms::emulator_detect::is_retroarch(
            exe_row.text().as_ref(),
        ));

        let is_retroarch = ira_platforms::emulator_detect::is_retroarch(exe_row.text().as_ref());
        if cores.is_empty() {
            let core_row = adw::ActionRow::new();
            core_row.set_title(&crate::tr!("RetroArch core"));
            core_row.set_subtitle(&crate::tr!(
                "No compatible cores installed. Install one with RetroArch's Core Downloader."
            ));
            core_row.set_sensitive(false);
            core_row.set_visible(is_retroarch);
            core_path.set_visible(is_retroarch);
            emu_group.add(&core_row);
            core_row_opt = Some(core_row);
        } else {
            let core_names = cores
                .iter()
                .map(|core| core.display_name.clone())
                .collect::<Vec<_>>();
            let mut core_names = core_names;
            core_names.push(crate::tr!("Custom core file..."));
            let dropdown = adw::ComboRow::new();
            dropdown.set_title(&crate::tr!("RetroArch core"));
            dropdown.set_subtitle(&crate::tr!("Select a core for this console"));
            dropdown.set_model(Some(&string_list_from(&core_names)));
            let selected_idx = selected_core
                .as_ref()
                .and_then(|path| cores.iter().position(|core| &core.path == path))
                .unwrap_or(cores.len());
            dropdown.set_selected(selected_idx as u32);
            dropdown.set_visible(is_retroarch);
            let custom_selected = Rc::new(Cell::new(selected_idx == cores.len()));
            core_path.set_visible(is_retroarch && custom_selected.get());
            let core_path_for_selection = core_path.clone();
            let cores_for_selection = cores;
            let custom_selected_for_selection = custom_selected.clone();
            dropdown.connect_selected_notify(move |dropdown| {
                if let Some(core) = cores_for_selection.get(dropdown.selected() as usize) {
                    core_path_for_selection.set_text(&core.path);
                    core_path_for_selection.set_visible(false);
                    custom_selected_for_selection.set(false);
                } else {
                    core_path_for_selection.set_visible(true);
                    custom_selected_for_selection.set(true);
                }
            });
            custom_core_selected = Some(custom_selected);
            emu_group.add(&dropdown);
            core_selector_opt = Some(dropdown);
        }
        emu_group.add(&core_path);
        core_path_row = Some(core_path);
    }

    let core_row_c = core_row_opt;
    let core_selector_c = core_selector_opt;
    let core_path_row_c = core_path_row.clone();
    let custom_core_selected_c = custom_core_selected;
    exe_row.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if let Some(ref cr) = core_row_c {
            cr.set_visible(ira_platforms::emulator_detect::is_retroarch(&text));
        }
        if let Some(ref selector) = core_selector_c {
            selector.set_visible(ira_platforms::emulator_detect::is_retroarch(&text));
        }
        if let Some(ref core_path) = core_path_row_c {
            core_path.set_visible(
                ira_platforms::emulator_detect::is_retroarch(&text)
                    && custom_core_selected_c
                        .as_ref()
                        .is_none_or(|selected| selected.get()),
            );
        }
    });

    let fullscreen_row = adw::SwitchRow::new();
    fullscreen_row.set_title(&crate::tr!("Start games in fullscreen"));
    fullscreen_row.set_subtitle(&crate::tr!("Launch the emulator in fullscreen mode"));
    fullscreen_row.set_active(cc.fullscreen);
    emu_group.add(&fullscreen_row);

    page.append(&emu_group);
    (
        page,
        ConsolePageWidgets {
            enable_row,
            exe_row,
            core_path_row,
            fullscreen_row,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu(command: &str) -> ira_platforms::emulator_detect::DetectedEmulator {
        ira_platforms::emulator_detect::DetectedEmulator {
            display_name: command.to_string(),
            launch_command: command.to_string(),
        }
    }

    #[test]
    fn test_selection_index_matches_detected_command() {
        let detected = [emu("flatpak:info.cemu.Cemu"), emu("/usr/bin/cemu")];
        assert_eq!(selection_index_for_text(&detected, "/usr/bin/cemu"), 1);
    }

    #[test]
    fn test_selection_index_unknown_path_lands_on_custom() {
        let detected = [emu("flatpak:info.cemu.Cemu")];
        assert_eq!(selection_index_for_text(&detected, "/opt/Cemu"), 1);
        assert_eq!(selection_index_for_text(&detected, ""), 1);
    }
}
