use crate::{err, DbConn};
use ira_models::PlaySession;
use rusqlite::params;

/// Column list of every play_sessions SELECT; the order must match
/// `play_session_from_row`.
const SESSION_COLUMNS: &str = "id, game_id, variant_id, started_at, ended_at, duration_seconds";
/// Filter shared by the per-game session and playtime queries; `IS` matches
/// both NULL and equal variant ids.
const GAME_VARIANT_WHERE: &str = "game_id = ?1 AND (variant_id IS ?2)";

pub fn record_session(
    conn: &DbConn,
    game_id: i64,
    variant_id: Option<i64>,
    started_at: i64,
    ended_at: i64,
) -> Result<i64, String> {
    let duration = ended_at - started_at;
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO play_sessions (game_id, variant_id, started_at, ended_at, duration_seconds) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![game_id, variant_id, started_at, ended_at, duration],
    )
    .map_err(err)?;
    Ok(c.last_insert_rowid())
}

fn play_session_from_row(row: &rusqlite::Row) -> rusqlite::Result<PlaySession> {
    Ok(PlaySession {
        id: row.get(0)?,
        game_id: row.get(1)?,
        variant_id: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        duration_seconds: row.get(5)?,
    })
}

pub fn get_sessions_for_game(
    conn: &DbConn,
    game_id: i64,
    variant_id: Option<i64>,
) -> Result<Vec<PlaySession>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM play_sessions WHERE {GAME_VARIANT_WHERE} ORDER BY started_at DESC"
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map(params![game_id, variant_id], play_session_from_row)
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

pub fn get_sessions_range(conn: &DbConn, from: i64, to: i64) -> Result<Vec<PlaySession>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM play_sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at DESC"
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map(params![from, to], play_session_from_row)
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

pub fn delete_sessions_for_game(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM play_sessions WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    Ok(())
}

/// Delete a single session, returning the removed row so the caller can
/// subtract its duration from the game's playtime.
pub fn delete_session(conn: &DbConn, session_id: i64) -> Result<Option<PlaySession>, String> {
    let session = crate::query_optional(
        conn,
        &format!("SELECT {SESSION_COLUMNS} FROM play_sessions WHERE id = ?1"),
        params![session_id],
        play_session_from_row,
    )?;
    if session.is_some() {
        let c = crate::lock_db(conn)?;
        c.execute(
            "DELETE FROM play_sessions WHERE id = ?1",
            params![session_id],
        )
        .map_err(err)?;
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::super::init_db;
    use super::*;
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_record_and_get_sessions() {
        let (conn, _tmp) = setup_db();
        let id = record_session(&conn, 1, None, 1000, 1050).unwrap();
        assert!(id > 0);

        let sessions = get_sessions_for_game(&conn, 1, None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].game_id, 1);
        assert_eq!(sessions[0].started_at, 1000);
        assert_eq!(sessions[0].ended_at, 1050);
        assert_eq!(sessions[0].duration_seconds, 50);
    }

    #[test]
    fn test_multiple_sessions() {
        let (conn, _tmp) = setup_db();
        record_session(&conn, 1, None, 1000, 1100).unwrap();
        record_session(&conn, 1, None, 2000, 2100).unwrap();
        record_session(&conn, 2, None, 3000, 3050).unwrap();

        let game1 = get_sessions_for_game(&conn, 1, None).unwrap();
        assert_eq!(game1.len(), 2);

        let game2 = get_sessions_for_game(&conn, 2, None).unwrap();
        assert_eq!(game2.len(), 1);
    }

    #[test]
    fn test_delete_sessions_for_game() {
        let (conn, _tmp) = setup_db();
        record_session(&conn, 1, None, 1000, 1100).unwrap();
        record_session(&conn, 1, None, 2000, 2100).unwrap();
        record_session(&conn, 2, None, 3000, 3050).unwrap();

        delete_sessions_for_game(&conn, 1).unwrap();
        assert_eq!(get_sessions_for_game(&conn, 1, None).unwrap().len(), 0);
        assert_eq!(get_sessions_for_game(&conn, 2, None).unwrap().len(), 1);
    }

    #[test]
    fn test_no_sessions_returns_empty() {
        let (conn, _tmp) = setup_db();
        let sessions = get_sessions_for_game(&conn, 999, None).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_delete_session_single() {
        let (conn, _tmp) = setup_db();
        let id = record_session(&conn, 1, None, 1000, 1050).unwrap();

        let removed = delete_session(&conn, id).unwrap().unwrap();
        assert_eq!(removed.id, id);
        assert_eq!(removed.duration_seconds, 50);
        assert!(get_sessions_for_game(&conn, 1, None).unwrap().is_empty());
    }

    #[test]
    fn test_delete_session_missing_returns_none() {
        let (conn, _tmp) = setup_db();
        let removed = delete_session(&conn, 9999).unwrap();
        assert!(removed.is_none());
    }
}
