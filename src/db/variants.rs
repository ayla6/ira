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
            env_vars TEXT NOT NULL DEFAULT '[]',
            emu_version TEXT NOT NULL DEFAULT '',
            emu_installed INTEGER NOT NULL DEFAULT 0
        );"
    ).expect("create game_variants table");
}

pub fn get_variants(conn: &DbConn, game_id: i64) -> Result<Vec<GameVariant>, String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = c.prepare(
        "SELECT id, game_id, name, exe, working_dir, args, env_vars, emu_version, emu_installed FROM game_variants WHERE game_id = ?1 ORDER BY id"
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
            emu_version: row.get(7)?,
            emu_installed: row.get::<_, i32>(8)? != 0,
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
        "INSERT INTO game_variants (game_id, name, exe, working_dir, args, env_vars, emu_version, emu_installed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![variant.game_id, variant.name, variant.exe, variant.working_dir, variant.args, env_str, variant.emu_version, variant.emu_installed as i32],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn update_variant(conn: &DbConn, variant: &GameVariant) -> Result<(), String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(|e| e.to_string())?;
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE game_variants SET name=?1, exe=?2, working_dir=?3, args=?4, env_vars=?5, emu_version=?6, emu_installed=?7 WHERE id=?8",
        params![variant.name, variant.exe, variant.working_dir, variant.args, env_str, variant.emu_version, variant.emu_installed as i32, variant.id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_variant(conn: &DbConn, variant_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM game_variants WHERE id = ?1", params![variant_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_all_variants(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = conn.lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM game_variants WHERE game_id = ?1", params![game_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
