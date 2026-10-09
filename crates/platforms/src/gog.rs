use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

use ira_parser::GALAXY_ID;

pub fn is_gog_game(folder: &str) -> bool {
    find_galaxy_dll(folder).is_some()
}

fn is_galaxy_dll_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    lower == "galaxy.dll" || lower == "galaxy64.dll"
}

fn find_galaxy_dll(folder: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return None;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if is_galaxy_dll_name(name) {
                return Some(Path::new(folder).to_path_buf());
            }
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            subdirs.push(entry.path());
        }
    }
    for sub in subdirs {
        let Ok(sub_entries) = std::fs::read_dir(&sub) else {
            continue;
        };
        for se in sub_entries.flatten() {
            if let Some(name) = se.file_name().to_str() {
                if is_galaxy_dll_name(name) {
                    return Some(sub.clone());
                }
            }
        }
    }
    None
}

/// Accepts string or numeric ids, `rootGameId`/`gameId` and `name`/`title` keys.
fn parse_gog_info_json(data: &str) -> Option<(String, String)> {
    let json = serde_json::from_str::<serde_json::Value>(data).ok()?;
    let product_id = json
        .get("rootGameId")
        .or_else(|| json.get("gameId"))
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                v.as_i64().map(|i| i.to_string())
            }
        })?;
    let name = json
        .get("name")
        .or_else(|| json.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown GOG Game")
        .to_string();
    Some((product_id, name))
}

/// Returns (info_dir, product_id, name).
pub fn find_gog_info(start: &str) -> Option<(PathBuf, String, String)> {
    let mut current = Path::new(start).to_path_buf();
    loop {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("goggame-") && name.ends_with(".info") {
                        if let Ok(data) = std::fs::read_to_string(entry.path()) {
                            if let Some((product_id, game_name)) = parse_gog_info_json(&data) {
                                return Some((current, product_id, game_name));
                            }
                        }
                    }
                }
            }
        }
        if !current.pop() {
            break;
        }
    }
    None
}

// ---- GOG emulator config generation ----

const OLD_HASH_GALAXY64: u64 = 0xa511c62ff1db29d9;
const OLD_HASH_GALAXY: u64 = 0x64b254c0a68a2718;

fn find_galaxy_dll_file(folder: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return None;
    };
    entries.flatten().find_map(|entry| {
        let name = entry.file_name();
        let name = name.to_str()?;
        is_galaxy_dll_name(name).then(|| entry.path())
    })
}

fn checksum_dll(path: &Path) -> Option<u64> {
    let data = std::fs::read(path).ok()?;
    Some(xxh3_64(&data))
}

fn galaxy_user_id() -> i64 {
    GALAXY_ID.parse::<i64>().unwrap_or(100000000000000000)
}

fn parse_product_id(product_id: &str) -> i64 {
    product_id.parse::<i64>().unwrap_or(0)
}

fn write_emu_json(settings_dir: &Path, config: serde_json::Value) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(settings_dir.join("NemirtingasGalaxyEmu.json"), json).map_err(|e| e.to_string())
}

fn write_old_emu_config(settings_dir: &Path, product_id: &str) -> Result<(), String> {
    write_emu_json(
        settings_dir,
        serde_json::json!({
        "api_version": "1.152.1.0",
        "disable_crashdump": true,
        "disable_online_networking": false,
        "enable_lan": true,
        "enable_overlay": false,
        "galaxyid": galaxy_user_id(),
        "ice_servers": [],
        "language": "english",
        "log_level": "off",
        "productid": parse_product_id(product_id),
        "savepath": "appdata",
        "signaling_servers": [],
        "unlock_dlcs": true,
        "username": "DefaultName"
        }),
    )
}

fn write_new_emu_config(settings_dir: &Path, product_id: &str) -> Result<(), String> {
    write_emu_json(
        settings_dir,
        serde_json::json!({
        "GalaxyEmu": {
            "Application": {
                "ApiVersion": "1.152.11.0",
                "AppId": parse_product_id(product_id),
                "DisableOnlineNetworking": false,
                "LogLevel": "off",
                "SavePath": "appdata"
            },
            "Apps": {
                "DlcList": {},
                "UnlockDlcs": true
            },
            "Plugins": {
                "Overlay": {
                    "DelayDetection": "6s",
                    "Enabled": true
                }
            },
            "User": {
                "GalaxyId": galaxy_user_id(),
                "Language": "english",
                "Languages": ["english"],
                "UserName": ""
            }
        }
        }),
    )
}

