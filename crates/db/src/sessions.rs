use crate::DbConn;
use ira_models::PlaySession;
use rusqlite::params;

pub fn record_session(conn: &DbConn, game_id: i64, variant_id: Option<i64>, started_at: i64, ended_at: i64) -> Result<i64, String> {
    let duration = ended_at - started_at;
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO play_sessions (game_id, variant_id, started_at, ended_at, duration_seconds) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![game_id, variant_id, started_at, ended_at, duration],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn get_sessions_for_game(conn: &DbConn, game_id: i64, variant_id: Option<i64>) -> Result<Vec<PlaySession>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT id, game_id, variant_id, started_at, ended_at, duration_seconds FROM play_sessions WHERE game_id = ?1 AND (variant_id IS ?2) ORDER BY started_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![game_id, variant_id], |row| {
            Ok(PlaySession {
                id: row.get(0)?,
                game_id: row.get(1)?,
                variant_id: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                duration_seconds: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

fn day_start_end(date: &chrono::NaiveDate) -> (i64, i64) {
    let start = date.and_hms_opt(0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(1);
    (start.and_utc().timestamp(), end.and_utc().timestamp())
}

pub fn get_sessions_for_date(conn: &DbConn, date: chrono::NaiveDate) -> Result<Vec<PlaySession>, String> {
    let (day_start, day_end) = day_start_end(&date);
    get_sessions_range(conn, day_start, day_end)
}

pub fn get_sessions_range(conn: &DbConn, from: i64, to: i64) -> Result<Vec<PlaySession>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT id, game_id, variant_id, started_at, ended_at, duration_seconds FROM play_sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![from, to], |row| {
            Ok(PlaySession {
                id: row.get(0)?,
                game_id: row.get(1)?,
                variant_id: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                duration_seconds: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_total_playtime_for_game(conn: &DbConn, game_id: i64, variant_id: Option<i64>) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT COALESCE(SUM(duration_seconds), 0) FROM play_sessions WHERE game_id = ?1 AND (variant_id IS ?2)")
        .map_err(|e| e.to_string())?;
    let result: i64 = stmt
        .query_row(params![game_id, variant_id], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(result)
}

pub fn get_playtime_by_day(conn: &DbConn, from: i64, to: i64) -> Result<Vec<(chrono::NaiveDate, i64)>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT date(started_at, 'unixepoch') AS day, SUM(duration_seconds) FROM play_sessions WHERE started_at >= ?1 AND started_at < ?2 GROUP BY day ORDER BY day DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![from, to], |row| {
            let day_str: String = row.get(0)?;
            let total: i64 = row.get(1)?;
            let day = chrono::NaiveDate::parse_from_str(&day_str, "%Y-%m-%d")
                .unwrap_or_else(|_| chrono::NaiveDate::default());
            Ok((day, total))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn delete_sessions_for_game(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM play_sessions WHERE game_id = ?1", params![game_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_get_sessions_for_date() {
        let (conn, _tmp) = setup_db();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let start = day.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let mid = day.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let end = day.and_hms_opt(23, 59, 0).unwrap().and_utc().timestamp();

        record_session(&conn, 1, None, start, start + 3600).unwrap();
        record_session(&conn, 1, None, mid, mid + 7200).unwrap();
        record_session(&conn, 1, None, end, end + 1800).unwrap();

        let sessions = get_sessions_for_date(&conn, day).unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[test]
    fn test_get_total_playtime_for_game() {
        let (conn, _tmp) = setup_db();
        record_session(&conn, 1, None, 1000, 13600).unwrap(); // 12600 sec = 3.5h
        record_session(&conn, 1, None, 20000, 22800).unwrap(); // 2800 sec
        record_session(&conn, 2, None, 30000, 30600).unwrap(); // 600 sec

        let total = get_total_playtime_for_game(&conn, 1, None).unwrap();
        assert_eq!(total, 12600 + 2800);

        let total2 = get_total_playtime_for_game(&conn, 2, None).unwrap();
        assert_eq!(total2, 600);
    }

    #[test]
    fn test_get_playtime_by_day() {
        let (conn, _tmp) = setup_db();
        let day1 = chrono::NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let day2 = chrono::NaiveDate::from_ymd_opt(2026, 7, 11).unwrap();

        let d1_start = day1.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let d2_start = day2.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

        record_session(&conn, 1, None, d1_start, d1_start + 3600).unwrap();
        record_session(&conn, 1, None, d1_start + 7200, d1_start + 10800).unwrap();
        record_session(&conn, 2, None, d2_start, d2_start + 1800).unwrap();

        let by_day = get_playtime_by_day(&conn, d1_start, d2_start + 86400).unwrap();
        assert_eq!(by_day.len(), 2);

        let d1_total = by_day.iter().find(|(d, _)| *d == day1).map(|(_, t)| *t).unwrap();
        assert_eq!(d1_total, 3600 + 3600);

        let d2_total = by_day.iter().find(|(d, _)| *d == day2).map(|(_, t)| *t).unwrap();
        assert_eq!(d2_total, 1800);
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
    fn test_get_total_playtime_no_sessions() {
        let (conn, _tmp) = setup_db();
        let total = get_total_playtime_for_game(&conn, 1, None).unwrap();
        assert_eq!(total, 0);
    }
}
