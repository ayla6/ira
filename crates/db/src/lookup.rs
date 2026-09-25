use crate::{err, lock_db, DbConn};
use ira_models::{GameEntry, GameKind};
use rusqlite::params;

fn find_all_games_by(
    conn: &DbConn,
    where_clause: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT {} FROM games WHERE {}",
            crate::GAME_COLUMNS,
            where_clause
        ))
        .map_err(err)?;
    let entries = stmt
        .query_map(params, crate::game_entry_from_row)
        .map_err(err)?;
    entries.collect::<Result<Vec<_>, _>>().map_err(err)
}

fn find_game_by(
    conn: &DbConn,
    where_clause: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Option<GameEntry>, String> {
    Ok(find_all_games_by(conn, where_clause, params)?
        .into_iter()
        .next())
}

/// The row carrying a Steam id on one platform: several platforms may
/// point at the same store entry (one Steam port, many console
/// releases), so the platform scopes the lookup — like the trophy and
/// native id lookups, never global.
pub fn find_by_steam_id(
    conn: &DbConn,
    steam_id: &str,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "steam_id = ?1 AND platform_id = ?2",
        params![steam_id, platform_id],
    )
}

pub fn find_by_trophy_id(
    conn: &DbConn,
    trophy_id: &str,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "trophy_id = ?1 AND platform_id = ?2",
        params![trophy_id, platform_id],
    )
}

pub fn find_by_native_id(
    conn: &DbConn,
    native_id: &str,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "native_id = ?1 AND platform_id = ?2",
        params![native_id, platform_id],
    )
}

pub fn find_by_db_id(conn: &DbConn, db_id: i64) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "id = ?1", params![db_id])
}

pub fn find_by_game_folder(conn: &DbConn, game_folder: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "game_folder = ?1 AND game_folder != ''",
        params![game_folder],
    )
}

/// Every ROM-library entry of one console platform: the generic Retro
/// kind plus Switch, the one ROM-folder console with a kind of its own.
pub fn find_all_rom_by_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<Vec<GameEntry>, String> {
    find_all_games_by(
        conn,
        "kind IN (?1, ?2) AND platform_id = ?3",
        params![
            GameKind::Retro.as_str(),
            GameKind::Switch.as_str(),
            platform_id
        ],
    )
}

/// Cached API-emulator DLL folder for the game (empty string if unknown).
pub fn get_api_dll_folder(conn: &DbConn, game_id: i64) -> Result<String, String> {
    let c = lock_db(conn)?;
    c.query_row(
        "SELECT api_dll_folder FROM games WHERE id = ?1",
        params![game_id],
        |r| r.get(0),
    )
    .map_err(err)
}

/// Whether the game's UFS saves are known to be centralized.
pub fn get_saves_centralized(conn: &DbConn, game_id: i64) -> Result<bool, String> {
    let c = lock_db(conn)?;
    c.query_row(
        "SELECT saves_centralized FROM games WHERE id = ?1",
        params![game_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|v| v != 0)
    .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::super::add_game;
    use super::super::init_db;
    use super::super::load_all_games;
    use super::*;
    use ira_models::TrophySource;
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_get_all_games_returns_all() {
        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "1",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Game 1",
            },
        )
        .unwrap();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "2",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Game 2",
            },
        )
        .unwrap();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Ra,
                steam_id: "",
                trophy_id: "",
                native_id: "r1",
                platform_id: "nes",
                title: "Game 3",
            },
        )
        .unwrap();
        let games = load_all_games(&conn).unwrap();
        assert_eq!(games.len(), 3);
    }

    #[test]
    fn test_find_all_rom_by_platform_covers_switch_kind() {
        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Switch,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "010051f0207b2000",
                platform_id: "switch",
                title: "Switch game",
            },
        )
        .unwrap();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "legacy",
                platform_id: "switch",
                title: "Legacy switch rom",
            },
        )
        .unwrap();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Switch,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "0100000000010000",
                platform_id: "not-a-rom-console",
                title: "Other platform",
            },
        )
        .unwrap();

        let switch_games = find_all_rom_by_platform(&conn, "switch").unwrap();
        let mut ids: Vec<&str> = switch_games.iter().map(|g| g.native_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["010051f0207b2000", "legacy"]);
    }

    #[test]
    fn test_find_by_db_id_returns_correct_game() {
        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "1",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Game 1",
            },
        )
        .unwrap();
        let id2 = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "2",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Game 2",
            },
        )
        .unwrap();
        let game = find_by_db_id(&conn, id2).unwrap().unwrap();
        assert_eq!(game.title, "Game 2");
        assert_eq!(game.steam_id, "2");
    }

    #[test]
    fn test_find_by_steam_id_returns_correct_game() {        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "100",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Steam Game",
            },
        )
        .unwrap();
        add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "200",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Other Game",
            },
        )
        .unwrap();
        let game = find_by_steam_id(&conn, "100", "").unwrap().unwrap();
        assert_eq!(game.steam_id, "100");
        assert_eq!(game.title, "Steam Game");
    }

    #[test]
    fn test_find_by_steam_id_scopes_per_platform() {
        // One Steam port, two console releases: both rows share the
        // store id and the platform tells them apart.
        let (conn, _tmp) = setup_db();
        for (platform, title) in [("psx", "Chrono Trigger"), ("nds", "Chrono Trigger")] {
            add_game(
                &conn,
                crate::NewGame {
                    kind: GameKind::Retro,
                    trophy_source: TrophySource::Empty,
                    steam_id: "398850",
                    trophy_id: "",
                    native_id: "",
                    platform_id: platform,
                    title,
                },
            )
            .unwrap();
        }
        let psx = find_by_steam_id(&conn, "398850", "psx").unwrap().unwrap();
        assert_eq!(psx.platform_id, "psx");
        let nds = find_by_steam_id(&conn, "398850", "nds").unwrap().unwrap();
        assert_eq!(nds.platform_id, "nds");
        assert!(find_by_steam_id(&conn, "398850", "steam").unwrap().is_none());
    }

    #[test]
    fn test_find_by_db_id_nonexistent_returns_none() {
        let (conn, _tmp) = setup_db();
        let game = find_by_db_id(&conn, 999).unwrap();
        assert!(game.is_none());
    }

    #[test]
    fn test_find_by_game_folder_returns_match() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Wine,
                trophy_source: TrophySource::Gse,
                steam_id: "555",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Folder Game",
            },
        )
        .unwrap();
        super::super::update_game_folder(&conn, id, "/games/MyGame").unwrap();
        let game = find_by_game_folder(&conn, "/games/MyGame")
            .unwrap()
            .unwrap();
        assert_eq!(game.id, id);
        assert_eq!(game.game_folder, "/games/MyGame");
    }

    #[test]
    fn test_find_by_game_folder_empty_returns_none() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Linux,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "No Folder",
            },
        )
        .unwrap();
        // game_folder defaults to "" — must NOT match an empty-string query
        let game = find_by_game_folder(&conn, "").unwrap();
        assert!(game.is_none());
        let _ = id;
    }
}
