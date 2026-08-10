use ira_models::{GameLaunchConfig, WineConfig};

pub(super) struct AddGameToDbParams<'a> {
    pub(super) db: &'a ira_db::DbConn,
    pub(super) name: &'a str,
    pub(super) kind: ira_models::GameKind,
    pub(super) trophy_source: ira_models::TrophySource,
    pub(super) app_id: &'a str,
    pub(super) platform_id: &'a str,
    pub(super) game_folder: &'a str,
    pub(super) launch_config: &'a GameLaunchConfig,
    pub(super) wine_config: &'a WineConfig,
    pub(super) profile_id: Option<i64>,
    pub(super) steam: &'a ira_api::SteamDataClient,
    pub(super) save_dir: &'a str,
}

pub(super) fn add_game_to_db(params: AddGameToDbParams) -> Result<i64, String> {
    let AddGameToDbParams {
        db,
        name,
        kind,
        trophy_source,
        app_id,
        platform_id,
        game_folder,
        launch_config,
        wine_config,
        profile_id,
        steam,
        save_dir,
    } = params;
    let (steam_id, game_id) = if trophy_source.has_steam_enrichment() {
        (app_id, "")
    } else {
        ("", app_id)
    };
    let game_id = ira_db::add_game(
        db,
        kind,
        trophy_source,
        steam_id,
        game_id,
        platform_id,
        name,
    )?;
    ira_db::save_game_config(db, game_id, launch_config, wine_config, profile_id)?;
    if !game_folder.is_empty() {
        if let Err(e) = ira_db::update_game_folder(db, game_id, game_folder) {
            eprintln!("Failed to set game_folder: {}", e);
        }
    }
    if !app_id.is_empty() && app_id.parse::<i64>().is_ok() {
        let folder = if !game_folder.is_empty() {
            game_folder.to_string()
        } else {
            std::path::Path::new(&launch_config.exe)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        if !folder.is_empty() {
            let _ = ira_platforms::steam::add_game_from_folder(
                &folder, app_id, kind, steam, db, save_dir,
            );
        }
    }
    Ok(game_id)
}
