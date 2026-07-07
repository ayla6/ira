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
    /// Set when user removes a game — prevents re-adding from Lutris.
    pub ignored: Option<i64>,
    /// Set when user manually unmatches — prevents auto-rematching.
    pub manual_unmatch: Option<i64>,
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
    let _ = conn.execute("ALTER TABLE games ADD COLUMN logo_size INTEGER NOT NULL DEFAULT 50", []);
    let _ = conn.execute("ALTER TABLE games ADD COLUMN ignored INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE games ADD COLUMN manual_unmatch INTEGER NOT NULL DEFAULT 0", []);
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
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch FROM games WHERE ignored = 0 ORDER BY title")
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
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for entry in entries {
        result.push(entry.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Get the set of Lutris IDs that have been ignored (removed by user).
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

pub fn set_game_hidden(conn: &DbConn, id: i64, hidden: bool) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET hidden = ?1 WHERE id = ?2", params![hidden, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET ignored = 1 WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn find_by_steam_id(conn: &DbConn, steam_id: &str) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch FROM games WHERE steam_id = ?1")
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
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch FROM games WHERE kind = 'gog' AND platform_id = ?1")
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
            ignored: row.get(10)?,
            manual_unmatch: row.get(11)?,
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

/// Remove the Lutris link from a game, making it unmatched.
pub fn unmatch_game(conn: &DbConn, lutris_db_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET lutris_db_id = NULL, manual_unmatch = 1 WHERE lutris_db_id = ?1",
        params![lutris_db_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Create or update the matching from a Lutris game to an achievement source
/// (Steam app id + kind). Used when the user matches an unmatched game.
pub fn upsert_matching(conn: &DbConn, lutris_db_id: i64, steam_id: &str, kind: &str, platform_id: &str) -> Result<i64, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;

    // Case 1: steam_id already exists (from save-dir scan) → set lutris_db_id on it
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

    // Case 2: lutris_db_id already exists (previously matched to a different steam_id) → update it
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

    // Case 3: neither exists → insert new row
    c.execute(
        "INSERT INTO games (lutris_db_id, kind, steam_id, platform_id) VALUES (?1, ?2, ?3, ?4)",
        params![lutris_db_id, kind, steam_id, platform_id],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

/// Look up a game by its Lutris id.
pub fn find_by_lutris_id(conn: &DbConn, lutris_db_id: i64) -> Result<Option<GameEntry>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare("SELECT id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch FROM games WHERE lutris_db_id = ?1")
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
