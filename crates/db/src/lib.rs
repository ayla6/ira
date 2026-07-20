use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

mod crud;
mod lookup;
mod settings;
mod game_config;
mod sessions;
mod profiles;
mod variants;
mod groups;
mod metadata;
mod migration;
mod discs;
pub use crud::*;
pub use lookup::*;
pub use settings::*;
pub use game_config::*;
pub use sessions::*;
pub use profiles::*;
pub use variants::*;
pub use groups::*;
pub use metadata::*;
pub use discs::*;

pub(crate) const GAME_COLUMNS: &str = "id, kind, trophy_source, steam_id, game_id, platform_id, title, hidden, sgdb_id, logo_position, logo_size, manual_unmatch, sort_title, shadps4_version, last_played, release_date, release_timestamp, metacritic_score, steam_review_score, steam_review_count, ra_core, emulator_override, rom_path, playtime, cached_earned_count, cached_total_count, cached_achievement_mtime";

pub(crate) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<ira_models::GameEntry> {
    Ok(ira_models::GameEntry {
        id: row.get(0)?,
        kind: ira_models::GameKind::from_string(&row.get::<_, String>(1)?),
        trophy_source: ira_models::TrophySource::from_string(&row.get::<_, String>(2)?),
        steam_id: row.get(3)?,
        game_id: row.get(4)?,
        platform_id: row.get(5)?,
        title: row.get(6)?,
        hidden: row.get(7)?,
        sgdb_id: row.get(8)?,
        logo_position: row.get(9)?,
        logo_size: row.get(10)?,
        manual_unmatch: row.get(11)?,
        sort_title: row.get(12)?,
        shadps4_version: row.get(13)?,
        last_played: row.get(14)?,
        release_date: row.get(15)?,
        release_timestamp: row.get(16)?,
        metacritic_score: row.get(17)?,
        steam_review_score: row.get(18)?,
        steam_review_count: row.get(19)?,
        ra_core: row.get(20)?,
        emulator_override: row.get(21)?,
        rom_path: row.get(22)?,
        playtime: row.get(23)?,
        cached_earned_count: row.get(24)?,
        cached_total_count: row.get(25)?,
        cached_achievement_mtime: row.get(26)?,
    })
}

pub type DbConn = Pool<SqliteConnectionManager>;

/// Forces all WAL data to be written to the main database file and truncates
/// the WAL. Call this on app shutdown to ensure .wal/.shm files are cleaned up.
pub fn checkpoint(conn: &DbConn) -> Result<(), String> {
    let c = lock_db(conn)?;
    c.pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn lock_db(conn: &DbConn) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, String> {
    conn.get().map_err(|e| e.to_string())
}

pub fn update_field(conn: &DbConn, id: i64, column: &str, value: &dyn rusqlite::types::ToSql) -> Result<(), String> {
    let c = lock_db(conn)?;
    let sql = format!("UPDATE games SET {} = ?1 WHERE id = ?2", column);
    c.execute(&sql, rusqlite::params![value, id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn init_db(db_path: &str) -> DbConn {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).expect("failed to create database directory");
    }
    let manager = SqliteConnectionManager::file(db_path);
    let pool = Pool::builder()
        .max_size(16)
        .connection_customizer(Box::new(WalCustomizer))
        .build(manager)
        .expect("failed to create connection pool");

    {
        let conn = pool.get().expect("failed to get connection from pool");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS games (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                trophy_source TEXT NOT NULL DEFAULT '',
                steam_id TEXT NOT NULL DEFAULT '',
                game_id TEXT NOT NULL DEFAULT '',
                platform_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                hidden INTEGER NOT NULL DEFAULT 0,
                sgdb_id TEXT,
                logo_position TEXT NOT NULL DEFAULT 'bottom-left',
                logo_size INTEGER NOT NULL DEFAULT 50,
                manual_unmatch INTEGER NOT NULL DEFAULT 0,
                sort_title TEXT NOT NULL DEFAULT '',
                shadps4_version TEXT NOT NULL DEFAULT '',
                last_played INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id ON games(steam_id) WHERE steam_id != '';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps4_serial ON games(kind, platform_id) WHERE kind = 'ps4';
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
            );
            CREATE TABLE IF NOT EXISTS groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );
            CREATE TABLE IF NOT EXISTS game_groups (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                PRIMARY KEY (game_id, group_id)
            );
            CREATE INDEX IF NOT EXISTS idx_game_groups_group ON game_groups(group_id);",
        ).expect("failed to create tables");

        migration::run_schema_migrations(&conn);
    }

    create_variants_table(&pool);
    {
        let c = lock_db(&pool).expect("failed to get connection for default variant table");
        let _ = c.execute_batch(
            "CREATE TABLE IF NOT EXISTS game_default_variant (
                game_id INTEGER PRIMARY KEY,
                variant_id INTEGER
            );"
        );
    }
    create_discs_table(&pool);
    create_default_disc_table(&pool);
    pool
}

#[derive(Debug)]
struct WalCustomizer;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for WalCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }
}
