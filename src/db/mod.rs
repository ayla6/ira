use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

mod crud;
mod lookup;
mod lutris_ops;
mod settings;
pub use crud::*;
pub use lookup::*;
pub use lutris_ops::*;
pub use settings::*;

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
