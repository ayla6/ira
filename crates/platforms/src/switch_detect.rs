//! Switch emulator detection: every fork and release channel labeled on
//! its own, searched on $PATH and in the usual non-PATH install locations
//! (AppImages, extracted release tarballs).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::emulator_detect::{detect_flatpak, which, DetectedEmulator};

/// Switch emulators searched for on this machine: binary names per
/// fork/channel with the label shown in the picker. More specific names
/// come before generic ones they share a prefix with, so versioned
/// AppImages are labeled exactly once (`Ryujinx-Canary…` before `Ryujinx`).
const SWITCH_VARIANTS: &[(&[&str], &str)] = &[
    (
        &["ryujinxcanary", "Ryujinx-Canary", "ryujinx-canary"],
        "Ryujinx Canary",
    ),
    (
        &[
            "Ryujinx",
            "ryujinx",
            "ryubing",
            "Ryubing",
            "Ryujinx.Launcher",
            "ryujinx-launcher",
        ],
        "Ryujinx",
    ),
    (&["Kenji-NX", "kenji-nx", "KenjiNX", "kenjinx"], "Kenji-NX"),
    (&["eden_nightly", "Eden-Nightly", "eden-nightly"], "Eden Nightly"),
    (&["eden", "Eden"], "Eden"),
    (&["yuzu", "Yuzu"], "Yuzu"),
    (&["suyu", "Suyu"], "Suyu"),
    (&["citron", "Citron"], "Citron"),
    (&["sudachi", "Sudachi"], "Sudachi"),
];

/// Every installable Switch emulator (native + Flatpak), each fork and
/// release channel labeled on its own.
pub fn choices() -> Vec<DetectedEmulator> {
    choices_in(&switch_search_dirs())
}

/// Launch commands of every Switch emulator found on this machine, so
/// portable installs of all forks can contribute their caches and
/// libraries — not just the emulator selected in settings.
pub fn detected_launch_commands() -> Vec<String> {
    choices()
        .into_iter()
        .map(|emulator| emulator.launch_command)
        .collect()
}

fn choices_in(search_dirs: &[PathBuf]) -> Vec<DetectedEmulator> {
    let mut choices = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |path: String, label: &str| {
        if seen.insert(path.clone()) {
            choices.push(DetectedEmulator {
                display_name: format!("{} (native)", label),
                launch_command: path,
            });
        }
    };
    for (names, label) in SWITCH_VARIANTS {
        for name in *names {
            if let Some(path) = which(name) {
                push(path, label);
            }
        }
    }
    // Release tarballs and AppImages rarely live on $PATH; probe the usual
    // install locations too.
    let mut files = Vec::new();
    for dir in search_dirs {
        collect_executable_files(dir, &mut files);
    }
    for (names, label) in SWITCH_VARIANTS {
        for file in &files {
            let Some(file_name) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if names
                .iter()
                .any(|name| name_matches_file(name, file_name))
            {
                push(file.to_string_lossy().into_owned(), label);
            }
        }
    }
    for (app_id, app_name) in [
        ("io.github.ryubing.Ryujinx", "Ryubing"),
        ("dev.eden_emu.eden", "Eden"),
    ] {
        if let Some(flatpak) = detect_flatpak(app_id, app_name) {
            choices.push(flatpak);
        }
    }
    choices
}

/// Directories probed for Switch emulators beyond $PATH: AppImages and
/// extracted release tarballs usually end up in one of these.
fn switch_search_dirs() -> Vec<PathBuf> {
    let home = crate::emu_dirs::home_dir();
    vec![
        home.join(".local/bin"),
        home.join("bin"),
        home.join("Applications"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt"),
    ]
}

/// True when `file_name` is a binary or AppImage of `name`: an exact
/// match, or a versioned AppImage sharing the prefix
/// (`Eden-0.0.1-x86_64.AppImage` for `Eden`).
fn name_matches_file(name: &str, file_name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let file = file_name.to_ascii_lowercase();
    file == name || (file.starts_with(&name) && file.ends_with(".appimage"))
}

/// Files directly in `dir` plus one directory level below it, so
/// `/opt/eden-nightly/eden_nightly` is found without deep scans.
fn collect_executable_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(t) if t.is_file() => out.push(entry.path()),
            Ok(t) if t.is_dir() => {
                if let Ok(sub) = std::fs::read_dir(entry.path()) {
                    out.extend(
                        sub.flatten()
                            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                            .map(|e| e.path()),
                    );
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_matches_file_exact_and_versioned_appimage() {
        assert!(name_matches_file("eden_nightly", "eden_nightly"));
        assert!(name_matches_file("Eden", "Eden-0.0.1-x86_64.AppImage"));
        assert!(name_matches_file(
            "Ryujinx",
            "ryujinx-1.3.0-linux_x64.AppImage"
        ));
        assert!(!name_matches_file("Eden", "Suyu.AppImage"));
        assert!(!name_matches_file("Eden", "eden_nightly"));
        assert!(!name_matches_file("Ryujinx", "Ryujinx-1.0.tar.gz"));
    }

    /// AppImages and bare binaries in well-known install locations are
    /// found and labeled per fork; more specific variants win their
    /// shared prefix.
    #[test]
    fn test_choices_scan_well_known_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let opt = tmp.path().join("opt");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::create_dir_all(opt.join("eden-nightly")).unwrap();
        std::fs::write(apps.join("Ryubing.AppImage"), b"").unwrap();
        std::fs::write(apps.join("Ryujinx-Canary-1.3.0.AppImage"), b"").unwrap();
        std::fs::write(opt.join("eden-nightly").join("eden_nightly"), b"").unwrap();

        let choices = choices_in(&[apps, opt]);

        let ryubing = choices
            .iter()
            .find(|e| e.launch_command.ends_with("Ryubing.AppImage"))
            .unwrap();
        assert_eq!(ryubing.display_name, "Ryujinx (native)");
        let canary = choices
            .iter()
            .find(|e| e.launch_command.ends_with("Ryujinx-Canary-1.3.0.AppImage"))
            .unwrap();
        assert_eq!(canary.display_name, "Ryujinx Canary (native)");
        // The canary file must not also be listed as plain Ryujinx.
        assert_eq!(
            choices
                .iter()
                .filter(|e| e.launch_command.ends_with("Ryujinx-Canary-1.3.0.AppImage"))
                .count(),
            1
        );
        let nightly = choices
            .iter()
            .find(|e| e.launch_command.ends_with("eden_nightly"))
            .unwrap();
        assert_eq!(nightly.display_name, "Eden Nightly (native)");
    }
}
