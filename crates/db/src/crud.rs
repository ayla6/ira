use crate::DbConn;
use ira_models::GameEntry;
use rusqlite::params;

pub fn add_game(conn: &DbConn, kind: &str, trophy_source: &str, steam_id: &str, game_id: &str, platform_id: &str, title: &str) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    if !steam_id.is_empty() {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(steam_id) WHERE steam_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, game_id, platform_id, title],
        ).map_err(|e| e.to_string())?;
    } else {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(game_id) WHERE game_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
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

pub fn update_game_ids(conn: &DbConn, id: i64, steam_id: &str, game_id: &str, trophy_source: &str, platform_id: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET steam_id = ?1, game_id = ?2, trophy_source = ?3, platform_id = ?4 WHERE id = ?5",
        params![steam_id, game_id, trophy_source, platform_id, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_sort_title(conn: &DbConn, id: i64, sort_title: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("UPDATE games SET sort_title = ?1 WHERE id = ?2", params![sort_title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games WHERE ignored = 0 ORDER BY CASE WHEN sort_title != '' THEN sort_title ELSE title END", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let entries = stmt.query_map([], |row| {
        crate::game_entry_from_row(row)
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for entry in entries {
        result.push(entry.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("UPDATE games SET ignored = 1 WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}
