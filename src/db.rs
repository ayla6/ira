use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct GameEntry {
    pub id: i64,
    pub kind: String,
    pub steam_id: String,
    pub platform_id: String,
    pub title: String,
    pub hidden: bool,
    /// Link to the Lutris game (its internal numeric id). None = not linked.
    pub lutris_db_id: Option<i64>,
    /// SteamGridDB id for games with no achievement source but need images.
    pub sgdb_id: Option<String>,
    /// Per-game logo overlay position (e.g. "bottom-left").
    pub logo_position: String,
    /// Per-game logo height constraint in pixels.
    pub logo_size: i32,
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
    // Migrations for databases created before these columns existed.
    let _ = conn.execute("ALTER TABLE games ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE games ADD COLUMN lutris_db_id INTEGER", []);
    let _ = conn.execute("ALTER TABLE games ADD COLUMN sgdb_id TEXT", []);
    let _ = conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_games_lutris_db_id ON games(lutris_db_id) WHERE lutris_db_id IS NOT NULL",
        [],
    );
    let _ = conn.execute("ALTER TABLE games ADD COLUMN logo_position TEXT NOT NULL DEFAULT 'bottom-left'", []);
    let _ = conn.execute("ALTER TABLE games ADD COLUMN logo_size INTEGER NOT NULL DEFAULT 25", []);
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

/// Update only the title of a game.
pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET title = ?1 WHERE id = ?2", params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size FROM games ORDER BY title")
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
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for entry in entries {
        result.push(entry.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn set_game_hidden(conn: &DbConn, id: i64, hidden: bool) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET hidden = ?1 WHERE id = ?2", params![hidden, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM games WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size FROM games WHERE steam_id = ?1")
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
        })
    }).map_err(|e| e.to_string())?;

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

/// Find a GOG game (kind="gog") by its product id.
pub fn find_gog_by_product_id(conn: &DbConn, product_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size FROM games WHERE kind = 'gog' AND platform_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut entries = stmt.query_map(params![product_id], |row| {
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
        })
    }).map_err(|e| e.to_string())?;

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}

/// Link one of our games to a Lutris game by its internal id.
pub fn set_lutris_db_id(conn: &DbConn, id: i64, lutris_db_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET lutris_db_id = ?1 WHERE id = ?2", params![lutris_db_id, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Create or update the matching from a Lutris game to an achievement source
/// (Steam app id + kind). Used when the user matches an unmatched game.
pub fn upsert_matching(conn: &DbConn, lutris_db_id: i64, steam_id: &str, kind: &str, platform_id: &str) -> Result<i64, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO games (lutris_db_id, kind, steam_id, platform_id) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(lutris_db_id) DO UPDATE SET steam_id = excluded.steam_id, kind = excluded.kind, platform_id = excluded.platform_id",
        params![lutris_db_id, kind, steam_id, platform_id],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

/// Look up a game by its Lutris id.
pub fn find_by_lutris_id(conn: &DbConn, lutris_db_id: i64) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size FROM games WHERE lutris_db_id = ?1")
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
        })
    }).map_err(|e| e.to_string())?;

    if let Some(entry) = entries.next() {
        Ok(Some(entry.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
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
