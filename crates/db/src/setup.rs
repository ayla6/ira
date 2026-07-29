use crate::DbConn;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::collections::HashMap;

pub fn checkpoint(conn: &DbConn) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_field(conn: &DbConn, id: i64, column: &str, value: &dyn rusqlite::types::ToSql) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
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
                last_played INTEGER NOT NULL DEFAULT 0,
                release_date TEXT NOT NULL DEFAULT '',
                release_timestamp INTEGER NOT NULL DEFAULT 0,
                metacritic_score INTEGER NOT NULL DEFAULT -1,
                steam_review_score INTEGER NOT NULL DEFAULT -1,
                steam_review_count INTEGER NOT NULL DEFAULT 0,
                ra_core TEXT NOT NULL DEFAULT '',
                emulator_override TEXT NOT NULL DEFAULT '',
                rom_path TEXT NOT NULL DEFAULT '',
                playtime REAL NOT NULL DEFAULT 0.0,
                cached_earned_count INTEGER NOT NULL DEFAULT 0,
                cached_total_count INTEGER NOT NULL DEFAULT 0,
                cached_achievement_mtime INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id ON games(steam_id) WHERE steam_id != '';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_game_id_platform ON games(game_id, platform_id) WHERE game_id != '';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps4_serial ON games(kind, platform_id) WHERE kind = 'ps4';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps3_serial ON games(kind, platform_id) WHERE kind = 'ps3';
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
                duration_seconds INTEGER NOT NULL,
                variant_id INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_game_id ON play_sessions(game_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON play_sessions(started_at);
            CREATE TABLE IF NOT EXISTS wine_profiles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                wine_version TEXT NOT NULL DEFAULT 'system',
                custom_wine_path TEXT NOT NULL DEFAULT '',
                prefix TEXT NOT NULL DEFAULT '',
                arch TEXT NOT NULL DEFAULT 'auto',
                umu_enabled INTEGER NOT NULL DEFAULT 1
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

        // PRE-RELEASE: schema migrations merged into CREATE TABLE above
    }

    crate::create_variants_table(&pool);
    {
        let c = crate::lock_db(&pool).expect("failed to get connection for default variant table");
        if let Err(e) = c.execute_batch(
            "CREATE TABLE IF NOT EXISTS game_default_variant (
                game_id INTEGER PRIMARY KEY,
                variant_id INTEGER
            );"
        ) {
            eprintln!("Failed to create game_default_variant table: {e}");
        }
    }
    crate::create_discs_table(&pool);
    crate::create_default_disc_table(&pool);
    pool
}

// PRE-RELEASE: remove after v0.X
pub fn migrate_rom_paths_to_relative(
    conn: &DbConn,
    console_folders: &HashMap<String, String>,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;

    let mut stmt = c.prepare(
        "SELECT id, platform_id, rom_path FROM games WHERE kind = 'retro' AND rom_path != ''"
    ).map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();
    drop(stmt);

    for (id, platform_id, rom_path) in &rows {
        if let Some(folder) = console_folders.get(platform_id) {
            if !folder.is_empty() && rom_path.starts_with(folder) {
                let relative = rom_path[folder.len()..].trim_start_matches('/');
                if relative != rom_path.as_str() {
                    c.execute(
                        "UPDATE games SET rom_path = ?1 WHERE id = ?2",
                        rusqlite::params![relative, id],
                    ).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    let mut stmt = c.prepare(
        "SELECT gd.id, g.platform_id, gd.rom_path
         FROM game_discs gd
         JOIN games g ON gd.game_id = g.id
         WHERE g.kind = 'retro' AND gd.rom_path != ''"
    ).map_err(|e| e.to_string())?;
    let disc_rows: Vec<(i64, String, String)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();
    drop(stmt);

    for (id, platform_id, rom_path) in &disc_rows {
        if let Some(folder) = console_folders.get(platform_id) {
            if !folder.is_empty() && rom_path.starts_with(folder) {
                let relative = rom_path[folder.len()..].trim_start_matches('/');
                if relative != rom_path.as_str() {
                    c.execute(
                        "UPDATE game_discs SET rom_path = ?1 WHERE id = ?2",
                        rusqlite::params![relative, id],
                    ).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    Ok(())
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
