use std::path::{Path, PathBuf};

use super::paths::SHADPS4_FLATPAK_ID;

/// Path to the shadPS4 Qt Launcher data directory.
fn shadps4qt_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .map(|p| p.join("shadPS4QtLauncher"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("shadPS4QtLauncher")
        })
}

/// A version entry from versions.json.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ShadPs4Version {
    pub codename: String,
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub version_type: i32,
    #[serde(default)]
    pub date: String,
}

/// Read the list of available shadPS4 versions from the Qt Launcher's versions.json.
pub fn read_shadps4_versions() -> Vec<ShadPs4Version> {
    let versions_path = shadps4qt_dir().join("versions.json");
    std::fs::read_to_string(versions_path)
        .ok()
        .and_then(|data| serde_json::from_str::<Vec<ShadPs4Version>>(&data).ok())
        .unwrap_or_default()
}

pub fn read_shadps4_launch_options() -> Vec<crate::emulator_detect::DetectedEmulator> {
    let mut options = read_shadps4_versions()
        .into_iter()
        .map(|version| crate::emulator_detect::DetectedEmulator {
            display_name: if version.date.is_empty() {
                version.name
            } else {
                format!("{} ({})", version.name, version.date)
            },
            launch_command: version.path.trim_matches('"').to_string(),
        })
        .filter(|option| executable_available(&option.launch_command))
        .collect::<Vec<_>>();
    let native_options =
        crate::emulator_detect::detect_emulator_choices(&["shadps4", "shadPS4"], &[], "shadPS4");
    for detected in native_options {
        if !options
            .iter()
            .any(|option| option.launch_command == detected.launch_command)
        {
            options.push(detected);
        }
    }
    options.extend(crate::emulator_detect::detect_emulator_choices(
        &[],
        &[(SHADPS4_FLATPAK_ID, "shadPS4")],
        "shadPS4",
    ));
    options
}

/// Find the path of the currently selected shadPS4 version by reading qt_ui.ini.
/// Falls back to checking `versions.json` for releases (type 0) if the ini is unavailable.
pub fn detect_shadps4_version_path() -> Option<String> {
    // Try reading qt_ui.ini for versionSelected
    let ini_path = shadps4qt_dir().join("qt_ui.ini");
    if let Ok(data) = std::fs::read_to_string(&ini_path) {
        for line in data.lines() {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("versionSelected=") {
                let path = val.trim_matches('"').to_string();
                if !path.is_empty() && executable_available(&path) {
                    return Some(path);
                }
            }
        }
    }

    // Fallback: pick the first installed version from versions.json.
    let versions = read_shadps4_versions();
    for v in &versions {
        let p = v.path.trim_matches('"').to_string();
        if !p.is_empty() && executable_available(&p) {
            return Some(p);
        }
    }
    None
}

/// Resolve which shadPS4 executable to launch, always against the current
/// versions.json so stale stored paths fall through instead of going stale:
/// 1. per-game version pin (if it still exists as a known version)
/// 2. global version pin (if it still exists as a known version)
/// 3. the version currently selected in the shadPS4 Qt Launcher
/// 4. `shadps4` from PATH
pub fn resolve_shadps4_executable(per_game: &str, global: &str) -> String {
    pick_shadps4_executable(
        per_game,
        global,
        &read_shadps4_launch_options(),
        detect_shadps4_version_path().as_deref(),
    )
}

fn pick_shadps4_executable(
    per_game: &str,
    global: &str,
    versions: &[crate::emulator_detect::DetectedEmulator],
    detected: Option<&str>,
) -> String {
    for candidate in [per_game, global] {
        let candidate = candidate.trim_matches('"');
        if candidate.is_empty() {
            continue;
        }
        if candidate.starts_with("flatpak:") {
            if shadps4_executable_available(candidate) {
                return candidate.to_string();
            }
            continue;
        }
        let is_known = versions.iter().any(|v| v.launch_command == candidate);
        if is_known && executable_available(candidate) {
            return candidate.to_string();
        }
    }
    if let Some(p) = detected {
        let p = p.trim_matches('"');
        if !p.is_empty() && executable_available(p) {
            return p.to_string();
        }
    }
    if let Some(candidate) = versions
        .iter()
        .map(|version| version.launch_command.as_str())
        .find(|candidate| executable_available(candidate))
    {
        return candidate.to_string();
    }
    for name in ["shadps4", "shadPS4"] {
        if let Ok(path) = which::which(name) {
            return path.to_string_lossy().into_owned();
        }
    }
    "shadps4".to_string()
}

pub fn shadps4_executable_available(candidate: &str) -> bool {
    candidate
        .strip_prefix("flatpak:")
        .map(crate::emulator_detect::is_flatpak_installed)
        .unwrap_or_else(|| executable_available(candidate))
}

fn executable_available(candidate: &str) -> bool {
    Path::new(candidate).is_file() || which::which(candidate).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(path: &str) -> crate::emulator_detect::DetectedEmulator {
        crate::emulator_detect::DetectedEmulator {
            display_name: String::new(),
            launch_command: path.to_string(),
        }
    }

    fn existing_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ira-ps4-versions-test-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("shadps4");
        std::fs::write(&f, b"").unwrap();
        f
    }

    #[test]
    fn test_pick_uses_per_game_pin_when_known_and_existing() {
        let path = existing_file("pergame");
        let versions = [version(path.to_str().unwrap())];
        let exe = pick_shadps4_executable(path.to_str().unwrap(), "", &versions, None);
        assert_eq!(exe, path.to_str().unwrap());
    }

    #[test]
    fn test_pick_skips_stale_per_game_pin_falls_back_to_detected() {
        let stale = existing_file("stale");
        let current = existing_file("detected");
        let versions = [version(current.to_str().unwrap())];
        let exe = pick_shadps4_executable(
            stale.to_str().unwrap(),
            "",
            &versions,
            Some(current.to_str().unwrap()),
        );
        assert_eq!(exe, current.to_str().unwrap());
    }

    #[test]
    fn test_pick_uses_global_pin_when_per_game_empty() {
        let path = existing_file("global");
        let versions = [version(path.to_str().unwrap())];
        let exe = pick_shadps4_executable("", path.to_str().unwrap(), &versions, None);
        assert_eq!(exe, path.to_str().unwrap());
    }

    #[test]
    fn test_pick_falls_back_to_shadps4_when_nothing_matches() {
        let versions = [version("/does/not/exist")];
        let exe = pick_shadps4_executable("", "", &versions, None);
        assert_eq!(exe, "shadps4");
    }

    #[test]
    fn test_pick_strips_quotes_from_pins() {
        let path = existing_file("quoted");
        let versions = [version(path.to_str().unwrap())];
        let quoted = format!("\"{}\"", path.display());
        let exe = pick_shadps4_executable("", &quoted, &versions, None);
        assert_eq!(exe, path.to_str().unwrap());
    }

    #[test]
    fn test_pick_skips_uninstalled_flatpak_pin() {
        let versions = [version("/does/not/exist")];
        let exe = pick_shadps4_executable("flatpak:invalid.ira.Test", "", &versions, None);
        assert_eq!(exe, "shadps4");
    }
}
