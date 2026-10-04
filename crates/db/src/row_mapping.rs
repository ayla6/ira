use crate::DbConn;
use ira_models::{GameEntry, GameKind, TrophySource};
use r2d2_sqlite::SqliteConnectionManager;

/// Every games column the app reads, in one place so queries and the
/// row mapper can't drift. `game_entry_from_row` reads them by name —
/// positional reads here have bitten before (inserting a column used to
/// silently shift every field after it).
pub(crate) const GAME_COLUMNS: &str = "id, kind, trophy_source, steam_id, trophy_id, platform_id, title, hidden, sgdb_id, logo_position, logo_size, manual_unmatch, sort_title, shadps4_version, last_played, release_date, release_timestamp, metacritic_score, steam_review_score, steam_review_count, ra_core, emulator_override, rom_path, game_folder, playtime, cached_earned_count, cached_total_count, cached_achievement_mtime, hashes, vanished, players, synopsis, screenscraper_id, screenscraper_rating, release_dates, title_trusted, native_id";

pub(crate) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<GameEntry> {
    Ok(GameEntry {
        id: row.get("id")?,
        kind: GameKind::from_string(&row.get::<_, String>("kind")?),
        trophy_source: TrophySource::from_string(&row.get::<_, String>("trophy_source")?),
        steam_id: row.get("steam_id")?,
        trophy_id: row.get("trophy_id")?,
        platform_id: row.get("platform_id")?,
        title: row.get("title")?,
        hidden: row.get("hidden")?,
        sgdb_id: row.get("sgdb_id")?,
        logo_position: row.get("logo_position")?,
        logo_size: row.get("logo_size")?,
        manual_unmatch: row.get("manual_unmatch")?,
        sort_title: row.get("sort_title")?,
        shadps4_version: row.get("shadps4_version")?,
        last_played: row.get("last_played")?,
        release_date: row.get("release_date")?,
        release_timestamp: row.get("release_timestamp")?,
        metacritic_score: row.get("metacritic_score")?,
        steam_review_score: row.get("steam_review_score")?,
        steam_review_count: row.get("steam_review_count")?,
        ra_core: row.get("ra_core")?,
        emulator_override: row.get("emulator_override")?,
        rom_path: row.get("rom_path")?,
        game_folder: row.get("game_folder")?,
        playtime: row.get("playtime")?,
        cached_earned_count: row.get("cached_earned_count")?,
        cached_total_count: row.get("cached_total_count")?,
        cached_achievement_mtime: row.get("cached_achievement_mtime")?,
        hashes: {
            let raw: String = row.get("hashes")?;
            if raw.is_empty() {
                Default::default()
            } else {
                serde_json::from_str(&raw).unwrap_or_else(|e| {
                    let id: rusqlite::Result<i64> = row.get("id");
                    eprintln!(
                        "Corrupt hashes JSON for game {:?}: {e}",
                        id.unwrap_or(-1)
                    );
                    Default::default()
                })
            }
        },
        vanished: row.get("vanished")?,
        players: row.get("players")?,
        synopsis: row.get("synopsis")?,
        screenscraper_id: row.get("screenscraper_id")?,
        screenscraper_rating: row.get("screenscraper_rating")?,
        release_dates: row.get("release_dates")?,
        title_trusted: row.get("title_trusted")?,
        native_id: row.get::<_, String>("native_id")?,
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
