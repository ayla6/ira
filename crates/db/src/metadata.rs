use crate::DbConn;
use rusqlite::params;

pub fn store_game_metadata(
    conn: &DbConn,
    game_id: i64,
    release_date: &str,
    release_timestamp: i64,
    metacritic_score: i64,
    steam_review_score: i64,
    steam_review_count: i64,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET
            release_date = ?1,
            release_timestamp = ?2,
            metacritic_score = ?3,
            steam_review_score = ?4,
            steam_review_count = ?5
         WHERE id = ?6",
        params![release_date, release_timestamp, metacritic_score, steam_review_score, steam_review_count, game_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
