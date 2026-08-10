use crate::DbConn;
use ira_models::GameDisc;
use rusqlite::params;

/// Returns all disc rom_paths for retro games on the given platform.
/// Used to check whether a scanned ROM is already known (in the DB)
/// without spawning `ira-disc-info` subprocesses.
pub fn get_disc_paths_for_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(
            "SELECT gd.rom_path FROM game_discs gd
         JOIN games g ON gd.game_id = g.id
         WHERE g.kind = 'retro' AND g.platform_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![platform_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut result = std::collections::HashSet::new();
    for row in rows {
        result.insert(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn create_discs_table(conn: &DbConn) {
    let c = crate::lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_discs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
            disc_number INTEGER NOT NULL,
            rom_path TEXT NOT NULL,
            label TEXT NOT NULL DEFAULT ''
        );",
    )
    .expect("create game_discs table");
}

pub fn get_discs(conn: &DbConn, game_id: i64) -> Result<Vec<GameDisc>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, game_id, disc_number, rom_path, label FROM game_discs WHERE game_id = ?1 ORDER BY disc_number"
    ).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![game_id], |row| {
            Ok(GameDisc {
                id: row.get(0)?,
                game_id: row.get(1)?,
                disc_number: row.get(2)?,
                rom_path: row.get(3)?,
                label: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn add_disc(conn: &DbConn, disc: &GameDisc) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO game_discs (game_id, disc_number, rom_path, label) VALUES (?1, ?2, ?3, ?4)",
        params![disc.game_id, disc.disc_number, disc.rom_path, disc.label],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn delete_discs(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM game_discs WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_default_disc(conn: &DbConn, game_id: i64) -> Result<Option<i64>, String> {
    let c = crate::lock_db(conn)?;
    match c.query_row(
        "SELECT disc_id FROM game_default_disc WHERE game_id = ?1",
        params![game_id],
        |row| row.get(0),
    ) {
        Ok(did) => Ok(Some(did)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_default_disc(conn: &DbConn, game_id: i64, disc_id: Option<i64>) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    if let Some(did) = disc_id {
        c.execute(
            "INSERT INTO game_default_disc (game_id, disc_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET disc_id = excluded.disc_id",
            params![game_id, did],
        )
        .map_err(|e| e.to_string())?;
    } else {
        c.execute(
            "DELETE FROM game_default_disc WHERE game_id = ?1",
            params![game_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn create_default_disc_table(conn: &DbConn) {
    let c = crate::lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_default_disc (
            game_id INTEGER PRIMARY KEY,
            disc_id INTEGER
        );",
    )
    .expect("create game_default_disc table");
}
