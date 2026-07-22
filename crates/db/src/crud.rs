use crate::DbConn;
use ira_models::{GameEntry, GameKind, TrophySource};
use rusqlite::params;

pub fn add_game(conn: &DbConn, kind: GameKind, trophy_source: TrophySource, steam_id: &str, game_id: &str, platform_id: &str, title: &str) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    let kind = kind.as_str();
    let trophy_source = trophy_source.as_str();
    if !steam_id.is_empty() {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(steam_id) WHERE steam_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, game_id, platform_id, title],
        ).map_err(|e| e.to_string())?;
    } else {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(game_id, platform_id) WHERE game_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, game_id, platform_id, title],
        ).map_err(|e| e.to_string())?;
    }
    Ok(c.last_insert_rowid())
}

pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("UPDATE games SET title = ?1 WHERE id = ?2", params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_game_ids(conn: &DbConn, id: i64, steam_id: &str, game_id: &str, trophy_source: TrophySource, platform_id: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET steam_id = ?1, game_id = ?2, trophy_source = ?3, platform_id = ?4 WHERE id = ?5",
        params![steam_id, game_id, trophy_source.as_str(), platform_id, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_sort_title(conn: &DbConn, id: i64, sort_title: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("UPDATE games SET sort_title = ?1 WHERE id = ?2", params![sort_title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_achievement_counts(conn: &DbConn, id: i64, earned: i64, total: i64, mtime: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET cached_earned_count = ?1, cached_total_count = ?2, cached_achievement_mtime = ?3 WHERE id = ?4",
        params![earned, total, mtime, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_manual_unmatch(conn: &DbConn, id: i64, value: bool) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("UPDATE games SET manual_unmatch = ?1 WHERE id = ?2", params![value, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games ORDER BY CASE WHEN sort_title != '' THEN sort_title ELSE title END", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let entries = stmt.query_map([], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;

    entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM games WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::find_by_db_id;
    use super::super::init_db;
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_add_game_inserts_new_game() {
        let (conn, _tmp) = setup_db();
        let id = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "Test Game").unwrap();
        assert!(id > 0);
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.title, "Test Game");
        assert_eq!(game.steam_id, "12345");
        assert_eq!(game.kind, GameKind::Steam);
    }

    #[test]
    fn test_add_game_conflict_updates_existing() {
        let (conn, _tmp) = setup_db();
        let id1 = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "").unwrap();
        let id2 = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "Updated Title").unwrap();
        assert_eq!(id1, id2);
        let game = find_by_db_id(&conn, id1).unwrap().unwrap();
        assert_eq!(game.title, "Updated Title");
    }

    #[test]
    fn test_update_game_title() {
        let (conn, _tmp) = setup_db();
        let id = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "Test Game").unwrap();
        update_game_title(&conn, id, "New Title").unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.title, "New Title");
    }

    #[test]
    fn test_update_game_ids() {
        let (conn, _tmp) = setup_db();
        let id = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "Test Game").unwrap();
        update_game_ids(&conn, id, "67890", "game123", TrophySource::Ra, "ps4").unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.steam_id, "67890");
        assert_eq!(game.game_id, "game123");
        assert_eq!(game.trophy_source, TrophySource::Ra);
        assert_eq!(game.platform_id, "ps4");
    }

    #[test]
    fn test_delete_game_removes_entry() {
        let (conn, _tmp) = setup_db();
        let id = add_game(&conn, GameKind::Steam, TrophySource::Gse, "12345", "", "", "Test Game").unwrap();
        remove_game(&conn, id).unwrap();
        let game = find_by_db_id(&conn, id).unwrap();
        assert!(game.is_none());
    }
}
