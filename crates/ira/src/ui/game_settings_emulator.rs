use crate::Game;
use adw::prelude::*;
use glib::clone::Downgrade;
use ira_platforms::emulator_detect::DetectedEmulator;
use std::cell::RefCell;
use std::rc::Rc;

use super::helpers::{entry_path_closure, make_browse_button, string_list_from};

type PendingCell = Rc<RefCell<Option<String>>>;

/// Dropdown row meaning "inherit the console's or integration's emulator".
const FOLLOW_GLOBAL: u32 = 0;

/// The extra trailing row that reveals a free-form executable entry. A
/// custom value may be any binary path or a `flatpak:<app id>` command.
fn custom_index(detected: &[DetectedEmulator]) -> u32 {
    detected.len() as u32 + 1
}

/// Dropdown row for a saved override: follow global when empty, the
/// detected emulator's row when the stored command matches one, and the
/// custom row for anything else (paths Ira never detected).
fn selection_for_override(override_cmd: &str, detected: &[DetectedEmulator]) -> u32 {
    if override_cmd.is_empty() {
        return FOLLOW_GLOBAL;
    }
    detected
        .iter()
        .position(|e| e.launch_command == override_cmd)
        .map_or_else(|| custom_index(detected), |i| i as u32 + 1)
}

/// The command a dropdown row stands for: empty inherits the global
/// setting, detected rows resolve to their commands, and the custom row to
/// whatever the entry carries.
fn command_for_selection(index: u32, detected: &[DetectedEmulator], custom: &str) -> String {
    if index == FOLLOW_GLOBAL {
        String::new()
    } else if index >= custom_index(detected) {
        custom.to_string()
    } else {
        detected
            .get(index as usize - 1)
            .map(|e| e.launch_command.clone())
            .unwrap_or_default()
    }
}

fn build_core_row(
    game: &Game,
    cores: &[ira_platforms::emulator_detect::RaCore],
    pending_ra_core: &PendingCell,
    emu_group: &adw::PreferencesGroup,
) -> Option<adw::ComboRow> {
    if cores.is_empty() {
        return None;
    }

    let mut core_names: Vec<String> = vec![crate::tr!("Follow global")];
    core_names.extend(cores.iter().map(|c| c.display_name.clone()));
    let core_dropdown = adw::ComboRow::new();
    core_dropdown.set_title(&crate::tr!("RetroArch core"));
    core_dropdown.set_subtitle(&crate::tr!("Override the RetroArch core for this game"));
    core_dropdown.set_model(Some(&string_list_from(&core_names)));

    let mut selected_idx: u32 = 0;
    if !game.ra_core.is_empty() {
        for (i, c) in cores.iter().enumerate() {
            if c.path == game.ra_core {
                selected_idx = (i + 1) as u32;
                break;
            }
        }
    }
    core_dropdown.set_selected(selected_idx);

    let pending_ra_core_c = pending_ra_core.clone();
    let cores_clone = cores.to_vec();
    core_dropdown.connect_selected_notify(move |dd| {
        let idx = dd.selected();
        let path = if idx == 0 {
            String::new()
        } else {
            match cores_clone.get((idx - 1) as usize) {
                Some(c) => c.path.clone(),
                None => return,
            }
        };
        *pending_ra_core_c.borrow_mut() = Some(path);
    });

    let is_ra = !game.emulator_override.is_empty()
        && ira_platforms::emulator_detect::is_retroarch(&game.emulator_override);
    core_dropdown.set_visible(is_ra);

    emu_group.add(&core_dropdown);
    Some(core_dropdown)
}

