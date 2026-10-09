use crate::{err, DbConn};
use rusqlite::params;

/// Copy-guarded UPDATE used when merging duplicates: moves child rows to the
/// canonical game only if it has none yet, so its existing values win.
fn copy_guarded_update_sql(table: &str) -> String {
    format!(
        "UPDATE {table} SET game_id = ?1\n             WHERE game_id = ?2 AND NOT EXISTS (\n                 SELECT 1 FROM {table} WHERE game_id = ?1\n             )"
    )
}

fn reassign_child_rows(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    canonical_id: i64,
    duplicate_id: i64,
) -> Result<(), String> {
    tx.execute(
        &copy_guarded_update_sql(table),
        params![canonical_id, duplicate_id],
    )
    .map_err(err)?;
    tx.execute(
        &format!("DELETE FROM {table} WHERE game_id = ?1"),
        params![duplicate_id],
    )
    .map_err(err)?;
    Ok(())
}

fn merge_one_duplicate(
    tx: &rusqlite::Transaction<'_>,
    canonical_id: i64,
    duplicate_id: i64,
) -> Result<(), String> {
    tx.execute(
        "UPDATE games
         SET title = CASE WHEN title = '' THEN (SELECT title FROM games WHERE id = ?2) ELSE title END,
              sort_title = CASE WHEN sort_title = '' THEN (SELECT sort_title FROM games WHERE id = ?2) ELSE sort_title END,
              hidden = MAX(hidden, (SELECT hidden FROM games WHERE id = ?2)),
              logo_position = CASE WHEN logo_position = 'bottom-left' THEN (SELECT logo_position FROM games WHERE id = ?2) ELSE logo_position END,
              logo_size = CASE WHEN logo_size = 50 THEN (SELECT logo_size FROM games WHERE id = ?2) ELSE logo_size END,
              playtime = playtime + (SELECT playtime FROM games WHERE id = ?2),
              last_played = MAX(last_played, (SELECT last_played FROM games WHERE id = ?2))
         WHERE id = ?1",
        params![canonical_id, duplicate_id],
    )
    .map_err(err)?;
    tx.execute(
        "INSERT OR IGNORE INTO game_groups (game_id, group_id)
         SELECT ?1, group_id FROM game_groups WHERE game_id = ?2",
        params![canonical_id, duplicate_id],
    )
    .map_err(err)?;
    tx.execute(
        "DELETE FROM game_groups WHERE game_id = ?1",
        params![duplicate_id],
    )
    .map_err(err)?;
    for (table, column) in [
        ("game_variants", "game_id"),
        ("play_sessions", "game_id"),
        ("game_discs", "game_id"),
    ] {
        tx.execute(
            &format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"),
            params![canonical_id, duplicate_id],
        )
        .map_err(err)?;
    }
    for table in ["game_configs", "game_default_variant", "game_default_disc"] {
        reassign_child_rows(tx, table, canonical_id, duplicate_id)?;
    }
    tx.execute("DELETE FROM games WHERE id = ?1", params![duplicate_id])
        .map_err(err)?;
    Ok(())
}

pub fn merge_duplicate_games(
    conn: &DbConn,
    canonical_id: i64,
    duplicate_ids: &[i64],
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    for &duplicate_id in duplicate_ids {
        if duplicate_id == canonical_id {
            continue;
        }
        merge_one_duplicate(&tx, canonical_id, duplicate_id)?;
    }
    // A duplicate carried a playtime link? Its row CASCADEd away with the
    // deletion; prune whatever link that left incomplete.
    crate::links::prune_incomplete_groups(&tx)?;
    tx.commit().map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{add_disc, add_game, add_game_to_group, create_group, get_discs, get_sessions_for_game};
    use super::super::{find_by_db_id, init_db, record_session, update_field};
    use super::*;
    use ira_models::{GameDisc, GameKind, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_merge_duplicate_games_preserves_related_data() {
        let (conn, _tmp) = setup_db();
        let canonical = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "canonical",
                platform_id: "saturn",
                title: "Canonical",
            },
        )
        .unwrap();
        let duplicate = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "duplicate",
                platform_id: "saturn",
                title: "Duplicate",
            },
        )
        .unwrap();
        let group = create_group(&conn, "Favorites").unwrap();
        add_game_to_group(&conn, duplicate, group).unwrap();
        update_field(&conn, duplicate, "playtime", &0.1_f64).unwrap();
        record_session(&conn, duplicate, None, 1000, 1301).unwrap();
        add_disc(
            &conn,
            &GameDisc {
                id: 0,
                game_id: duplicate,
                disc_number: 2,
                rom_path: "disc2.chd".to_string(),
                label: "Disc 2".to_string(),
            },
        )
        .unwrap();

        merge_duplicate_games(&conn, canonical, &[duplicate]).unwrap();

        assert!(find_by_db_id(&conn, duplicate).unwrap().is_none());
        assert_eq!(get_discs(&conn, canonical).unwrap().len(), 1);
        assert_eq!(crate::get_all_groups(&conn).unwrap()[0].id, group);
        assert_eq!(
            get_sessions_for_game(&conn, canonical, None).unwrap().len(),
            1
        );
        assert!((find_by_db_id(&conn, canonical).unwrap().unwrap().playtime - 0.1).abs() < 0.001);
    }
}
