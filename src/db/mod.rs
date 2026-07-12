use rusqlite::Connection;
use std::sync::{Arc, Mutex};

mod crud;
mod lookup;
mod lutris_ops;
mod settings;
mod game_config;
mod sessions;
mod profiles;
mod variants;
pub use crud::*;
pub use lookup::*;
pub use lutris_ops::*;
pub use settings::*;
pub use game_config::*;
pub use sessions::*;
pub use profiles::*;
pub use variants::*;

pub(super) const GAME_COLUMNS: &str = "id, kind, trophy_source, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played";

pub(super) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<crate::models::GameEntry> {
    Ok(crate::models::GameEntry {
        id: row.get(0)?,
        kind: row.get(1)?,
        trophy_source: row.get(2)?,
        steam_id: row.get(3)?,
        platform_id: row.get(4)?,
        title: row.get(5)?,
        hidden: row.get(6)?,
        lutris_db_id: row.get(7)?,
        sgdb_id: row.get(8)?,
        logo_position: row.get(9)?,
        logo_size: row.get(10)?,
        ignored: row.get(11)?,
        manual_unmatch: row.get(12)?,
        sort_title: row.get(13)?,
        shadps4_version: row.get(14)?,
        last_played: row.get(15)?,
    })
}

pub type DbConn = Arc<Mutex<Connection>>;

pub fn init_db(db_path: &str) -> DbConn {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).expect("failed to create database directory");
    }
    let conn = Connection::open(db_path).expect("failed to open database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS games (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            trophy_source TEXT NOT NULL DEFAULT '',
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
            shadps4_version TEXT NOT NULL DEFAULT '',
            last_played INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id ON games(steam_id) WHERE steam_id != '';
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_trophy_platform ON games(trophy_source, platform_id) WHERE trophy_source != '';
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps4_serial ON games(kind, platform_id) WHERE kind = 'ps4';
        CREATE UNIQUE INDEX IF NOT EXISTS idx_games_lutris_db_id ON games(lutris_db_id) WHERE lutris_db_id IS NOT NULL;
        CREATE TABLE IF NOT EXISTS lutris_meta (
            lutris_id INTEGER PRIMARY KEY,
            hidden INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS game_configs (
            game_id INTEGER NOT NULL UNIQUE,
            launch_config TEXT NOT NULL DEFAULT '',
            wine_config TEXT NOT NULL DEFAULT '',
            profile_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS play_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER NOT NULL,
            duration_seconds INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_game_id ON play_sessions(game_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON play_sessions(started_at);
        CREATE TABLE IF NOT EXISTS wine_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            wine_version TEXT NOT NULL DEFAULT 'system',
            custom_wine_path TEXT NOT NULL DEFAULT '',
            prefix TEXT NOT NULL DEFAULT '',
            arch TEXT NOT NULL DEFAULT 'auto'
        );",
    ).expect("failed to create tables");

    let conn = Arc::new(Mutex::new(conn));
    create_variants_table(&conn);
    conn
}
