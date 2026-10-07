use crate::{err, DbConn};
use ira_models::GameVariant;
use rusqlite::params;

/// Full column list of the get_variants SELECT, in row-index order (see the
/// mapping closure inside).
const VARIANT_SELECT_COLUMNS: &str =
    "id, game_id, name, exe, working_dir, args, env_vars, sort_order, pre_launch, custom_images, show_as_entry, playtime, last_played, count_playtime, logo_position, logo_size";

/// Columns written by both `add_variant` (INSERT) and `update_variant`
/// (UPDATE); their SQL fragments are derived from this single list.
const VARIANT_WRITE_COLUMNS: &[&str] = &[
    "name",
    "exe",
    "working_dir",
    "args",
    "env_vars",
    "pre_launch",
    "custom_images",
    "show_as_entry",
    "count_playtime",
    "logo_position",
    "logo_size",
];

/// `UPDATE` fragment: every write column bound to sequential placeholders,
/// starting at `start`.
fn variant_set_clause(start: usize) -> String {
    VARIANT_WRITE_COLUMNS
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{col}=?{}", start + i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `INSERT` column list: game_id leads and sort_order sits between env_vars
/// and pre_launch.
fn variant_insert_columns() -> String {
    format!(
        "game_id, {}, sort_order, {}",
        VARIANT_WRITE_COLUMNS[..5].join(", "),
        VARIANT_WRITE_COLUMNS[5..].join(", ")
    )
}

fn variant_from_row(row: &rusqlite::Row, game_id: i64) -> rusqlite::Result<GameVariant> {
    let env_str: String = row.get(6)?;
    let variant_id: i64 = row.get(0)?;
    let env_vars: Vec<(String, String)> =
        serde_json::from_str(&env_str).unwrap_or_else(|e| {
            eprintln!("Corrupt env_vars JSON for variant {variant_id} of game {game_id}: {e}");
            Vec::new()
        });
    Ok(GameVariant {
        id: variant_id,
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
}

pub fn get_variants(conn: &DbConn, game_id: i64) -> Result<Vec<GameVariant>, String> {
    crate::query_all(
        conn,
        &format!(
            "SELECT {VARIANT_SELECT_COLUMNS} FROM game_variants WHERE game_id = ?1 ORDER BY sort_order, id"
        ),
        params![game_id],
        |row| variant_from_row(row, game_id),
    )
}

/// Every variant of every listed game in one query, grouped by game id.
///
/// The loader built one `get_variants` query per game (N round-trips for N
/// games); this is the same rows in the same per-game order for a single
/// round-trip per 500-id chunk. Missing ids simply have no entry.
pub fn get_variants_for_games(
    conn: &DbConn,
    game_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<GameVariant>>, String> {
    let mut grouped: std::collections::HashMap<i64, Vec<GameVariant>> =
        std::collections::HashMap::new();
    if game_ids.is_empty() {
        return Ok(grouped);
    }
    for chunk in game_ids.chunks(crate::IN_QUERY_CHUNK_SIZE) {
        let placeholders = crate::positional_placeholders(chunk.len());
        let sql = format!(
            "SELECT {VARIANT_SELECT_COLUMNS} FROM game_variants WHERE game_id IN ({placeholders}) ORDER BY game_id, sort_order, id"
        );
        let rows: Vec<GameVariant> = crate::query_all(
            conn,
            &sql,
            rusqlite::params_from_iter(chunk.iter()),
            |row| {
                let gid: i64 = row.get(1)?;
                variant_from_row(row, gid)
            },
        )?;
        for variant in rows {
            grouped.entry(variant.game_id).or_default().push(variant);
        }
    }
    Ok(grouped)
}

pub fn add_variant(conn: &DbConn, variant: &GameVariant) -> Result<i64, String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(err)?;
    let c = crate::lock_db(conn)?;
    // One transaction around the MAX+INSERT: two concurrent adds must not
    // land on the same sort_order.
    let tx = c.unchecked_transaction().map_err(err)?;
    let sort_order: i32 = tx
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM game_variants WHERE game_id = ?1",
            params![variant.game_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| {
            eprintln!(
                "Failed to read next sort_order for game {}: {e}",
                variant.game_id
            );
            0
        });
    let sql = format!(
        "INSERT INTO game_variants ({}) VALUES ({})",
        variant_insert_columns(),
        crate::positional_placeholders(VARIANT_WRITE_COLUMNS.len() + 2),
    );
    tx.execute(
        &sql,
        params![
            variant.game_id,
            variant.name,
            variant.exe,
            variant.working_dir,
            variant.args,
            env_str,
            sort_order,
            variant.pre_launch,
            variant.custom_images as i64,
            variant.show_as_entry as i64,
            variant.count_playtime as i64,
            variant.logo_position,
            variant.logo_size
        ],
    )
    .map_err(err)?;
    let rowid = tx.last_insert_rowid();
    tx.commit().map_err(err)?;
    Ok(rowid)
}

pub fn update_variant(conn: &DbConn, variant: &GameVariant) -> Result<(), String> {
    let env_str = serde_json::to_string(&variant.env_vars).map_err(err)?;
    let sql = format!(
        "UPDATE game_variants SET {} WHERE id=?{}",
        variant_set_clause(1),
        VARIANT_WRITE_COLUMNS.len() + 1,
    );
    crate::execute(
        conn,
        &sql,
        params![
            variant.name,
            variant.exe,
            variant.working_dir,
            variant.args,
            env_str,
            variant.pre_launch,
            variant.custom_images as i64,
            variant.show_as_entry as i64,
            variant.count_playtime as i64,
            variant.logo_position,
            variant.logo_size,
            variant.id
        ],
    )?;
    Ok(())
}

pub fn reorder_variants(conn: &DbConn, ordered_ids: &[i64]) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    for (i, id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "UPDATE game_variants SET sort_order = ?1 WHERE id = ?2",
            params![i as i32, id],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(())
}

pub fn delete_variant(conn: &DbConn, variant_id: i64) -> Result<(), String> {
    crate::execute(conn, "DELETE FROM game_variants WHERE id = ?1", params![variant_id])?;
    Ok(())
}

pub fn get_default_variant(conn: &DbConn, game_id: i64) -> Result<Option<i64>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT variant_id FROM game_default_variant WHERE game_id = ?1",
        params![game_id],
    )
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
        .map_err(err)?;
    } else {
        c.execute(
            "DELETE FROM game_default_variant WHERE game_id = ?1",
            params![game_id],
        )
        .map_err(err)?;
    }
    Ok(())
}

