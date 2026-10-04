use crate::entities::{KIND_COMPANY, spec};
use crate::{err, DbConn};
use rusqlite::params;

/// Whether an id belongs to the source's namespace — real
/// ScreenScraper ids are positive; hand-minted locals are negative.
fn is_real_id(id: i64) -> bool {
    id > 0
}

/// A game holding both rows keeps one reference with the union of the
/// flags; companies only — genres and families have no roles.
fn merge_company_roles(
    tx: &rusqlite::Transaction,
    junction: &str,
    col: &str,
    survivor: i64,
    absorbed: i64,
) -> Result<(), String> {
    tx.execute(
        &format!(
            "UPDATE {junction} AS dst
             SET is_developer = MAX(dst.is_developer,
                 COALESCE((SELECT src.is_developer FROM {junction} src
                           WHERE src.game_id = dst.game_id AND src.{col} = ?1), 0)),
                 is_publisher = MAX(dst.is_publisher,
                 COALESCE((SELECT src.is_publisher FROM {junction} src
                           WHERE src.game_id = dst.game_id AND src.{col} = ?1), 0))
             WHERE {col} = ?2
               AND EXISTS (SELECT 1 FROM {junction} src
                           WHERE src.game_id = dst.game_id AND src.{col} = ?1)",
        ),
        params![survivor, absorbed],
    )
    .map_err(err)?;
    Ok(())
}

/// Every game reference moves from the absorbed row to the survivor.
fn move_junction_refs(
    tx: &rusqlite::Transaction,
    junction: &str,
    col: &str,
    survivor: i64,
    absorbed: i64,
) -> Result<(), String> {
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO {junction} (game_id, {col})
             SELECT game_id, ?1 FROM {junction} WHERE {col} = ?2",
        ),
        params![survivor, absorbed],
    )
    .map_err(err)?;
    tx.execute(
        &format!("DELETE FROM {junction} WHERE {col} = ?1"),
        params![absorbed],
    )
    .map_err(err)?;
    Ok(())
}

/// Aliases follow the references, and the absorbed spelling itself
/// becomes the survivor's alias — the dedupe's whole point.
fn repoint_aliases(
    tx: &rusqlite::Transaction,
    kind: &str,
    lookup: &str,
    survivor: i64,
    absorbed: i64,
) -> Result<(), String> {
    tx.execute(
        "UPDATE scraper_aliases SET target_id = ?1
         WHERE kind = ?2 AND target_id = ?3",
        params![survivor, kind, absorbed],
    )
    .map_err(err)?;
    let absorbed_name: Option<String> = tx
        .query_row(
            &format!("SELECT name FROM {lookup} WHERE id = ?1"),
            params![absorbed],
            |row| row.get(0),
        )
        .ok();
    if let Some(name) = absorbed_name.filter(|n| !n.trim().is_empty()) {
        tx.execute(
            "INSERT INTO scraper_aliases (kind, alias, target_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind, alias) DO UPDATE SET target_id = excluded.target_id",
            params![kind, name.trim(), survivor],
        )
        .map_err(err)?;
    }
    tx.execute(&format!("DELETE FROM {lookup} WHERE id = ?1"), params![absorbed])
        .map_err(err)?;
    Ok(())
}

/// Merge `absorbed` into `survivor`: every game reference moves over
/// (a company's developer/publisher roles take the stronger of the
/// two), aliases re-point, and the absorbed name stays behind as an
/// alias so future matches collapse onto the survivor. When exactly
/// one of the two carries a real ScreenScraper id, that row wins the
/// survivor role no matter which was picked — the source owns the id
/// space. Returns the id that survived.
pub fn merge_entities(
    conn: &DbConn,
    kind: &str,
    survivor: i64,
    absorbed: i64,
) -> Result<i64, String> {
    let spec = spec(kind);
    if survivor == absorbed {
        return Err("cannot merge an entity into itself".to_string());
    }
    // The source's id outranks a hand-minted local one — the roles
    // swap together with the survivor.
    let (survivor, absorbed) = if !is_real_id(survivor) && is_real_id(absorbed) {
        (absorbed, survivor)
    } else {
        (survivor, absorbed)
    };

    let mut c = crate::lock_db(conn)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if spec.kind == KIND_COMPANY {
        merge_company_roles(&tx, spec.junction, spec.column, survivor, absorbed)?;
    }
    move_junction_refs(&tx, spec.junction, spec.column, survivor, absorbed)?;
    repoint_aliases(&tx, spec.kind, spec.lookup, survivor, absorbed)?;
    tx.commit().map_err(err)?;
    Ok(survivor)
}

