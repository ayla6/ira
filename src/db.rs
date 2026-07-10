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
    /// Sort title (empty = use title for sorting).
    pub sort_title: String,
    /// Per-game shadPS4 version path (empty = use global default).
    pub shadps4_version: Option<String>,
    /// Unix timestamp of last time the game was launched via our play button.
    pub last_played: i64,
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
            hidden INTEGER NOT NULL DEFAULT 0,
            lutris_db_id INTEGER,
            sgdb_id TEXT,
            logo_position TEXT NOT NULL DEFAULT 'bottom-left',
            logo_size INTEGER NOT NULL DEFAULT 50,
            ignored INTEGER NOT NULL DEFAULT 0,
            manual_unmatch INTEGER NOT NULL DEFAULT 0,
            sort_title TEXT NOT NULL DEFAULT '',
            last_played INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id ON games(steam_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_kind_platform ON games(kind, platform_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_lutris_db_id ON games(lutris_db_id) WHERE lutris_db_id IS NOT NULL;
        CREATE TABLE IF NOT EXISTS lutris_meta (
            lutris_id INTEGER PRIMARY KEY,
            hidden INTEGER NOT NULL DEFAULT 0
        );",
    ).expect("failed to create tables");

    // Migrations for databases created before columns existed
    let columns = [
        ("hidden", "INTEGER NOT NULL DEFAULT 0"),
        ("lutris_db_id", "INTEGER"),
        ("sgdb_id", "TEXT"),
        ("logo_position", "TEXT NOT NULL DEFAULT 'bottom-left'"),
        ("logo_size", "INTEGER NOT NULL DEFAULT 50"),
        ("ignored", "INTEGER NOT NULL DEFAULT 0"),
        ("manual_unmatch", "INTEGER NOT NULL DEFAULT 0"),
        ("sort_title", "TEXT NOT NULL DEFAULT ''"),
        ("shadps4_version", "TEXT NOT NULL DEFAULT ''"),
        ("last_played", "INTEGER NOT NULL DEFAULT 0"),
    ];
    for (col, def) in &columns {
        let _ = conn.execute(&format!("ALTER TABLE games ADD COLUMN {} {}", col, def), []);
    }
    let _ = conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_games_lutris_db_id ON games(lutris_db_id) WHERE lutris_db_id IS NOT NULL",
        [],
    );
    conn.execute_batch("CREATE TABLE IF NOT EXISTS lutris_meta (lutris_id INTEGER PRIMARY KEY, hidden INTEGER NOT NULL DEFAULT 0);")
        .expect("failed to create lutris_meta table");
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

/// Update sort title for a game.
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

/// Set hidden state for a Lutris game that has no DB row (unmatched).
pub fn set_lutris_hidden(conn: &DbConn, lutris_id: i64, hidden: bool) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO lutris_meta (lutris_id, hidden) VALUES (?1, ?2)
         ON CONFLICT(lutris_id) DO UPDATE SET hidden = excluded.hidden",
        params![lutris_id, hidden],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Get the set of Lutris IDs that are hidden but have no DB row.
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

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("UPDATE games SET ignored = 1 WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    Ok(())
}

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

/// Find a GOG game (kind="gog") by its product id.
pub fn find_gog_by_product_id(conn: &DbConn, product_id: &str) -> Result<Option<GameEntry>, String> {
    find_by_kind_platform(conn, "gog", product_id)
}

/// Find a game by (kind, platform_id).
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
    let new_id = c.last_insert_rowid();
    // Sync hidden state from lutris_meta (if game was hidden while unmatched)
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

/// Look up a game by its Lutris id.
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

pub fn set_logo_settings(conn: &DbConn, id: i64, position: &str, size: i32) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET logo_position = ?1, logo_size = ?2 WHERE id = ?3",
        params![position, size, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Set the SteamGridDB game ID for a game (enables SGDB image downloads).
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
