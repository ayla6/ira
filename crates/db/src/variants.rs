use crate::DbConn;
use ira_models::GameVariant;
use rusqlite::params;

pub fn create_variants_table(conn: &DbConn) {
    let c = crate::lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_variants (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            exe TEXT NOT NULL DEFAULT '',
            working_dir TEXT NOT NULL DEFAULT '',
            args TEXT NOT NULL DEFAULT '',
            env_vars TEXT NOT NULL DEFAULT '[]',
            sort_order INTEGER NOT NULL DEFAULT 0,
            pre_launch TEXT NOT NULL DEFAULT '',
            custom_images INTEGER NOT NULL DEFAULT 0,
            show_as_entry INTEGER NOT NULL DEFAULT 0,
            playtime REAL NOT NULL DEFAULT 0.0,
            last_played INTEGER NOT NULL DEFAULT 0,
            count_playtime INTEGER NOT NULL DEFAULT 1,
            logo_position TEXT NOT NULL DEFAULT '',
            logo_size INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create game_variants table");
}

pub fn get_variants(conn: &DbConn, game_id: i64) -> Result<Vec<GameVariant>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, game_id, name, exe, working_dir, args, env_vars, sort_order, pre_launch, custom_images, show_as_entry, playtime, last_played, count_playtime, logo_position, logo_size FROM game_variants WHERE game_id = ?1 ORDER BY sort_order, id"
    ).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![game_id], |row| {
            let env_str: String = row.get(6)?;
            let env_vars: Vec<(String, String)> =
                serde_json::from_str(&env_str).unwrap_or_default();
            Ok(GameVariant {
                id: row.get(0)?,
                game_id: row.get(1)?,
                name: row.get(2)?,
                exe: row.get(3)?,
                working_dir: row.get(4)?,
                args: row.get(5)?,
                env_vars,
                sort_order: row.get(7)?,
                pre_launch: row.get(8)?,
                custom_images: row.get::<_, i64>(9)? != 0,
                show_as_entry: row.get::<_, i64>(10)? != 0,
                playtime: row.get(11)?,
                last_played: row.get(12)?,
                count_playtime: row.get::<_, i64>(13)? != 0,
                logo_position: row.get(14)?,
                logo_size: row.get(15)?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn add_variant(conn: &DbConn, variant: &GameVariant) -> Result<i64, String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(|e| e.to_string())?;
    let c = crate::lock_db(conn)?;
    let sort_order: i32 = c
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM game_variants WHERE game_id = ?1",
            params![variant.game_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    c.execute(
        "INSERT INTO game_variants (game_id, name, exe, working_dir, args, env_vars, sort_order, pre_launch, custom_images, show_as_entry, count_playtime, logo_position, logo_size) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![variant.game_id, variant.name, variant.exe, variant.working_dir, variant.args, env_str, sort_order, variant.pre_launch, variant.custom_images as i64, variant.show_as_entry as i64, variant.count_playtime as i64, variant.logo_position, variant.logo_size],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn update_variant(conn: &DbConn, variant: &GameVariant) -> Result<(), String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(|e| e.to_string())?;
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE game_variants SET name=?1, exe=?2, working_dir=?3, args=?4, env_vars=?5, pre_launch=?6, custom_images=?7, show_as_entry=?8, count_playtime=?9, logo_position=?10, logo_size=?11 WHERE id=?12",
        params![variant.name, variant.exe, variant.working_dir, variant.args, env_str, variant.pre_launch, variant.custom_images as i64, variant.show_as_entry as i64, variant.count_playtime as i64, variant.logo_position, variant.logo_size, variant.id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn reorder_variants(conn: &DbConn, ordered_ids: &[i64]) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
    for (i, id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "UPDATE game_variants SET sort_order = ?1 WHERE id = ?2",
            params![i as i32, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_variant(conn: &DbConn, variant_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM game_variants WHERE id = ?1",
        params![variant_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_default_variant(conn: &DbConn, game_id: i64) -> Result<Option<i64>, String> {
    let c = crate::lock_db(conn)?;
    match c.query_row(
        "SELECT variant_id FROM game_default_variant WHERE game_id = ?1",
        params![game_id],
        |row| row.get(0),
    ) {
        Ok(vid) => Ok(Some(vid)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_default_variant(
    conn: &DbConn,
    game_id: i64,
    variant_id: Option<i64>,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    if let Some(vid) = variant_id {
        c.execute(
            "INSERT INTO game_default_variant (game_id, variant_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET variant_id = excluded.variant_id",
            params![game_id, vid],
        )
        .map_err(|e| e.to_string())?;
    } else {
        c.execute(
            "DELETE FROM game_default_variant WHERE game_id = ?1",
            params![game_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn delete_all_variants(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM game_variants WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_variant_playtime(
    conn: &DbConn,
    variant_id: i64,
    playtime: f64,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE game_variants SET playtime = ?1 WHERE id = ?2",
        params![playtime, variant_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_variant_last_played(
    conn: &DbConn,
    variant_id: i64,
    last_played: i64,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE game_variants SET last_played = ?1 WHERE id = ?2",
        params![last_played, variant_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
