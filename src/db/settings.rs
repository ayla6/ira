use crate::db::DbConn;
use rusqlite::params;

pub fn set_game_hidden(conn: &DbConn, id: i64, hidden: bool) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET hidden = ?1 WHERE id = ?2", params![hidden, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_logo_settings(conn: &DbConn, id: i64, position: &str, size: i32) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET logo_position = ?1, logo_size = ?2 WHERE id = ?3",
        params![position, size, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_sgdb_id(conn: &DbConn, id: i64, sgdb_id: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET sgdb_id = ?1 WHERE id = ?2",
        params![if sgdb_id.is_empty() { None } else { Some(sgdb_id) }, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_shadps4_version(conn: &DbConn, id: i64, version: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET shadps4_version = ?1 WHERE id = ?2",
        params![if version.is_empty() { "" } else { version }, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_last_played(conn: &DbConn, id: i64, timestamp: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET last_played = ?1 WHERE id = ?2",
        params![timestamp, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
