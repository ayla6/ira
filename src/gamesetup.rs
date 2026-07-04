use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

use crate::db::DbConn;
use crate::parser::GALAXY_ID;
use crate::steam::SteamClient;

/// Detect a Steam/Goldberg game by looking for steam_appid.txt.
pub fn detect_app_id(folder: &str) -> Option<String> {
    let candidates = [
        Path::new(folder).join("steam_appid.txt"),
        Path::new(folder).join("steam_settings").join("steam_appid.txt"),
    ];
    for p in &candidates {
        if let Ok(data) = std::fs::read_to_string(p) {
            let id = data.trim();
            if !id.is_empty() && id.parse::<i64>().is_ok() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Check if a folder contains Galaxy.dll or Galaxy64.dll (case-insensitive).
pub fn is_gog_game(folder: &str) -> bool {
    find_galaxy_dll(folder).is_some()
}

fn find_galaxy_dll(folder: &str) -> Option<PathBuf> {
    let dll_names = ["galaxy.dll", "galaxy64.dll"];
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if dll_names.contains(&name.to_lowercase().as_str()) {
                    return Some(Path::new(folder).to_path_buf());
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub = entry.path();
                if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                    for se in sub_entries.flatten() {
                        if let Some(name) = se.file_name().to_str() {
                            if dll_names.contains(&name.to_lowercase().as_str()) {
                                return Some(sub);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Walk up from `start` looking for a directory containing goggame-*.info files.
/// Returns (info_dir, product_id, name).
pub fn find_gog_info(start: &str) -> Option<(PathBuf, String, String)> {
    let mut current = Path::new(start).to_path_buf();
    loop {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("goggame-") && name.ends_with(".info") {
                        if let Ok(data) = std::fs::read_to_string(entry.path()) {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                let product_id = json.get("rootGameId")
                                    .or_else(|| json.get("gameId"))
                                    .and_then(|v| {
                                        if let Some(s) = v.as_str() {
                                            Some(s.to_string())
                                        } else {
                                            v.as_i64().map(|i| i.to_string())
                                        }
                                    });
                                let name = json
                                    .get("name")
                                    .or_else(|| json.get("title"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown GOG Game")
                                    .to_string();
                                if let Some(id) = product_id {
                                    return Some((current, id, name));
                                }
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
    let dll_names = ["galaxy.dll", "galaxy64.dll"];
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if dll_names.contains(&name.to_lowercase().as_str()) {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

fn checksum_dll(path: &Path) -> Option<u64> {
    let data = std::fs::read(path).ok()?;
    Some(xxh3_64(&data))
}

fn write_old_emu_config(settings_dir: &Path, product_id: &str) -> Result<(), String> {
    let config = serde_json::json!({
        "api_version": "1.152.1.0",
        "disable_crashdump": true,
        "disable_online_networking": false,
        "enable_lan": true,
        "enable_overlay": false,
        "galaxyid": GALAXY_ID.parse::<i64>().unwrap_or(100000000000000000),
        "ice_servers": [],
        "language": "english",
        "log_level": "off",
        "productid": product_id.parse::<i64>().unwrap_or(0),
        "savepath": "appdata",
        "signaling_servers": [],
        "unlock_dlcs": true,
        "username": "DefaultName"
    });
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(settings_dir.join("NemirtingasGalaxyEmu.json"), json).map_err(|e| e.to_string())
}

fn write_new_emu_config(settings_dir: &Path, product_id: &str) -> Result<(), String> {
    let config = serde_json::json!({
        "GalaxyEmu": {
            "Application": {
                "ApiVersion": "1.152.11.0",
                "AppId": product_id.parse::<i64>().unwrap_or(0),
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
                "GalaxyId": GALAXY_ID.parse::<i64>().unwrap_or(100000000000000000),
                "Language": "english",
                "Languages": ["english"],
                "UserName": ""
            }
        }
    });
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(settings_dir.join("NemirtingasGalaxyEmu.json"), json).map_err(|e| e.to_string())
}

/// Generate NemirtingasGalaxyEmu.json in the game's ngalaxye_settings folder.
/// Uses XXH3 checksum of the Galaxy DLL to determine old vs new config format.
fn generate_galaxy_emu_config(galaxy_dll_folder: &str, product_id: &str) -> Result<(), String> {
    let settings_dir = Path::new(galaxy_dll_folder).join("ngalaxye_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("could not create ngalaxye_settings: {}", e))?;

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

// ---- Game setup ----

pub fn add_game_from_folder(
    folder: &str,
    app_id: &str,
    steam: &SteamClient,
    db: &DbConn,
    save_dir: &str,
) -> Result<String, String> {
    let folder = folder.trim();
    let app_id = app_id.trim();
    if folder.is_empty() {
        return Err("no folder selected".into());
    }
    if app_id.is_empty() || app_id.parse::<i64>().is_err() {
        return Err(format!("invalid Steam App ID {:?}", app_id));
    }

    // Ensure the game's steam_settings exists (Goldberg emulator needs it)
    let game_settings_dir = Path::new(folder).join("steam_settings");
    std::fs::create_dir_all(&game_settings_dir).map_err(|e| format!("could not create steam_settings: {}", e))?;

    let app_id_path = game_settings_dir.join("steam_appid.txt");
    if !app_id_path.exists() {
        std::fs::write(&app_id_path, app_id).map_err(|e| format!("could not write steam_appid.txt: {}", e))?;
    }

    // Create data/<app_id>/achievements/ as symlink to game's steam_settings
    let data_ach_dir = crate::parser::achievements_dir(save_dir, app_id);
    if !data_ach_dir.exists() {
        std::fs::create_dir_all(data_ach_dir.parent().unwrap()).map_err(|e| format!("could not create data dir: {}", e))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&game_settings_dir, &data_ach_dir).map_err(|e| format!("could not symlink achievements: {}", e))?;
    }

    // Create steam/<app_id>/ save directory
    let saves_game_dir = Path::new(save_dir).join("steam").join(app_id);
    std::fs::create_dir_all(&saves_game_dir).map_err(|e| format!("could not create saves directory: {}", e))?;

    // Generate achievement definitions
    steam.generate_steam_settings(app_id)?;

    // Add to DB
    let title = std::fs::read_to_string(game_settings_dir.join("title.txt"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    crate::db::add_game(db, "steam", app_id, app_id, &title)?;

    Ok(saves_game_dir.to_string_lossy().into_owned())
}

pub fn add_gog_game_from_folder(
    galaxy_dll_folder: &str,
    product_id: &str,
    game_name: &str,
    steam_app_id: &str,
    steam: &SteamClient,
    db: &DbConn,
    save_dir: &str,
) -> Result<String, String> {
    let steam_app_id = steam_app_id.trim();
    if steam_app_id.is_empty() || steam_app_id.parse::<i64>().is_err() {
        return Err(format!("invalid Steam App ID {:?}", steam_app_id));
    }

    // Generate NemirtingasGalaxyEmu.json in the game's ngalaxye_settings
    generate_galaxy_emu_config(galaxy_dll_folder, product_id)?;

    // Create GOG saves directory: save_dir/gog/<GALAXY_ID>/<product_id>/
    let gog_game_dir = Path::new(save_dir)
        .join("gog")
        .join(GALAXY_ID)
        .join(product_id);
    std::fs::create_dir_all(&gog_game_dir).map_err(|e| format!("could not create GOG saves dir: {}", e))?;

    // Create empty achievements.json so the parser has something to read
    let ach_path = gog_game_dir.join("achievements.json");
    if !ach_path.exists() {
        std::fs::write(&ach_path, "{}").map_err(|e| format!("could not write achievements.json: {}", e))?;
    }

    // Create data/<steam_app_id>/achievements/ (real folder for GOG games)
    let data_ach_dir = crate::parser::achievements_dir(save_dir, steam_app_id);
    std::fs::create_dir_all(&data_ach_dir).map_err(|e| format!("could not create data achievements dir: {}", e))?;

    // Write title.txt
    let title_path = data_ach_dir.join("title.txt");
    if !title_path.exists() {
        std::fs::write(&title_path, game_name).map_err(|e| format!("could not write title.txt: {}", e))?;
    }

    // Fetch achievement definitions from Steam API
    steam.generate_steam_settings(steam_app_id)?;

    // Add to DB
    crate::db::add_game(db, "gog", steam_app_id, product_id, game_name)?;

    Ok(gog_game_dir.to_string_lossy().into_owned())
}
