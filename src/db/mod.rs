use rusqlite::Connection;
use std::sync::{Arc, Mutex};

mod crud;
mod lookup;
mod lutris_ops;
mod settings;
pub use crud::*;
pub use lookup::*;
pub use lutris_ops::*;
pub use settings::*;

pub(super) const GAME_COLUMNS: &str = "id, kind, steam_id, platform_id, title, hidden, lutris_db_id, sgdb_id, logo_position, logo_size, ignored, manual_unmatch, sort_title, shadps4_version, last_played";

pub(super) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<crate::models::GameEntry> {
    Ok(crate::models::GameEntry {
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
            shadps4_version TEXT NOT NULL DEFAULT '',
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

    Arc::new(Mutex::new(conn))
}
