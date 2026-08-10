use std::path::Path;

use ira_api::SteamDataClient;
use ira_db::DbConn;

/// Detect a Steam/Goldberg game by looking for steam_appid.txt.
pub fn detect_app_id(folder: &str) -> Option<String> {
    let candidates = [
        Path::new(folder).join("steam_appid.txt"),
        Path::new(folder)
            .join("steam_settings")
            .join("steam_appid.txt"),
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
    kind: ira_models::GameKind,
    steam: &SteamDataClient,
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
    std::fs::create_dir_all(&game_settings_dir)
        .map_err(|e| format!("could not create steam_settings: {}", e))?;

    let app_id_path = game_settings_dir.join("steam_appid.txt");
    if !app_id_path.exists() {
        std::fs::write(&app_id_path, app_id)
            .map_err(|e| format!("could not write steam_appid.txt: {}", e))?;
    }

    // Create data/<app_id>/achievements/ as symlink to game's steam_settings
    let data_ach_dir = ira_parser::achievements_dir(save_dir, app_id);
    if !data_ach_dir.exists() {
        std::fs::create_dir_all(data_ach_dir.parent().unwrap())
            .map_err(|e| format!("could not create data dir: {}", e))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&game_settings_dir, &data_ach_dir)
            .map_err(|e| format!("could not symlink achievements: {}", e))?;
    }

    // Create emulator_saves/gbe/<app_id>/ save directory
    let saves_game_dir = Path::new(save_dir)
        .join("emulator_saves")
        .join("gbe")
        .join(app_id);
    std::fs::create_dir_all(&saves_game_dir)
        .map_err(|e| format!("could not create saves directory: {}", e))?;

    // Generate achievement definitions
    steam.generate_steam_settings(app_id)?;

    // Add to DB (title will be filled from appdetails.json during enrichment)
    ira_db::add_game(
        db,
        kind,
        ira_models::TrophySource::Gse,
        app_id,
        "",
        app_id,
        "",
    )?;

    Ok(saves_game_dir.to_string_lossy().into_owned())
}
