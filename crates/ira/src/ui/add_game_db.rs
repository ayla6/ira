use ira_models::{GameLaunchConfig, WineConfig};

pub(super) struct AddGameToDbParams<'a> {
    pub(super) db: &'a ira_db::DbConn,
    pub(super) name: &'a str,
    pub(super) kind: ira_models::GameKind,
    pub(super) trophy_source: ira_models::TrophySource,
    pub(super) app_id: &'a str,
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
        game_folder,
        launch_config,
        wine_config,
        profile_id,
        steam,
        save_dir,
    } = params;
    // The platform is the kind's system, never the typed id. The typed id
    // lands by source: a Steam app id is both match and native id, a GOG
    // product id is native only (its Steam id, if known, arrives through
    // matching), anything else is a native marker.
    let (steam_id, native_id) = match trophy_source {
        ira_models::TrophySource::Gse | ira_models::TrophySource::SteamNative => (app_id, app_id),
        _ => ("", app_id),
    };
    let game_id = ira_db::add_game(
        db,
        ira_db::NewGame {
            kind,
            trophy_source,
            steam_id,
            trophy_id: "",
            native_id,
            platform_id: kind.as_str(),
            title: name,
        },
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