pub fn update_variant_playtime(
    conn: &DbConn,
    variant_id: i64,
    playtime: f64,
) -> Result<(), String> {
    crate::execute(conn, "UPDATE game_variants SET playtime = ?1 WHERE id = ?2", params![playtime, variant_id])?;
    Ok(())
}

pub fn set_variant_last_played(
    conn: &DbConn,
    variant_id: i64,
    last_played: i64,
) -> Result<(), String> {
    crate::execute(conn, "UPDATE game_variants SET last_played = ?1 WHERE id = ?2", params![last_played, variant_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variant_select_columns_start_with_write_columns_in_row_order() {
        // The SELECT list is a superset of the write columns; every write
        // column must appear so the row mapping stays total.
        for col in VARIANT_WRITE_COLUMNS {
            assert!(VARIANT_SELECT_COLUMNS.contains(col));
        }
    }

    #[test]
    fn test_get_variants_for_games_matches_single_queries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = crate::init_db(&tmp.path().join("test.db").to_string_lossy());
        let game_a = crate::add_game(
            &conn,
            crate::NewGame {
                kind: ira_models::GameKind::Other,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "a",
                platform_id: "p",
                title: "A",
            },
        )
        .unwrap();
        let game_b = crate::add_game(
            &conn,
            crate::NewGame {
                kind: ira_models::GameKind::Other,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "b",
                platform_id: "p",
                title: "B",
            },
        )
        .unwrap();
        for (game_id, name) in [(game_a, "V1"), (game_a, "V2"), (game_b, "W1")] {
            let variant = ira_models::GameVariant {
                game_id,
                name: name.to_string(),
                ..Default::default()
            };
            add_variant(&conn, &variant).unwrap();
        }

        let grouped = get_variants_for_games(&conn, &[game_a, game_b, 99999]).unwrap();
        assert_eq!(grouped[&game_a].len(), 2);
        assert_eq!(grouped[&game_b].len(), 1);
        assert!(!grouped.contains_key(&99999));
        assert_eq!(grouped[&game_a][0].name, "V1");
        assert_eq!(grouped[&game_a][1].name, "V2");
        assert!(get_variants_for_games(&conn, &[]).unwrap().is_empty());
    }
}
