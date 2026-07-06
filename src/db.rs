use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct GameEntry {
    pub id: i64,
    pub kind: String,
    pub steam_id: String,
    pub platform_id: String,
    pub title: String,
    #[allow(dead_code)]
    pub lutris_id: Option<String>,
}

pub type DbConn = Arc<Mutex<Connection>>;

pub fn init_db(db_path: &str) -> DbConn {
    let conn = Connection::open(db_path).expect("failed to open database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS games (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            steam_id TEXT NOT NULL,
            platform_id TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            lutris_id TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id ON games(steam_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_kind_platform ON games(kind, platform_id);",
    ).expect("failed to create tables");
    Arc::new(Mutex::new(conn))
}

pub fn add_game(conn: &DbConn, kind: &str, steam_id: &str, platform_id: &str, title: &str) -> Result<i64, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO games (kind, steam_id, platform_id, title) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(steam_id) DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
        params![kind, steam_id, platform_id, title],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn update_game(conn: &DbConn, id: i64, title: &str, lutris_id: Option<&str>) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET title = ?1, lutris_id = ?2 WHERE id = ?3",
        params![title, lutris_id, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Update only the title of a game (leaves lutris_id untouched).
pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET title = ?1 WHERE id = ?2", params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, lutris_id FROM games ORDER BY title")
        .map_err(|e| e.to_string())?;
    let entries = stmt.query_map([], |row| {
        Ok(GameEntry {
            id: row.get(0)?,
            kind: row.get(1)?,
            steam_id: row.get(2)?,
            platform_id: row.get(3)?,
            title: row.get(4)?,
            lutris_id: row.get(5)?,
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
    c.execute("DELETE FROM games WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, lutris_id FROM games WHERE steam_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![steam_id], |row| {
        Ok(GameEntry {
            id: row.get(0)?,
            kind: row.get(1)?,
            steam_id: row.get(2)?,
            platform_id: row.get(3)?,
            title: row.get(4)?,
            lutris_id: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}
