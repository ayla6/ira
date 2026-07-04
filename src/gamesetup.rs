use std::path::{Path, PathBuf};

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
    // Check the folder itself
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if dll_names.contains(&name.to_lowercase().as_str()) {
                    return Some(Path::new(folder).to_path_buf());
                }
            }
        }
    }
    // Check one level of subdirectories
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

/// Walk up from `start` looking for a `ngalaxye_settings` directory containing
/// `NemirtingasGalaxyEmu.json`. Returns the path to the `ngalaxye_settings` folder.
pub fn find_ngalaxye_settings(start: &str) -> Option<PathBuf> {
    let mut current = Path::new(start).to_path_buf();
    loop {
        let candidate = current.join("ngalaxye_settings").join("NemirtingasGalaxyEmu.json");
        if candidate.exists() {
            return Some(current.join("ngalaxye_settings"));
        }
        if !current.pop() {
            break;
        }
    }
    None
}

/// Read NemirtingasGalaxyEmu.json from the given ngalaxye_settings folder.
/// Handles both the old flat format and the new nested format.
pub fn read_galaxy_emu_config(settings_dir: &Path) -> Option<(String, String)> {
    let config_path = settings_dir.join("NemirtingasGalaxyEmu.json");
    let data = std::fs::read_to_string(&config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;

    // Try old flat format: { "galaxyid": N, "productid": N }
    if let Some(gid) = json.get("galaxyid").and_then(|v| v.as_i64()) {
        let pid = json.get("productid").and_then(|v| v.as_i64())?;
        return Some((gid.to_string(), pid.to_string()));
    }

    // Try new nested format: { "GalaxyEmu": { "Application": { "AppId": N }, "User": { "GalaxyId": N } } }
    let galaxy_emu = json.get("GalaxyEmu")?;
    let app_id = galaxy_emu
        .get("Application")
        .and_then(|a| a.get("AppId"))
        .and_then(|v| v.as_i64())?;
    let galaxy_id = galaxy_emu
        .get("User")
        .and_then(|u| u.get("GalaxyId"))
        .and_then(|v| v.as_i64())?;
    Some((galaxy_id.to_string(), app_id.to_string()))
}

pub fn add_game_from_folder(
    folder: &str,
    app_id: &str,
    steam: &crate::steam::SteamClient,
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

    let settings_dir = Path::new(folder).join("steam_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("could not create steam_settings: {}", e))?;

    let app_id_path = settings_dir.join("steam_appid.txt");
    if !app_id_path.exists() {
        std::fs::write(&app_id_path, app_id).map_err(|e| format!("could not write steam_appid.txt: {}", e))?;
    }

    let saves_game_dir = Path::new(save_dir).join("steam").join(app_id);
    std::fs::create_dir_all(save_dir).map_err(|e| format!("could not create saves directory: {}", e))?;

    let link_path = saves_game_dir.join("steam_settings");
    if !link_path.exists() {
        std::fs::create_dir_all(&saves_game_dir).map_err(|e| format!("could not create game save directory: {}", e))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&settings_dir, &link_path).map_err(|e| format!("could not symlink steam_settings into saves: {}", e))?;
    }

    steam.generate_steam_settings(app_id, &saves_game_dir)?;

    Ok(saves_game_dir.to_string_lossy().into_owned())
}

/// Error sentinel used to distinguish "emulator config not found" from other errors.
pub const EMU_NOT_FOUND: &str = "__EMU_NOT_FOUND__";

/// Set up a GOG game: symlinks the emulator's achievement data into the GOG saves tree
/// and creates steam_settings + steam_appid.txt for definitions.
pub fn add_gog_game_from_folder(
    galaxy_dll_folder: &str,
    _game_root_folder: &str,
    product_id: &str,
    game_name: &str,
    steam_app_id: &str,
    steam: &crate::steam::SteamClient,
    save_dir: &str,
) -> Result<String, String> {
    let steam_app_id = steam_app_id.trim();
    if steam_app_id.is_empty() || steam_app_id.parse::<i64>().is_err() {
        return Err(format!("invalid Steam App ID {:?}", steam_app_id));
    }

    // Find ngalaxye_settings by walking up directories from the Galaxy.dll folder
    let ngalaxye_src = find_ngalaxye_settings(galaxy_dll_folder)
        .ok_or_else(|| EMU_NOT_FOUND.to_string())?;

    // Read galaxyid from NemirtingasGalaxyEmu.json
    let (galaxyid, _config_productid) = read_galaxy_emu_config(&ngalaxye_src)
        .ok_or_else(|| "Could not parse NemirtingasGalaxyEmu.json in ngalaxye_settings.".to_string())?;

    // Create GOG saves directory: save_dir/gog/<galaxyid>/<product_id>/
    let gog_game_dir = Path::new(save_dir)
        .join("gog")
        .join(&galaxyid)
        .join(product_id);
    std::fs::create_dir_all(&gog_game_dir).map_err(|e| format!("could not create GOG saves dir: {}", e))?;

    // Symlink ngalaxye_settings from the found location into the GOG saves tree
    let ngalaxye_dst = gog_game_dir.join("ngalaxye_settings");
    if ngalaxye_src.exists() && !ngalaxye_dst.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ngalaxye_src, &ngalaxye_dst).map_err(|e| format!("could not symlink ngalaxye_settings: {}", e))?;
    }

    // Create an empty achievements.json so the parser has something to read
    // before the emulator writes to it on first launch.
    let ach_path = gog_game_dir.join("achievements.json");
    if !ach_path.exists() {
        std::fs::write(&ach_path, "{}").map_err(|e| format!("could not write achievements.json: {}", e))?;
    }

    // Create steam_settings for achievement definitions (fetched from Steam API)
    let settings_dir = gog_game_dir.join("steam_settings");
    std::fs::create_dir_all(&settings_dir).map_err(|e| format!("could not create steam_settings: {}", e))?;

    // Write steam_appid.txt so load_games can find the Steam App ID
    let appid_path = gog_game_dir.join("steam_appid.txt");
    std::fs::write(&appid_path, steam_app_id).map_err(|e| format!("could not write steam_appid.txt: {}", e))?;

    // Write title.txt with the game name from the .info file
    let title_path = settings_dir.join("title.txt");
    if !title_path.exists() {
        std::fs::write(&title_path, game_name).map_err(|e| format!("could not write title.txt: {}", e))?;
    }

    // Fetch achievement definitions from Steam API
    steam.generate_steam_settings(steam_app_id, &gog_game_dir)?;

    Ok(gog_game_dir.to_string_lossy().into_owned())
}
