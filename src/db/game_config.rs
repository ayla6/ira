use crate::db::DbConn;
use crate::models::{GameLaunchConfig, WineConfig};
use rusqlite::params;

pub fn get_game_config(conn: &DbConn, game_id: i64) -> Result<Option<(GameLaunchConfig, WineConfig)>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c
        .prepare("SELECT launch_config, wine_config FROM game_configs WHERE game_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map(params![game_id], |row| {
        let launch_str: String = row.get(0)?;
        let wine_str: String = row.get(1)?;
        Ok((launch_str, wine_str))
    })
    .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(Ok((launch_str, wine_str))) => {
            let launch: GameLaunchConfig = if launch_str.is_empty() {
                GameLaunchConfig::default()
            } else {
                serde_json::from_str(&launch_str).map_err(|e| e.to_string())?
            };
            let wine: WineConfig = if wine_str.is_empty() {
                WineConfig::default()
            } else {
                serde_json::from_str(&wine_str).map_err(|e| e.to_string())?
            };
            Ok(Some((launch, wine)))
        }
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

pub fn save_game_config(
    conn: &DbConn,
    game_id: i64,
    launch: &GameLaunchConfig,
    wine: &WineConfig,
) -> Result<(), String> {
    let launch_str = serde_json::to_string(launch).map_err(|e| e.to_string())?;
    let wine_str = serde_json::to_string(wine).map_err(|e| e.to_string())?;
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO game_configs (game_id, launch_config, wine_config) VALUES (?1, ?2, ?3)
         ON CONFLICT(game_id) DO UPDATE SET launch_config = excluded.launch_config, wine_config = excluded.wine_config",
        params![game_id, launch_str, wine_str],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_game_config(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM game_configs WHERE game_id = ?1", params![game_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_save_and_get_game_config() {
        let (conn, _tmp) = setup_db();
        let launch = GameLaunchConfig::default();
        let wine = WineConfig::default();

        save_game_config(&conn, 1, &launch, &wine).unwrap();
        let result = get_game_config(&conn, 1).unwrap();
        assert!(result.is_some());
        let (_saved_launch, saved_wine) = result.unwrap();
        assert!(saved_wine.enabled);
        assert_eq!(saved_wine.version, "system");
    }

    #[test]
    fn test_get_nonexistent_config() {
        let (conn, _tmp) = setup_db();
        let result = get_game_config(&conn, 999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_existing_config() {
        let (conn, _tmp) = setup_db();
        let launch = GameLaunchConfig::default();
        let mut wine = WineConfig::default();
        wine.version = "ge-proton".to_string();
        save_game_config(&conn, 1, &launch, &wine).unwrap();

        let mut wine2 = WineConfig::default();
        wine2.version = "winehq-staging".to_string();
        save_game_config(&conn, 1, &launch, &wine2).unwrap();

        let result = get_game_config(&conn, 1).unwrap().unwrap();
        assert_eq!(result.1.version, "winehq-staging");
    }

    #[test]
    fn test_delete_game_config() {
        let (conn, _tmp) = setup_db();
        let launch = GameLaunchConfig::default();
        let wine = WineConfig::default();
        save_game_config(&conn, 1, &launch, &wine).unwrap();
        delete_game_config(&conn, 1).unwrap();
        let result = get_game_config(&conn, 1).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_config_roundtrip_full_fields() {
        let (conn, _tmp) = setup_db();
        let mut launch = GameLaunchConfig::default();
        launch.exe = "/home/user/game.exe".to_string();
        launch.args = "--windowed".to_string();
        launch.working_dir = "/home/user".to_string();
        launch.env_vars = vec![("MY_VAR".to_string(), "value".to_string())];

        let mut wine = WineConfig::default();
        wine.enabled = true;
        wine.prefix = "/home/user/wineprefix".to_string();
        wine.version = "winehq-devel".to_string();
        wine.esync = true;
        wine.dxvk = true;
        wine.vkd3d = false;
        wine.dll_overrides = vec![("d3d11".to_string(), "native,builtin".to_string())];

        save_game_config(&conn, 1, &launch, &wine).unwrap();
        let result = get_game_config(&conn, 1).unwrap().unwrap();

        assert_eq!(result.0.exe, "/home/user/game.exe");
        assert_eq!(result.0.env_vars[0].0, "MY_VAR");
        assert_eq!(result.1.prefix, "/home/user/wineprefix");
        assert_eq!(result.1.version, "winehq-devel");
        assert_eq!(result.1.dll_overrides[0].0, "d3d11");
    }
}
