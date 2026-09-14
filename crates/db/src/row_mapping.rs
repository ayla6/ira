use crate::DbConn;
use ira_models::{GameEntry, GameKind, TrophySource};
use r2d2_sqlite::SqliteConnectionManager;

pub(crate) const GAME_COLUMNS: &str = "id, kind, trophy_source, steam_id, ra_id, platform_id, title, hidden, sgdb_id, logo_position, logo_size, manual_unmatch, sort_title, shadps4_version, last_played, release_date, release_timestamp, metacritic_score, steam_review_score, steam_review_count, ra_core, emulator_override, rom_path, game_folder, playtime, cached_earned_count, cached_total_count, cached_achievement_mtime, hashes, vanished, developer, publisher, genre, players, synopsis, screenscraper_id, screenscraper_rating, release_dates, developer_id, publisher_id, genre_ids, classification_ids, title_trusted, native_id";

pub(crate) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<GameEntry> {
    Ok(GameEntry {
        id: row.get(0)?,
        kind: GameKind::from_string(&row.get::<_, String>(1)?),
        trophy_source: TrophySource::from_string(&row.get::<_, String>(2)?),
        steam_id: row.get(3)?,
        ra_id: row.get(4)?,
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
        game_folder: row.get(23)?,
        playtime: row.get(24)?,
        cached_earned_count: row.get(25)?,
        cached_total_count: row.get(26)?,
        cached_achievement_mtime: row.get(27)?,
        hashes: serde_json::from_str(
            &row.get::<_, String>(28)?,
        )
        .unwrap_or_default(),
        vanished: row.get(29)?,
        players: row.get(33)?,
        synopsis: row.get(34)?,
        screenscraper_id: row.get(35)?,
        screenscraper_rating: row.get(36)?,
        release_dates: row.get(37)?,
        developer_id: row.get(38)?,
        publisher_id: row.get(39)?,
        genre_ids: row.get(40)?,
        classification_ids: row.get(41)?,
        title_trusted: row.get::<_, i64>(42)? != 0,
        native_id: row.get::<_, String>(43)?,
    })
}

pub(crate) fn lock_db(
    conn: &DbConn,
) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, String> {
    conn.get().map_err(err)
}

/// Maps any error type onto the crate-wide `String` error channel, so call
/// sites can write `.map_err(err)?` instead of repeating `|e| e.to_string()`.
pub(crate) fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
