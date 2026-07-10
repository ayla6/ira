use crate::db::DbConn;
use rusqlite::params;

pub fn get_ignored_lutris_ids(conn: &DbConn) -> std::collections::HashSet<i64> {
    let c = match conn.lock() {
        Ok(c) => c,
        Err(_) => return std::collections::HashSet::new(),
    };
    let mut stmt = match c.prepare("SELECT lutris_db_id FROM games WHERE ignored = 1 AND lutris_db_id IS NOT NULL") {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(),
    };
    stmt.query_map([], |row| row.get::<_, i64>(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn set_lutris_hidden(conn: &DbConn, lutris_id: i64, hidden: bool) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO lutris_meta (lutris_id, hidden) VALUES (?1, ?2)
         ON CONFLICT(lutris_id) DO UPDATE SET hidden = excluded.hidden",
        params![lutris_id, hidden],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_hidden_lutris_ids(conn: &DbConn) -> std::collections::HashSet<i64> {
    let c = match conn.lock() {
        Ok(c) => c,
        Err(_) => return std::collections::HashSet::new(),
    };
    let mut stmt = match c.prepare("SELECT lutris_id FROM lutris_meta WHERE hidden = 1") {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(),
    };
    stmt.query_map([], |row| row.get::<_, i64>(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn set_lutris_db_id(conn: &DbConn, id: i64, lutris_db_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET lutris_db_id = ?1 WHERE id = ?2", params![lutris_db_id, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unmatch_game(conn: &DbConn, lutris_db_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET lutris_db_id = NULL, manual_unmatch = 1 WHERE lutris_db_id = ?1",
        params![lutris_db_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn upsert_matching(conn: &DbConn, lutris_db_id: i64, steam_id: &str, kind: &str, platform_id: &str) -> Result<i64, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;

    let existing_by_steam: Option<i64> = c.query_row(
        "SELECT id FROM games WHERE steam_id = ?1",
        params![steam_id],
        |row| row.get(0),
    ).ok();

    if let Some(id) = existing_by_steam {
        c.execute(
            "UPDATE games SET lutris_db_id = ?1, kind = ?2, platform_id = ?3 WHERE id = ?4",
            params![lutris_db_id, kind, platform_id, id],
        ).map_err(|e| e.to_string())?;
        return Ok(id);
    }

    let existing_by_lutris: Option<i64> = c.query_row(
        "SELECT id FROM games WHERE lutris_db_id = ?1",
        params![lutris_db_id],
        |row| row.get(0),
    ).ok();

    if let Some(id) = existing_by_lutris {
        c.execute(
            "UPDATE games SET steam_id = ?1, kind = ?2, platform_id = ?3 WHERE id = ?4",
            params![steam_id, kind, platform_id, id],
        ).map_err(|e| e.to_string())?;
        return Ok(id);
    }

    c.execute(
        "INSERT INTO games (lutris_db_id, kind, steam_id, platform_id) VALUES (?1, ?2, ?3, ?4)",
        params![lutris_db_id, kind, steam_id, platform_id],
    ).map_err(|e| e.to_string())?;
    let new_id = c.last_insert_rowid();
    if let Ok(h) = c.query_row::<bool, _, _>(
        "SELECT hidden FROM lutris_meta WHERE lutris_id = ?1",
        params![lutris_db_id],
        |row| row.get::<_, i64>(0).map(|v| v != 0),
    ) {
        let _ = c.execute("UPDATE games SET hidden = ?1 WHERE id = ?2", params![h, new_id]);
        let _ = c.execute("DELETE FROM lutris_meta WHERE lutris_id = ?1", params![lutris_db_id]);
    }
    Ok(new_id)
}
