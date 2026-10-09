use crate::DbConn;
use rusqlite::params;

/// Miss sources for the failure tombstones below: one row per game
/// per source, so a dead ScreenScraper search never blocks a Steam one
/// and no per-source columns pile up on the games table.
pub mod miss_source {
    /// ScreenScraper title/hash matching.
    pub const SS: &str = "ss";
    /// The console→Steam exact-title pass.
    pub const STEAM: &str = "steam";
}

/// Remember that a game's automatic match for one source came up
/// empty, so the mass matcher stops re-asking every time it opens.
/// Manual picks and later matches clear it again.
pub fn tombstone_match_miss(conn: &DbConn, game_id: i64, source: &str) -> Result<(), String> {
    crate::execute(
        conn,
        "INSERT INTO match_misses (game_id, source, checked_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(game_id, source) DO UPDATE SET checked_at = excluded.checked_at",
        params![game_id, source, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

pub fn match_missed_ids(conn: &DbConn, source: &str) -> Result<Vec<i64>, String> {
    crate::query_scalar_all(
        conn,
        "SELECT game_id FROM match_misses WHERE source = ?1",
        [source],
    )
}

/// Forget a game's miss for one source — something matched it after all.
pub fn clear_match_miss(conn: &DbConn, game_id: i64, source: &str) -> Result<(), String> {
    crate::execute(
        conn,
        "DELETE FROM match_misses WHERE game_id = ?1 AND source = ?2",
        params![game_id, source],
    )?;
    Ok(())
}

/// Remember that Steam's rich sources (store page, steamcmd) were
/// consulted for a game, so later refetches stop re-asking a question
/// already answered. Marked on a good store-page answer; transport
/// failures stay unmarked and retry.
pub fn mark_steam_garnished(conn: &DbConn, game_id: i64) -> Result<(), String> {
    crate::execute(
        conn,
        "INSERT INTO steam_garnish (game_id, checked_at) VALUES (?1, ?2)
         ON CONFLICT(game_id) DO UPDATE SET checked_at = excluded.checked_at",
        params![game_id, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

/// The games Steam already answered for: consulted, even when there
/// was nothing new to merge.
pub fn steam_garnished_ids(conn: &DbConn) -> Result<Vec<i64>, String> {
    crate::query_scalar_all(conn, "SELECT game_id FROM steam_garnish", [])
}

/// A game's Steam identity moved: any earlier consult answered for a
/// different store entry, so forget it. Called wherever `steam_id` is
/// written — the marker is only valid for the current id.
pub fn clear_steam_garnish(conn: &DbConn, game_id: i64) -> Result<(), String> {
    crate::execute(conn, "DELETE FROM steam_garnish WHERE game_id = ?1", params![game_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{add_game, init_db};
    use super::*;
    use ira_models::{GameKind, TrophySource};
    use tempfile::TempDir;

    fn test_game(conn: &DbConn) -> i64 {
        add_game(
            conn,
            crate::NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Obscure Rom",
            },
        )
        .unwrap()
    }

    #[test]
    fn test_scraper_miss_tombstone_round_trip() {
        let tmp = TempDir::new().unwrap();
        let conn = init_db(&tmp.path().join("test.db").to_string_lossy());
        let id = test_game(&conn);

        assert!(match_missed_ids(&conn, miss_source::SS).unwrap().is_empty());
        tombstone_match_miss(&conn, id, miss_source::SS).unwrap();
        assert_eq!(match_missed_ids(&conn, miss_source::SS).unwrap(), vec![id]);
        // Re-missing the same game stays one row, refreshed.
        tombstone_match_miss(&conn, id, miss_source::SS).unwrap();
        assert_eq!(match_missed_ids(&conn, miss_source::SS).unwrap(), vec![id]);
        // Sources stay separate: an SS miss is no Steam miss.
        assert!(match_missed_ids(&conn, miss_source::STEAM).unwrap().is_empty());
        tombstone_match_miss(&conn, id, miss_source::STEAM).unwrap();
        assert_eq!(match_missed_ids(&conn, miss_source::STEAM).unwrap(), vec![id]);
        // A later match clears it and the game re-enters the pool.
        clear_match_miss(&conn, id, miss_source::SS).unwrap();
        assert!(match_missed_ids(&conn, miss_source::SS).unwrap().is_empty());
        assert_eq!(match_missed_ids(&conn, miss_source::STEAM).unwrap(), vec![id]);
    }

    #[test]
    fn test_steam_garnish_marks_and_clears() {
        let tmp = TempDir::new().unwrap();
        let conn = init_db(&tmp.path().join("test.db").to_string_lossy());
        let id = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "100",
                trophy_id: "",
                native_id: "",
                platform_id: "steam",
                title: "G",
            },
        )
        .unwrap();
        assert!(steam_garnished_ids(&conn).unwrap().is_empty());
        mark_steam_garnished(&conn, id).unwrap();
        assert_eq!(steam_garnished_ids(&conn).unwrap(), vec![id]);
        // Re-marking stays one row, refreshed.
        mark_steam_garnished(&conn, id).unwrap();
        assert_eq!(steam_garnished_ids(&conn).unwrap(), vec![id]);
        // A Steam identity move forgets the consult.
        clear_steam_garnish(&conn, id).unwrap();
        assert!(steam_garnished_ids(&conn).unwrap().is_empty());
    }
}
