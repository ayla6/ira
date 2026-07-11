use std::path::PathBuf;

/// Path to the shadPS4 Qt Launcher data directory.
fn shadps4qt_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .map(|b| b.get_data_home().join("shadPS4QtLauncher"))
        .unwrap_or_else(|_| {
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
