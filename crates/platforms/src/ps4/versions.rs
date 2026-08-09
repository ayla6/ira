use std::path::{Path, PathBuf};

/// Path to the shadPS4 Qt Launcher data directory.
fn shadps4qt_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .map(|p| p.join("shadPS4QtLauncher"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local").join("share").join("shadPS4QtLauncher")
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
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }

    // Fallback: pick the first release (type 0) from versions.json
    let versions = read_shadps4_versions();
    for v in &versions {
        if v.version_type == 0 {
            let p = v.path.trim_matches('"').to_string();
            if !p.is_empty() {
                return Some(p);
            }
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
        &read_shadps4_versions(),
        detect_shadps4_version_path().as_deref(),
    )
}

fn pick_shadps4_executable(
    per_game: &str,
    global: &str,
    versions: &[ShadPs4Version],
    detected: Option<&str>,
) -> String {
    for candidate in [per_game, global] {
        let candidate = candidate.trim_matches('"');
        if candidate.is_empty() {
            continue;
        }
        let is_known = versions.iter().any(|v| v.path.trim_matches('"') == candidate);
        if is_known && Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    if let Some(p) = detected {
        let p = p.trim_matches('"');
        if !p.is_empty() && Path::new(p).exists() {
            return p.to_string();
        }
    }
    "shadps4".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(path: &str) -> ShadPs4Version {
        ShadPs4Version {
            codename: String::new(),
            name: String::new(),
            path: path.to_string(),
            version_type: 0,
            date: String::new(),
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
}
