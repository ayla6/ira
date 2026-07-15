use crate::{DbConn, lock_db};
use ira_models::GameEntry;
use rusqlite::params;

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE steam_id = ?1", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![steam_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_game_id(conn: &DbConn, game_id: &str) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE game_id = ?1", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![game_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_db_id(conn: &DbConn, db_id: i64) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE id = ?1", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![db_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_gog_by_product_id(conn: &DbConn, product_id: &str) -> Result<Option<GameEntry>, String> {
    find_by_trophy_platform(conn, ira_models::NGE, product_id)
}

pub fn find_by_trophy_platform(conn: &DbConn, trophy_source: &str, platform_id: &str) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE trophy_source = ?1 AND platform_id = ?2", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![trophy_source, platform_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_kind_platform(conn: &DbConn, kind: &str, platform_id: &str) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE kind = ?1 AND platform_id = ?2", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![kind, platform_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_rom_path(conn: &DbConn, rom_path: &str) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE rom_path = ?1", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![rom_path], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

pub fn find_by_lutris_id(conn: &DbConn, lutris_db_id: i64) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE lutris_db_id = ?1", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![lutris_db_id], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;
    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}