#[cfg(test)]
mod tests {
    use super::super::add_game;
    use super::super::init_db;
    use super::*;
    use crate::entities::{GENRES, KIND_GENRE};
    use ira_models::{GameKind, ScraperEntity, ScraperMetadata, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let conn = init_db(tmp.path().join("test.db").to_str().unwrap());
        (conn, tmp)
    }

    fn entity(id: i64, name: &str) -> ScraperEntity {
        ScraperEntity {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn genre_metadata(genres: Vec<ScraperEntity>) -> ScraperMetadata {
        ScraperMetadata {
            genres,
            ..Default::default()
        }
    }

    #[test]
    fn test_merge_moves_references_and_leaves_an_alias() {
        let (conn, _tmp) = setup_db();
        let a = add_game(&conn, crate::NewGame { kind: GameKind::Steam, trophy_source: TrophySource::Gse, steam_id: "", trophy_id: "", native_id: "", platform_id: "", title: "One" }).unwrap();
        let b = add_game(&conn, crate::NewGame { kind: GameKind::Steam, trophy_source: TrophySource::Gse, steam_id: "", trophy_id: "", native_id: "", platform_id: "", title: "Two" }).unwrap();
        crate::store_scraper_metadata(
            &conn,
            a,
            &genre_metadata(vec![entity(-3, "RPG")]),
        )
        .unwrap();
        crate::store_scraper_metadata(
            &conn,
            b,
            &genre_metadata(vec![entity(2620, "Role Playing Game")]),
        )
        .unwrap();

        // Merging the local row into the real one: the real id wins.
        let survivor = merge_entities(&conn, KIND_GENRE, -3, 2620).unwrap();
        assert_eq!(survivor, 2620);
        assert_eq!(crate::entity_usage(&conn, KIND_GENRE).unwrap().get(&2620), Some(&2));
        assert_eq!(crate::entity_usage(&conn, KIND_GENRE).unwrap().get(&-3), None);
        // The absorbed spelling stays behind as an alias of the
        // survivor: the next match bringing "RPG" resolves to 2620.
        assert!(crate::entity_aliases(&conn, KIND_GENRE, 2620)
            .unwrap()
            .iter()
            .any(|(_, text)| text == "RPG"));
        let resolved = crate::resolve_entities(&conn, KIND_GENRE, &[entity(-3, "RPG")]);
        assert_eq!(resolved[0].id, "2620");
        // An explicit alias outranks the merge trail — the user said
        // so, and the user is who the table serves.
        crate::add_entity_alias(&conn, KIND_GENRE, "Role Playing Game", -3).unwrap();
        let resolved = crate::resolve_entities(&conn, KIND_GENRE, &[entity(2620, "Role Playing Game")]);
        assert_eq!(resolved[0].id, "-3");
        let _ = GENRES;
    }

    #[test]
    fn test_merge_prefers_the_real_id_whichever_way_it_was_asked() {
        // Picking the local row as the target: the real id survives.
        assert_eq!(merge_entities(&setup_db().0, KIND_GENRE, -3, 2620).unwrap(), 2620);
        // Two locals, or two reals: the user's target wins.
        assert_eq!(merge_entities(&setup_db().0, KIND_GENRE, -7, -3).unwrap(), -7);
        assert_eq!(merge_entities(&setup_db().0, KIND_GENRE, 2620, 2406).unwrap(), 2620);
        // Merging a row into itself is refused.
        assert!(merge_entities(&setup_db().0, KIND_GENRE, 2620, 2620).is_err());
    }
}