/// Uses the Galaxy DLL's XXH3 checksum to pick old vs new config format.
pub fn generate_galaxy_emu_config(galaxy_dll_folder: &str, product_id: &str) -> Result<(), String> {
    let settings_dir = Path::new(galaxy_dll_folder).join("ngalaxye_settings");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("could not create ngalaxye_settings: {}", e))?;

    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    if config_path.exists() {
        return Ok(());
    }

    let use_old_format = find_galaxy_dll_file(galaxy_dll_folder)
        .and_then(|dll_path| checksum_dll(&dll_path))
        .map(|hash| hash == OLD_HASH_GALAXY64 || hash == OLD_HASH_GALAXY)
        .unwrap_or(false);

    if use_old_format {
        write_old_emu_config(&settings_dir, product_id)
    } else {
        write_new_emu_config(&settings_dir, product_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_gog_info_reads_manifest_in_start_dir() {
        let tmp = TempDir::new().unwrap();
        let game = tmp.path().join("Super Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(
            game.join("goggame-1207658924.info"),
            r#"{"buildId":51,"gameId":"1207658924","name":"Super Game"}"#,
        )
        .unwrap();

        let (dir, id, name) = find_gog_info(&game.to_string_lossy()).unwrap();
        assert_eq!(dir, game);
        assert_eq!(id, "1207658924");
        assert_eq!(name, "Super Game");
    }

    #[test]
    fn test_find_gog_info_walks_up_to_manifest_parent() {
        let tmp = TempDir::new().unwrap();
        let game = tmp.path().join("Super Game");
        std::fs::create_dir_all(game.join("bin")).unwrap();
        std::fs::write(
            game.join("goggame-1207658924.info"),
            r#"{"rootGameId":1207658924,"title":"Super Game"}"#,
        )
        .unwrap();

        let sub = game.join("bin");
        let (dir, id, name) = find_gog_info(&sub.to_string_lossy()).unwrap();
        assert_eq!(dir, game);
        assert_eq!(id, "1207658924");
        assert_eq!(name, "Super Game");
    }

    #[test]
    fn test_find_gog_info_none_without_manifest() {
        let tmp = TempDir::new().unwrap();
        assert!(find_gog_info(&tmp.path().to_string_lossy()).is_none());
    }

    #[test]
    fn test_is_galaxy_dll_name_matches_case_insensitive() {
        assert!(is_galaxy_dll_name("Galaxy.dll"));
        assert!(is_galaxy_dll_name("GALAXY64.DLL"));
        assert!(!is_galaxy_dll_name("galaxy.txt"));
        assert!(!is_galaxy_dll_name(""));
    }

    #[test]
    fn test_parse_gog_info_json_variants() {
        let (id, name) =
            parse_gog_info_json(r#"{"gameId":"42","name":"Game"}"#).unwrap();
        assert_eq!(id, "42");
        assert_eq!(name, "Game");
        let (id, name) =
            parse_gog_info_json(r#"{"rootGameId":99,"title":"Other"}"#).unwrap();
        assert_eq!(id, "99");
        assert_eq!(name, "Other");
        assert!(parse_gog_info_json(r#"{"name":"No id"}"#).is_none());
        assert!(parse_gog_info_json("not json").is_none());
    }

    #[test]
    fn test_parse_product_id_defaults_to_zero() {
        assert_eq!(parse_product_id("1207658924"), 1207658924);
        assert_eq!(parse_product_id("abc"), 0);
    }

    #[test]
    fn test_is_gog_game_detects_dll() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_gog_game(tmp.path().to_str().unwrap()));
        std::fs::write(tmp.path().join("Galaxy64.dll"), b"x").unwrap();
        assert!(is_gog_game(tmp.path().to_str().unwrap()));
    }
}