/// The free-form executable entry shown when the dropdown sits on its
/// custom row. Accepts a binary path or a `flatpak:<app id>` command.
fn build_custom_executable_row(win: &adw::Window, initial: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&crate::tr!("Executable"));
    row.set_text(initial);
    let browse = make_browse_button(
        Some(win),
        &crate::tr!("Select executable"),
        false,
        Some(("Executable", &["application/x-executable"])),
        entry_path_closure(&row),
        {
            let row_c = Downgrade::downgrade(&row);
            move |path| {
                if let Some(row_c) = row_c.upgrade() {
                    row_c.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    row.add_suffix(&browse);
    row
}

/// The per-game Emulator group: a dropdown of the console's detected
/// emulators, a custom-executable entry, and — for consoles with
/// RetroArch cores — the per-game core picker.
pub(super) fn add_emulator_dropdown_section(
    page: &gtk4::Box,
    game: &Game,
    win: &adw::Window,
    pending_ra_core: &PendingCell,
    pending_emulator: &PendingCell,
) {
    // Retro games key detection off their console platform; the emulator
    // integrations carry a title id or serial as platform, so map the kind
    // onto the console whose emulators get detected.
    let console_id: &str = match game.kind {
        ira_models::GameKind::ThreeDS => "3ds",
        ira_models::GameKind::WiiU => "wiiu",
        ira_models::GameKind::Ps3 => "ps3",
        _ => &game.platform_id,
    };
    let emulators = ira_platforms::emulator_detect::detect_emulators(console_id);
    let cores = ira_platforms::emulator_detect::detect_ra_cores_for_console(console_id);
    // No early return when nothing is detected: the custom-executable row
    // is exactly how a game gets pointed at an emulator Ira never found.

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));

    let mut emu_names: Vec<String> = vec![crate::tr!("Follow global")];
    emu_names.extend(emulators.iter().map(|e| e.display_name.clone()));
    emu_names.push(crate::tr!("Custom executable\u{2026}"));
    let emu_dropdown = adw::ComboRow::new();
    emu_dropdown.set_title(&crate::tr!("Emulator"));
    emu_dropdown.set_subtitle(&crate::tr!("Override the emulator for this game"));
    emu_dropdown.set_model(Some(&string_list_from(&emu_names)));

    let custom = custom_index(&emulators);
    let selected = selection_for_override(&game.emulator_override, &emulators);
    let custom_row = build_custom_executable_row(
        win,
        if selected == custom {
            &game.emulator_override
        } else {
            ""
        },
    );

    emu_group.add(&emu_dropdown);
    let core_row = build_core_row(game, &cores, pending_ra_core, &emu_group);
    emu_group.add(&custom_row);

    emu_dropdown.set_selected(selected);
    custom_row.set_visible(selected == custom);

    // Selection and text handlers go on last: `set_selected` above fires
    // selected-notify, and the initial state must not be staged as a
    // pending change the user never made.
    let pending_for_selection = pending_emulator.clone();
    let emus_for_selection = emulators.clone();
    let core_row_for_selection = core_row.clone();
    let custom_row_for_selection = custom_row.clone();
    emu_dropdown.connect_selected_notify(move |dd| {
        let cmd = command_for_selection(
            dd.selected(),
            &emus_for_selection,
            &custom_row_for_selection.text(),
        );
        *pending_for_selection.borrow_mut() = Some(cmd.clone());
        custom_row_for_selection.set_visible(dd.selected() == custom);
        if let Some(ref cr) = core_row_for_selection {
            cr.set_visible(ira_platforms::emulator_detect::is_retroarch(&cmd));
        }
    });

    let pending_for_text = pending_emulator.clone();
    let core_row_for_text = core_row;
    custom_row.connect_changed(move |row| {
        if !row.is_visible() {
            return;
        }
        let cmd = row.text().to_string();
        *pending_for_text.borrow_mut() = Some(cmd.clone());
        if let Some(ref cr) = core_row_for_text {
            cr.set_visible(ira_platforms::emulator_detect::is_retroarch(&cmd));
        }
    });

    page.append(&emu_group);
}

#[cfg(test)]
mod tests {
    use super::{command_for_selection, custom_index, selection_for_override};
    use ira_platforms::emulator_detect::DetectedEmulator;

    fn emu(cmd: &str) -> DetectedEmulator {
        DetectedEmulator {
            display_name: cmd.to_string(),
            launch_command: cmd.to_string(),
        }
    }

    fn detected() -> Vec<DetectedEmulator> {
        vec![emu("/usr/bin/dolphin-emu"), emu("flatpak:org.DolphinEmu.dolphin-emu")]
    }

    #[test]
    fn test_selection_for_override_empty_follows_global() {
        assert_eq!(selection_for_override("", &detected()), 0);
    }

    #[test]
    fn test_selection_for_override_matches_detected_commands() {
        assert_eq!(selection_for_override("/usr/bin/dolphin-emu", &detected()), 1);
        assert_eq!(
            selection_for_override("flatpak:org.DolphinEmu.dolphin-emu", &detected()),
            2
        );
    }

    #[test]
    fn test_selection_for_override_unknown_path_lands_on_custom() {
        assert_eq!(
            selection_for_override("/opt/primehack/primehack", &detected()),
            custom_index(&detected())
        );
    }

    #[test]
    fn test_command_for_selection_roundtrips_every_row() {
        let emus = detected();
        assert_eq!(command_for_selection(0, &emus, "/opt/emu"), "");
        assert_eq!(
            command_for_selection(1, &emus, "/opt/emu"),
            "/usr/bin/dolphin-emu"
        );
        assert_eq!(
            command_for_selection(custom_index(&emus), &emus, "/opt/primehack/primehack"),
            "/opt/primehack/primehack"
        );
    }
}
