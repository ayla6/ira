use std::path::Path;

use ira_db::DbConn;
use ira_api::SteamDataClient;
use ira_parser::GALAXY_ID;
use crate::gog::generate_galaxy_emu_config;

pub fn add_gog_game_from_folder(
    galaxy_dll_folder: &str,
    product_id: &str,
    game_name: &str,
    steam_app_id: &str,
    steam: &SteamDataClient,
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
    let data_ach_dir = ira_parser::achievements_dir(save_dir, steam_app_id);
    std::fs::create_dir_all(&data_ach_dir).map_err(|e| format!("could not create data achievements dir: {}", e))?;

    // Fetch achievement definitions from Steam API
    steam.generate_steam_settings(steam_app_id)?;

    // Add to DB
    ira_db::add_game(db, ira_models::GameKind::Wine, ira_models::TrophySource::Nge, steam_app_id, "", product_id, game_name)?;

    Ok(gog_game_dir.to_string_lossy().into_owned())
}
