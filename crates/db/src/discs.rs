use crate::{err, DbConn};
use ira_models::GameDisc;
use rusqlite::params;

/// Returns all disc rom_paths for retro games on the given platform.
/// Used to check whether a scanned ROM is already known (in the DB)
/// without spawning `ira-disc-info` subprocesses.
pub fn get_disc_paths_for_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    Ok(get_disc_owners_for_platform(conn, platform_id)?
        .into_keys()
        .collect())
}

/// Returns each known disc path and the game row it belongs to.
pub fn get_disc_owners_for_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let rows: Vec<(String, i64)> = crate::query_all(
        conn,
        &format!(
            "SELECT gd.rom_path, gd.game_id FROM game_discs gd
         JOIN games g ON gd.game_id = g.id
         WHERE g.kind = '{}' AND g.platform_id = ?1",
            ira_models::GameKind::Retro.as_str()
        ),
        params![platform_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(rows.into_iter().collect())
}

pub fn get_discs(conn: &DbConn, game_id: i64) -> Result<Vec<GameDisc>, String> {
    crate::query_all(
        conn,
        "SELECT id, game_id, disc_number, rom_path, label FROM game_discs WHERE game_id = ?1 ORDER BY disc_number",
        params![game_id],
        |row| {
            Ok(GameDisc {
                id: row.get(0)?,
                game_id: row.get(1)?,
                disc_number: row.get(2)?,
                rom_path: row.get(3)?,
                label: row.get(4)?,
            })
        },
    )
}

pub fn add_disc(conn: &DbConn, disc: &GameDisc) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO game_discs (game_id, disc_number, rom_path, label) VALUES (?1, ?2, ?3, ?4)",
        params![disc.game_id, disc.disc_number, disc.rom_path, disc.label],
    )
    .map_err(err)?;
    Ok(c.last_insert_rowid())
}

pub fn delete_discs(conn: &DbConn, game_id: i64) -> Result<(), String> {
    crate::execute(conn, "DELETE FROM game_discs WHERE game_id = ?1", params![game_id])?;
    Ok(())
}

pub fn get_default_disc(conn: &DbConn, game_id: i64) -> Result<Option<i64>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT disc_id FROM game_default_disc WHERE game_id = ?1",
        params![game_id],
    )
}

pub fn set_default_disc(conn: &DbConn, game_id: i64, disc_id: Option<i64>) -> Result<(), String> {
    if let Some(did) = disc_id {
        crate::execute(
            conn,
            "INSERT INTO game_default_disc (game_id, disc_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET disc_id = excluded.disc_id",
            params![game_id, did],
        )?;
    } else {
        crate::execute(conn, "DELETE FROM game_default_disc WHERE game_id = ?1", params![game_id])?;
    }
    Ok(())
}
