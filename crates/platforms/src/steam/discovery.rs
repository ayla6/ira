use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::paths;
use super::vdf::{self, VdfValue};

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
    for value in folders.values() {
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
        .or_else(|| vdf::get_nested_str(&parsed, "UserConfig", "name"))
        .unwrap_or("")
        .to_string();
    let installdir = vdf::get_str(&parsed, "installdir")
        .unwrap_or("")
        .to_string();
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

    Some(SteamGame {
        app_id,
        name,
        install_dir,
    })
}

/// Discover all installed Steam games across all library folders.
/// Filters out non-game apps (tools, runtimes, etc.) using Steam's appinfo.vdf.
pub fn discover_games() -> Vec<SteamGame> {
    let mut games = Vec::new();

    let mut manifests = Vec::new();
    for apps_dir in all_steamapps_dirs() {
        let Ok(entries) = std::fs::read_dir(&apps_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name_str = entry.file_name().to_string_lossy().into_owned();
            if !vdf::is_app_manifest_file_name(&name_str) {
                continue;
            }
            if let Some(game) = parse_app_manifest(&entry.path()) {
                manifests.push(game);
            }
        }
    }

    let manifests = deduplicate_by_app_id(manifests);
    if manifests.is_empty() {
        return games;
    }

    let app_ids: Vec<u32> = manifests
        .iter()
        .filter_map(|g| g.app_id.parse::<u32>().ok())
        .collect();
    let app_types = super::appinfo::get_app_types(&app_ids);

    for game in manifests {
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

fn deduplicate_by_app_id(games: Vec<SteamGame>) -> Vec<SteamGame> {
    let mut seen = HashSet::new();
    games
        .into_iter()
        .filter(|game| seen.insert(game.app_id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{deduplicate_by_app_id, parse_app_manifest, SteamGame};

    #[test]
    fn test_deduplicate_by_app_id_keeps_one_manifest() {
        let games = deduplicate_by_app_id(vec![
            SteamGame {
                app_id: "123".to_string(),
                name: "Game".to_string(),
                install_dir: "/games/one".into(),
            },
            SteamGame {
                app_id: "123".to_string(),
                name: "Duplicate".to_string(),
                install_dir: "/games/two".into(),
            },
            SteamGame {
                app_id: "456".to_string(),
                name: "Other".to_string(),
                install_dir: "/games/other".into(),
            },
        ]);

        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "Game");
        assert_eq!(games[1].app_id, "456");
    }

    #[test]
    fn test_parse_app_manifest_reads_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appmanifest_440.acf");
        std::fs::write(
            &path,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"440\"\n\t\"name\"\t\t\"Team Fortress 2\"\n\t\"installdir\"\t\t\"Team Fortress 2\"\n\t\"StateFlags\"\t\t\"4\"\n}",
        )
        .unwrap();
        let game = parse_app_manifest(&path).unwrap();
        assert_eq!(game.app_id, "440");
        assert_eq!(game.name, "Team Fortress 2");
        assert!(game.install_dir.ends_with("common/Team Fortress 2"));
    }

    #[test]
    fn test_parse_app_manifest_falls_back_to_userconfig_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appmanifest_440.acf");
        std::fs::write(
            &path,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"440\"\n\t\"UserConfig\"\n\t{\n\t\t\"name\"\t\t\"Fallback Name\"\n\t}\n\t\"installdir\"\t\t\"Game\"\n\t\"StateFlags\"\t\t\"4\"\n}",
        )
        .unwrap();
        let game = parse_app_manifest(&path).unwrap();
        assert_eq!(game.name, "Fallback Name");
    }

    #[test]
    fn test_parse_app_manifest_skips_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appmanifest_440.acf");
        std::fs::write(
            &path,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"440\"\n\t\"name\"\t\t\"Game\"\n\t\"installdir\"\t\t\"Game\"\n\t\"StateFlags\"\t\t\"2\"\n}",
        )
        .unwrap();
        assert!(parse_app_manifest(&path).is_none());
    }
}
