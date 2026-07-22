use crate::DbConn;
use ira_models::{GameEntry, GameKind, TrophySource};
use r2d2_sqlite::SqliteConnectionManager;

pub(crate) const GAME_COLUMNS: &str = "id, kind, trophy_source, steam_id, game_id, platform_id, title, hidden, sgdb_id, logo_position, logo_size, manual_unmatch, sort_title, shadps4_version, last_played, release_date, release_timestamp, metacritic_score, steam_review_score, steam_review_count, ra_core, emulator_override, rom_path, playtime, cached_earned_count, cached_total_count, cached_achievement_mtime";

pub(crate) fn game_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<GameEntry> {
    Ok(GameEntry {
        id: row.get(0)?,
        kind: GameKind::from_string(&row.get::<_, String>(1)?),
        trophy_source: TrophySource::from_string(&row.get::<_, String>(2)?),
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

pub(crate) fn lock_db(conn: &DbConn) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, String> {
    conn.get().map_err(|e| e.to_string())
}
