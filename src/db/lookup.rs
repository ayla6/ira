use crate::db::DbConn;
use crate::models::GameEntry;
use rusqlite::params;

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played FROM games WHERE steam_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![steam_id], |row| {
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

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_gog_by_product_id(conn: &DbConn, product_id: &str) -> Result<Option<GameEntry>, String> {
    find_by_kind_platform(conn, "gog", product_id)
}

pub fn find_by_kind_platform(conn: &DbConn, kind: &str, platform_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played FROM games WHERE kind = ?1 AND platform_id = ?2")
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![kind, platform_id], |row| {
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

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_lutris_id(conn: &DbConn, lutris_db_id: i64) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played FROM games WHERE lutris_db_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![lutris_db_id], |row| {
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

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}
