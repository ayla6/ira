use crate::DbConn;
use ira_models::GameDisc;
use rusqlite::params;

pub fn create_discs_table(conn: &DbConn) {
    let c = crate::lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_discs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
            disc_number INTEGER NOT NULL,
            rom_path TEXT NOT NULL,
            label TEXT NOT NULL DEFAULT ''
        );"
    ).expect("create game_discs table");
}

pub fn get_discs(conn: &DbConn, game_id: i64) -> Result<Vec<GameDisc>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, game_id, disc_number, rom_path, label FROM game_discs WHERE game_id = ?1 ORDER BY disc_number"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![game_id], |row| {
        Ok(GameDisc {
            id: row.get(0)?,
            game_id: row.get(1)?,
            disc_number: row.get(2)?,
            rom_path: row.get(3)?,
            label: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn add_disc(conn: &DbConn, disc: &GameDisc) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO game_discs (game_id, disc_number, rom_path, label) VALUES (?1, ?2, ?3, ?4)",
        params![disc.game_id, disc.disc_number, disc.rom_path, disc.label],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn delete_discs(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM game_discs WHERE game_id = ?1", params![game_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_default_disc(conn: &DbConn, game_id: i64) -> Option<i64> {
    let c = crate::lock_db(conn).ok()?;
    c.query_row(
        "SELECT disc_id FROM game_default_disc WHERE game_id = ?1",
        params![game_id],
        |row| row.get(0),
    ).ok()
}

pub fn set_default_disc(conn: &DbConn, game_id: i64, disc_id: Option<i64>) {
    let Ok(c) = crate::lock_db(conn) else { return; };
    if let Some(did) = disc_id {
        let _ = c.execute(
            "INSERT INTO game_default_disc (game_id, disc_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET disc_id = excluded.disc_id",
            params![game_id, did],
        );
    } else {
        let _ = c.execute(
            "DELETE FROM game_default_disc WHERE game_id = ?1",
            params![game_id],
        );
    }
}

pub fn create_default_disc_table(conn: &DbConn) {
    let c = crate::lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_default_disc (
            game_id INTEGER PRIMARY KEY,
            disc_id INTEGER
        );"
    ).expect("create game_default_disc table");
}
