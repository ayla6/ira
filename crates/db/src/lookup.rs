use crate::{DbConn, lock_db};
use ira_models::GameEntry;
use rusqlite::params;

fn find_game_by(conn: &DbConn, where_clause: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Option<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE {}", crate::GAME_COLUMNS, where_clause))
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params, crate::game_entry_from_row)
        .map_err(|e| e.to_string())?;
    match entries.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

fn find_all_games_by(conn: &DbConn, where_clause: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<GameEntry>, String> {
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE {}", crate::GAME_COLUMNS, where_clause))
        .map_err(|e| e.to_string())?;
    let entries = stmt.query_map(params, crate::game_entry_from_row)
        .map_err(|e| e.to_string())?;
    entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "steam_id = ?1", params![steam_id])
}

pub fn find_by_game_id(conn: &DbConn, game_id: &str, platform_id: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "game_id = ?1 AND platform_id = ?2", params![game_id, platform_id])
}

pub fn find_by_db_id(conn: &DbConn, db_id: i64) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "id = ?1", params![db_id])
}

pub fn find_by_trophy_platform(conn: &DbConn, trophy_source: ira_models::TrophySource, platform_id: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "trophy_source = ?1 AND platform_id = ?2", params![trophy_source.as_str(), platform_id])
}

pub fn find_by_kind_platform(conn: &DbConn, kind: ira_models::GameKind, platform_id: &str) -> Result<Option<GameEntry>, String> {
    find_game_by(conn, "kind = ?1 AND platform_id = ?2", params![kind.as_str(), platform_id])
}

pub fn find_all_retro_by_platform(conn: &DbConn, platform_id: &str) -> Result<Vec<GameEntry>, String> {
    find_all_games_by(conn, "kind = 'retro' AND platform_id = ?1", params![platform_id])
}

