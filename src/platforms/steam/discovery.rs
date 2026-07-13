use std::path::{Path, PathBuf};

use super::vdf::{self, VdfValue};
use super::paths;

/// A discovered Steam game.
pub struct SteamGame {
    pub app_id: String,
    pub name: String,
    pub install_dir: PathBuf,
}

/// Parse libraryfolders.vdf and return all library folder paths.
/// Handles both old (string values) and new (object with "path") formats.
pub fn parse_library_folders() -> Vec<PathBuf> {
    let Some(path) = paths::library_folders_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Some(VdfValue::Obj(folders)) = vdf::parse_vdf(&text) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for (_, value) in &folders {
        match value {
            VdfValue::Str(path) => {
                if Path::new(path).is_dir() {
                    result.push(PathBuf::from(path));
                }
            }
            VdfValue::Obj(obj) => {
                if let Some(VdfValue::Str(path)) = obj.get("path") {
                    if Path::new(path).is_dir() {
                        result.push(PathBuf::from(path));
                    }
                }
            }
        }
    }
    result
}

/// Get all steamapps directories — the main one plus any additional library folders.
fn all_steamapps_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(main) = paths::steamapps_dir() {
        dirs.push(main);
    }
    for lib in parse_library_folders() {
        let apps = lib.join("steamapps");
        if apps.is_dir() && !dirs.contains(&apps) {
            dirs.push(apps);
        }
    }
    dirs
}

/// Parse a single appmanifest_*.acf file.
fn parse_app_manifest(path: &Path) -> Option<SteamGame> {
    let text = std::fs::read_to_string(path).ok()?;
    let parsed = vdf::parse_vdf(&text)?;

    let app_id = vdf::get_str(&parsed, "appid")?.to_string();
    let name = vdf::get_str(&parsed, "name")
        .or_else(|| {
            vdf::get_obj(&parsed, "UserConfig")
                .and_then(|uc| uc.get("name"))
                .and_then(|v| match v {
                    VdfValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
        })
        .unwrap_or("")
        .to_string();
    let installdir = vdf::get_str(&parsed, "installdir").unwrap_or("").to_string();
    let state_flags: u32 = vdf::get_str(&parsed, "StateFlags")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    const FULLY_INSTALLED: u32 = 4;
    if state_flags & FULLY_INSTALLED == 0 {
        return None;
    }

    let install_dir = path
        .parent()
        .map(|p| p.join("common").join(&installdir))
        .unwrap_or_default();

    Some(SteamGame { app_id, name, install_dir })
}

/// Discover all installed Steam games across all library folders.
/// Filters out non-game apps (tools, runtimes, etc.) using Steam's appinfo.vdf.
pub fn discover_games() -> Vec<SteamGame> {
    let mut games = Vec::new();

    let mut all_manifests = Vec::new();
    for apps_dir in all_steamapps_dirs() {
        let Ok(entries) = std::fs::read_dir(&apps_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("appmanifest_") || !name_str.ends_with(".acf") {
                continue;
            }
            if name_str.ends_with(".tmp") {
                continue;
            }
            if let Some(game) = parse_app_manifest(&entry.path()) {
                all_manifests.push(game);
            }
        }
    }

    if all_manifests.is_empty() {
        return games;
    }

    let app_ids: Vec<u32> = all_manifests.iter()
        .filter_map(|g| g.app_id.parse::<u32>().ok())
        .collect();
    let app_types = super::appinfo::get_app_types(&app_ids);

    for game in all_manifests {
        if let Ok(app_id) = game.app_id.parse::<u32>() {
            if let Some(app_type) = app_types.get(&app_id) {
                if !super::appinfo::is_game_or_demo(app_type) {
                    continue;
                }
            }
        }
        games.push(game);
    }
    games
}
