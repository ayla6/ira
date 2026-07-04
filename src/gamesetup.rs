use std::path::{Path, PathBuf};

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

    let saves_game_dir = Path::new(save_dir).join(app_id);
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
