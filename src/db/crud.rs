use crate::db::DbConn;
use crate::models::GameEntry;
use rusqlite::params;

pub fn add_game(conn: &DbConn, kind: &str, steam_id: &str, platform_id: &str, title: &str) -> Result<i64, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO games (kind, steam_id, platform_id, title) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(steam_id) DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
        params![kind, steam_id, platform_id, title],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET title = ?1 WHERE id = ?2", params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_sort_title(conn: &DbConn, id: i64, sort_title: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET sort_title = ?1 WHERE id = ?2", params![sort_title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played FROM games WHERE ignored = 0 ORDER BY CASE WHEN sort_title != '' THEN sort_title ELSE title END")
        .map_err(|e| e.to_string())?;
    let entries = stmt.query_map([], |row| {
        Ok(GameEntry {
            id: row.get(0)?,
            kind: row.get(1)?,
            steam_id: row.get(2)?,
            platform_id: row.get(3)?,
            title: row.get(4)?,
            hidden: row.get(5)?,
            lutris_db_id: row.get(6)?,
            sgdb_id: row.get(7)?,
            logo_position: row.get(8)?,
            logo_size: row.get(9)?,
            ignored: row.get(10)?,
            manual_unmatch: row.get(11)?,
            sort_title: row.get(12)?,
            shadps4_version: row.get(13)?,
            last_played: row.get(14)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for entry in entries {
        result.push(entry.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET ignored = 1 WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}
