use crate::{err, DbConn};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

pub fn checkpoint(conn: &DbConn) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .map_err(err)?;
    Ok(())
}

pub fn update_field(
    conn: &DbConn,
    id: i64,
    column: &str,
    value: &dyn rusqlite::types::ToSql,
) -> Result<(), String> {
    // The column name cannot be a bound parameter, so anything outside
    // this set is refused instead of formatted into the SQL.
    const UPDATABLE_COLUMNS: &[&str] = &[
        "hidden",
        "vanished",
        "playtime",
        "last_played",
        "shadps4_version",
        "ra_core",
        "emulator_override",
        "rom_path",
        "screenscraper_id",
        "title_trusted",
        "steam_id",
    ];
    if !UPDATABLE_COLUMNS.contains(&column) {
        return Err(format!("update_field: unknown column {column}"));
    }
    let c = crate::lock_db(conn)?;
    let sql = format!("UPDATE games SET {column} = ?1 WHERE id = ?2");
    c.execute(&sql, rusqlite::params![value, id]).map_err(err)?;
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
                trophy_id TEXT NOT NULL DEFAULT '',
                native_id TEXT NOT NULL DEFAULT '',
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
                game_folder TEXT NOT NULL DEFAULT '',
                playtime REAL NOT NULL DEFAULT 0.0,
                cached_earned_count INTEGER NOT NULL DEFAULT 0,
                cached_total_count INTEGER NOT NULL DEFAULT 0,
                cached_achievement_mtime INTEGER NOT NULL DEFAULT 0,
                api_dll_folder TEXT NOT NULL DEFAULT '',
                saves_centralized INTEGER NOT NULL DEFAULT 0,
                hashes TEXT NOT NULL DEFAULT '',
                title_trusted INTEGER NOT NULL DEFAULT 0,
                vanished INTEGER NOT NULL DEFAULT 0,
                developer TEXT NOT NULL DEFAULT '',
                publisher TEXT NOT NULL DEFAULT '',
                genre TEXT NOT NULL DEFAULT '',
                players TEXT NOT NULL DEFAULT '',
                synopsis TEXT NOT NULL DEFAULT '',
                screenscraper_id TEXT NOT NULL DEFAULT '',
                screenscraper_rating REAL NOT NULL DEFAULT -1,
                release_dates TEXT NOT NULL DEFAULT ''
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_steam_id_platform ON games(steam_id, platform_id) WHERE steam_id != '';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_trophy_id_platform ON games(trophy_id, platform_id) WHERE trophy_id != '';
            CREATE UNIQUE INDEX IF NOT EXISTS idx_games_native_id_platform ON games(native_id, platform_id) WHERE native_id != '';
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
            CREATE TABLE IF NOT EXISTS auto_groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                criteria TEXT NOT NULL DEFAULT '[]'
            );
            CREATE INDEX IF NOT EXISTS idx_game_groups_group ON game_groups(group_id);
            CREATE TABLE IF NOT EXISTS game_playtime_links (
                game_id INTEGER PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
                group_id INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS scraper_companies (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                user_renamed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS scraper_game_companies (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                company_id INTEGER NOT NULL REFERENCES scraper_companies(id) ON DELETE CASCADE,
                is_developer INTEGER NOT NULL DEFAULT 0,
                is_publisher INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (game_id, company_id)
            );
            CREATE INDEX IF NOT EXISTS idx_scraper_game_companies_company
                ON scraper_game_companies(company_id);
            CREATE TABLE IF NOT EXISTS scraper_game_genres (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                genre_id INTEGER NOT NULL REFERENCES scraper_genres(id) ON DELETE CASCADE,
                PRIMARY KEY (game_id, genre_id)
            );
            CREATE INDEX IF NOT EXISTS idx_scraper_game_genres_genre
                ON scraper_game_genres(genre_id);
            CREATE TABLE IF NOT EXISTS scraper_game_classifications (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                value TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (game_id, kind)
            );
            CREATE TABLE IF NOT EXISTS scraper_genres (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                user_renamed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS scraper_families (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                user_renamed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS scraper_fetch_log (
                kind TEXT PRIMARY KEY,
                fetched_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS scraper_aliases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                alias TEXT NOT NULL,
                target_id INTEGER NOT NULL,
                UNIQUE (kind, alias)
            );
            CREATE TABLE IF NOT EXISTS scraper_game_families (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                family_id INTEGER NOT NULL REFERENCES scraper_families(id) ON DELETE CASCADE,
                PRIMARY KEY (game_id, family_id)
            );
            CREATE INDEX IF NOT EXISTS idx_scraper_game_families_family
                ON scraper_game_families(family_id);
            CREATE TABLE IF NOT EXISTS match_misses (
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                source TEXT NOT NULL,
                checked_at INTEGER NOT NULL,
                PRIMARY KEY (game_id, source)
            );
            CREATE TABLE IF NOT EXISTS steam_garnish (
                game_id INTEGER PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
                checked_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS game_variants (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                exe TEXT NOT NULL DEFAULT '',
                working_dir TEXT NOT NULL DEFAULT '',
                args TEXT NOT NULL DEFAULT '',
                env_vars TEXT NOT NULL DEFAULT '[]',
                sort_order INTEGER NOT NULL DEFAULT 0,
                pre_launch TEXT NOT NULL DEFAULT '',
                custom_images INTEGER NOT NULL DEFAULT 0,
                show_as_entry INTEGER NOT NULL DEFAULT 0,
                playtime REAL NOT NULL DEFAULT 0.0,
                last_played INTEGER NOT NULL DEFAULT 0,
                count_playtime INTEGER NOT NULL DEFAULT 1,
                logo_position TEXT NOT NULL DEFAULT '',
                logo_size INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS game_default_variant (
                game_id INTEGER PRIMARY KEY,
                variant_id INTEGER
            );
            CREATE TABLE IF NOT EXISTS game_discs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                disc_number INTEGER NOT NULL,
                rom_path TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS game_default_disc (
                game_id INTEGER PRIMARY KEY,
                disc_id INTEGER
            );
            CREATE TABLE IF NOT EXISTS rom_serials (
                rom_path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                serial TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL DEFAULT ''
            );",
        ).expect("failed to create tables");
        // Serial-number indexes keyed by the models' kind strings — raw
        // literals here would silently diverge from GameKind's serialized form.
        // Serials live in native_id (platform_id is the system scope, one
        // value per kind, so it can no longer disambiguate installs).
        conn.execute_batch(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps4_serial ON games(kind, native_id) WHERE kind = '{}';
             CREATE UNIQUE INDEX IF NOT EXISTS idx_games_ps3_serial ON games(kind, native_id) WHERE kind = '{}';",
            ira_models::GameKind::Ps4.as_str(),
            ira_models::GameKind::Ps3.as_str(),
        ))
        .expect("failed to create kind serial indexes");
    }

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
