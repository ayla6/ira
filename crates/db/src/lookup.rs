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

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "steam_id = ?1", params![steam_id])
}

pub fn find_by_game_id(
    conn: &DbConn,
    game_id: &str,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "game_id = ?1 AND platform_id = ?2",
        params![game_id, platform_id],
    )
}

pub fn find_by_db_id(conn: &DbConn, db_id: i64) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "id = ?1", params![db_id])
}

pub fn find_by_trophy_platform(
    conn: &DbConn,
    trophy_source: ira_models::TrophySource,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "trophy_source = ?1 AND platform_id = ?2",
        params![trophy_source.as_str(), platform_id],
    )
}

pub fn find_by_kind_platform(
    conn: &DbConn,
    kind: ira_models::GameKind,
    platform_id: &str,
) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "kind = ?1 AND platform_id = ?2",
        params![kind.as_str(), platform_id],
    )
}

pub fn find_by_game_folder(conn: &DbConn, game_folder: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(
        conn,
        "game_folder = ?1 AND game_folder != ''",
        params![game_folder],
    )
}

pub fn find_all_retro_by_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<Vec<GameEntry>, String> {
    find_all_games_by(
        conn,
        "kind = ?1 AND platform_id = ?2",
        params![GameKind::Retro.as_str(), platform_id],
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
            GameKind::Steam,
            TrophySource::Gse,
            "1",
            "",
            "",
            "Game 1",
        )
        .unwrap();
        add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "2",
            "",
            "",
            "Game 2",
        )
        .unwrap();
        add_game(
            &conn,
            GameKind::Retro,
            TrophySource::Ra,
            "",
            "r1",
            "nes",
            "Game 3",
        )
        .unwrap();
        let games = load_all_games(&conn).unwrap();
        assert_eq!(games.len(), 3);
    }

    #[test]
    fn test_find_by_db_id_returns_correct_game() {
        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "1",
            "",
            "",
            "Game 1",
        )
        .unwrap();
        let id2 = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "2",
            "",
            "",
            "Game 2",
        )
        .unwrap();
        let game = find_by_db_id(&conn, id2).unwrap().unwrap();
        assert_eq!(game.title, "Game 2");
        assert_eq!(game.steam_id, "2");
    }

    #[test]
    fn test_find_by_steam_id_returns_correct_game() {
        let (conn, _tmp) = setup_db();
        add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "100",
            "",
            "",
            "Steam Game",
        )
        .unwrap();
        add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "200",
            "",
            "",
            "Other Game",
        )
        .unwrap();
        let game = find_by_steam_id(&conn, "100").unwrap().unwrap();
        assert_eq!(game.steam_id, "100");
        assert_eq!(game.title, "Steam Game");
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
            GameKind::Wine,
            TrophySource::Gse,
            "555",
            "",
            "",
            "Folder Game",
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
            GameKind::Linux,
            TrophySource::Empty,
            "",
            "manual_1",
            "manual_1",
            "No Folder",
        )
        .unwrap();
        // game_folder defaults to "" — must NOT match an empty-string query
        let game = find_by_game_folder(&conn, "").unwrap();
        assert!(game.is_none());
        let _ = id;
    }
}
