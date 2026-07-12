use crate::db::DbConn;
use crate::models::GameVariant;
use rusqlite::params;

pub fn create_variants_table(conn: &DbConn) {
    let c = conn.lock().expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_variants (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            exe TEXT NOT NULL DEFAULT '',
            working_dir TEXT NOT NULL DEFAULT '',
            args TEXT NOT NULL DEFAULT '',
            env_vars TEXT NOT NULL DEFAULT '[]'
        );"
    ).expect("create game_variants table");
}

pub fn get_variants(conn: &DbConn, game_id: i64) -> Result<Vec<GameVariant>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare(
        "SELECT id, game_id, name, exe, working_dir, args, env_vars FROM game_variants WHERE game_id = ?1 ORDER BY id"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![game_id], |row| {
        let env_str: String = row.get(6)?;
        let env_vars: Vec<(String, String)> = serde_json::from_str(&env_str).unwrap_or_default();
        Ok(GameVariant {
            id: row.get(0)?,
            game_id: row.get(1)?,
            name: row.get(2)?,
            exe: row.get(3)?,
            working_dir: row.get(4)?,
            args: row.get(5)?,
            env_vars,
        })
    }).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn add_variant(conn: &DbConn, variant: &GameVariant) -> Result<i64, String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(|e| e.to_string())?;
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO game_variants (game_id, name, exe, working_dir, args, env_vars) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![variant.game_id, variant.name, variant.exe, variant.working_dir, variant.args, env_str],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn update_variant(conn: &DbConn, variant: &GameVariant) -> Result<(), String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(|e| e.to_string())?;
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE game_variants SET name=?1, exe=?2, working_dir=?3, args=?4, env_vars=?5 WHERE id=?6",
        params![variant.name, variant.exe, variant.working_dir, variant.args, env_str, variant.id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_variant(conn: &DbConn, variant_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM game_variants WHERE id = ?1", params![variant_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_default_variant(conn: &DbConn, game_id: i64) -> Option<i64> {
    let c = conn.lock().ok()?;
    c.query_row(
        "SELECT variant_id FROM game_default_variant WHERE game_id = ?1",
        params![game_id],
        |row| row.get(0),
    ).ok()
}

pub fn set_default_variant(conn: &DbConn, game_id: i64, variant_id: Option<i64>) {
    let Ok(c) = conn.lock() else { return; };
    if let Some(vid) = variant_id {
        let _ = c.execute(
            "INSERT INTO game_default_variant (game_id, variant_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET variant_id = excluded.variant_id",
            params![game_id, vid],
        );
    } else {
        let _ = c.execute(
            "DELETE FROM game_default_variant WHERE game_id = ?1",
            params![game_id],
        );
    }
}

pub fn delete_all_variants(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM game_variants WHERE game_id = ?1", params![game_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
