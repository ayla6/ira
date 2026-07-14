use rusqlite::Connection;

fn column_exists(conn: &Connection, table: &str, col: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({})", table)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect();
    names.iter().any(|name| name == col)
}

fn ensure_column(conn: &Connection, table: &str, col: &str, def: &str) {
    if !column_exists(conn, table, col) {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, def);
        if let Err(e) = conn.execute(&sql, []) {
            eprintln!("Migration: could not add column {}.{} ({}): {}", table, col, def, e);
        }
    }
}

pub fn run_schema_migrations(conn: &Connection) {
    ensure_column(conn, "games", "release_date", "TEXT NOT NULL DEFAULT ''");
    ensure_column(conn, "games", "release_timestamp", "INTEGER NOT NULL DEFAULT 0");
    ensure_column(conn, "games", "metacritic_score", "INTEGER NOT NULL DEFAULT -1");
    ensure_column(conn, "games", "steam_review_score", "INTEGER NOT NULL DEFAULT -1");
    ensure_column(conn, "games", "steam_review_count", "INTEGER NOT NULL DEFAULT 0");
    ensure_column(conn, "games", "ra_core", "TEXT NOT NULL DEFAULT ''");
    ensure_column(conn, "games", "emulator_override", "TEXT NOT NULL DEFAULT ''");
    // Drop obsolete unique indexes that prevented multiple retro games per console
    let _ = conn.execute("DROP INDEX IF EXISTS idx_games_trophy_platform", []);
    let _ = conn.execute("DROP INDEX IF EXISTS idx_games_kind_platform", []);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE games (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT ''
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_ensure_column_adds_missing() {
        let conn = temp_conn();
        assert!(!column_exists(&conn, "games", "release_date"));
        ensure_column(&conn, "games", "release_date", "TEXT NOT NULL DEFAULT ''");
        assert!(column_exists(&conn, "games", "release_date"));
    }

    #[test]
    fn test_ensure_column_idempotent() {
        let conn = temp_conn();
        ensure_column(&conn, "games", "release_date", "TEXT NOT NULL DEFAULT ''");
        ensure_column(&conn, "games", "release_date", "TEXT NOT NULL DEFAULT ''");
        assert!(column_exists(&conn, "games", "release_date"));
    }

    #[test]
    fn test_run_schema_migrations() {
        let conn = temp_conn();
        run_schema_migrations(&conn);
        assert!(column_exists(&conn, "games", "release_date"));
        assert!(column_exists(&conn, "games", "release_timestamp"));
        assert!(column_exists(&conn, "games", "metacritic_score"));
        assert!(column_exists(&conn, "games", "steam_review_score"));
    assert!(column_exists(&conn, "games", "steam_review_count"));
    assert!(column_exists(&conn, "games", "ra_core"));
}
}
