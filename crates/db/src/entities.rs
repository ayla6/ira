//! Management of the scraper entity caches — the companies, genres and
//! families every match harvests. Three user operations live here, on
//! top of the alias rules that keep future matches agreeing with them:
//!
//! - **Rename** fixes a spelling for display. The row latches
//!   `user_renamed`, so the store's freshen-from-source pass — which
//!   otherwise lets the source's spelling win — leaves it alone.
//! - **Aliases** teach the cache that an incoming name means some
//!   canonical entry ("Role Playing Game" is RPG): every store resolves
//!   names through this table first and writes the canonical id, so an
//!   alias declared once fixes every future match and refetch.
//! - **Merge** deduplicates two rows into one, moving every game
//!   reference over. The survivor is the row with a real
//!   ScreenScraper id when exactly one has it — the source owns the id
//!   space — and the absorbed row's name stays behind as an alias of
//!   the survivor, so the next match bringing the old entry collapses
//!   onto it instead of resurrecting the duplicate.

use crate::{err, DbConn};
use rusqlite::params;

/// Which entity cache an alias or operation refers to.
pub const KIND_COMPANY: &str = "company";
pub const KIND_GENRE: &str = "genre";
pub const KIND_FAMILY: &str = "family";

/// The lookup and junction tables backing one entity kind. Companies
/// get the roles junction; genres and families share the plain shape.
#[derive(Clone, Copy)]
pub(crate) struct EntitySpec {
    pub kind: &'static str,
    pub lookup: &'static str,
    pub junction: &'static str,
    pub column: &'static str,
}

pub(crate) const COMPANIES: EntitySpec = EntitySpec {
    kind: KIND_COMPANY,
    lookup: "scraper_companies",
    junction: "scraper_game_companies",
    column: "company_id",
};

pub(crate) const GENRES: EntitySpec = EntitySpec {
    kind: KIND_GENRE,
    lookup: "scraper_genres",
    junction: "scraper_game_genres",
    column: "genre_id",
};

pub(crate) const FAMILIES: EntitySpec = EntitySpec {
    kind: KIND_FAMILY,
    lookup: "scraper_families",
    junction: "scraper_game_families",
    column: "family_id",
};

/// The tables backing a kind constant. Unknown kinds land on companies
/// — the callers pass the constants above, and the match only keeps
/// the stringly kind out of the SQL.
fn spec(kind: &str) -> EntitySpec {
    match kind {
        KIND_GENRE => GENRES,
        KIND_FAMILY => FAMILIES,
        _ => COMPANIES,
    }
}

/// Whether an id belongs to the source's namespace — real
/// ScreenScraper ids are positive; hand-minted locals are negative.
fn is_real_id(id: i64) -> bool {
    id > 0
}

/// Remember `alias` as another name for the entity `target_id`. A
/// repeated alias re-targets, so a mistake is fixed by declaring it
/// again, never by surgery on the table.
pub fn add_entity_alias(
    conn: &DbConn,
    kind: &str,
    alias: &str,
    target_id: i64,
) -> Result<(), String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("empty alias".to_string());
    }
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO scraper_aliases (kind, alias, target_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(kind, alias) DO UPDATE SET target_id = excluded.target_id",
        params![kind, alias, target_id],
    )
    .map_err(err)?;
    Ok(())
}

pub fn remove_entity_alias(conn: &DbConn, alias_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM scraper_aliases WHERE id = ?1",
        params![alias_id],
    )
    .map_err(err)?;
    Ok(())
}

/// The aliases pointing at one entity, `(rowid, text)`, alphabetical.
pub fn entity_aliases(
    conn: &DbConn,
    kind: &str,
    target_id: i64,
) -> Result<Vec<(i64, String)>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(
            "SELECT id, alias FROM scraper_aliases
             WHERE kind = ?1 AND target_id = ?2
             ORDER BY alias COLLATE NOCASE",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![kind, target_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows)
}

/// Resolve entities through the alias table: a name some alias spells
/// becomes its canonical entity, id and all. Unknown names pass
/// through untouched, and a canonical row the lookup has lost leaves
/// the incoming name on the resolved id rather than dropping data.
pub fn resolve_entities(
    conn: &DbConn,
    kind: &str,
    entities: &[ira_models::ScraperEntity],
) -> Vec<ira_models::ScraperEntity> {
    let spec = spec(kind);
    let Ok(c) = crate::lock_db(conn) else {
        return entities.to_vec();
    };
    entities
        .iter()
        .map(|entity| {
            let alias = entity.name.trim();
            if alias.is_empty() {
                return entity.clone();
            }
            let target: Option<i64> = c
                .query_row(
                    "SELECT target_id FROM scraper_aliases
                     WHERE kind = ?1 AND alias = ?2 COLLATE NOCASE",
                    params![spec.kind, alias],
                    |row| row.get(0),
                )
                .ok();
            let Some(target_id) = target else {
                return entity.clone();
            };
            let name: String = c
                .query_row(
                    &format!("SELECT name FROM {} WHERE id = ?1", spec.lookup),
                    params![target_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| entity.name.clone());
            ira_models::ScraperEntity {
                id: target_id.to_string(),
                name,
            }
        })
        .collect()
}

/// Resolve a whole metadata record — companies, genres and families
/// each through their own aliases. Runs at the UI's staging points so
/// the draft shows the canonical form, and inside the store for every
/// path that skips the UI.
pub fn resolve_metadata_aliases(conn: &DbConn, meta: &mut ira_models::ScraperMetadata) {
    let resolve = |kind: &'static str, list: &mut Vec<ira_models::ScraperEntity>| {
        *list = resolve_entities(conn, kind, list);
    };
    resolve(KIND_COMPANY, &mut meta.developers);
    resolve(KIND_COMPANY, &mut meta.publishers);
    resolve(KIND_GENRE, &mut meta.genres);
    resolve(KIND_FAMILY, &mut meta.families);
}

/// Upsert a fetched batch of the source's own rows into one lookup
/// cache — the genre and family tables' search index for the pickers.
/// Honors the rename latch, like every writer.
pub fn warm_entity_cache(
    conn: &DbConn,
    kind: &str,
    entries: &[(i64, String)],
) -> Result<(), String> {
    let spec = spec(kind);
    let mut c = crate::lock_db(conn)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    for (id, name) in entries {
        tx.execute(
            &format!(
                "INSERT INTO {} (id, name) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name
                 WHERE user_renamed = 0",
                spec.lookup
            ),
            params![id, name],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(())
}

/// When a kind's cache last fetched the source's whole table, if
/// ever. Absent means "never" — the cue to fetch it all.
pub fn entity_fetched_at(conn: &DbConn, kind: &str) -> Result<Option<i64>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT fetched_at FROM scraper_fetch_log WHERE kind = ?1",
        params![kind],
    )
}

/// Record a completed whole-table fetch.
pub fn set_entity_fetched(conn: &DbConn, kind: &str, at: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO scraper_fetch_log (kind, fetched_at) VALUES (?1, ?2)
         ON CONFLICT(kind) DO UPDATE SET fetched_at = excluded.fetched_at",
        params![kind, at],
    )
    .map_err(err)?;
    Ok(())
}

/// A user rename: the new spelling lands and latches the row, so the
/// store's freshen-from-source pass stops overwriting it.
pub fn rename_entity(conn: &DbConn, kind: &str, id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("empty name".to_string());
    }
    let c = crate::lock_db(conn)?;
    c.execute(
        &format!(
            "UPDATE {} SET name = ?1, user_renamed = 1 WHERE id = ?2",
            spec(kind).lookup
        ),
        params![name, id],
    )
    .map_err(err)?;
    Ok(())
}

/// Every entity of a kind, `(id, name)`, alphabetical, optionally
/// narrowed by a substring — the manager's list page.
pub fn list_entities(
    conn: &DbConn,
    kind: &str,
    filter: &str,
) -> Result<Vec<(i64, String)>, String> {
    let spec = spec(kind);
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT id, name FROM {}
             WHERE name LIKE '%' || ?1 || '%' COLLATE NOCASE
             ORDER BY name COLLATE NOCASE",
            spec.lookup
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map(params![filter.trim()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows)
}

/// How many games reference each entity of a kind, for the manager's
/// subtitles — the number that makes a duplicate worth merging.
pub fn entity_usage(
    conn: &DbConn,
    kind: &str,
) -> Result<std::collections::HashMap<i64, i64>, String> {
    let spec = spec(kind);
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT {col}, COUNT(*) FROM {junction} GROUP BY {col}",
            col = spec.column,
            junction = spec.junction,
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows.into_iter().collect())
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

    // A game holding both rows keeps one reference with the union of
    // the flags; companies only — genres and families have no roles.
    if spec.kind == KIND_COMPANY {
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
                junction = spec.junction,
                col = spec.column,
            ),
            params![survivor, absorbed],
        )
        .map_err(err)?;
    }
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO {junction} (game_id, {col})
             SELECT game_id, ?1 FROM {junction} WHERE {col} = ?2",
            junction = spec.junction,
            col = spec.column,
        ),
        params![survivor, absorbed],
    )
    .map_err(err)?;
    tx.execute(
        &format!(
            "DELETE FROM {junction} WHERE {col} = ?1",
            junction = spec.junction,
            col = spec.column,
        ),
        params![absorbed],
    )
    .map_err(err)?;

    // Aliases follow the references, and the absorbed spelling itself
    // becomes the survivor's alias — the dedupe's whole point.
    tx.execute(
        "UPDATE scraper_aliases SET target_id = ?1
         WHERE kind = ?2 AND target_id = ?3",
        params![survivor, spec.kind, absorbed],
    )
    .map_err(err)?;
    let absorbed_name: Option<String> = tx
        .query_row(
            &format!("SELECT name FROM {} WHERE id = ?1", spec.lookup),
            params![absorbed],
            |row| row.get(0),
        )
        .ok();
    if let Some(name) = absorbed_name.filter(|n| !n.trim().is_empty()) {
        tx.execute(
            "INSERT INTO scraper_aliases (kind, alias, target_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind, alias) DO UPDATE SET target_id = excluded.target_id",
            params![spec.kind, name.trim(), survivor],
        )
        .map_err(err)?;
    }

    tx.execute(
        &format!("DELETE FROM {} WHERE id = ?1", spec.lookup),
        params![absorbed],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(survivor)
}

#[cfg(test)]
mod tests {
    use super::super::add_game;
    use super::super::init_db;
    use super::*;
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

    /// Seed one lookup row directly, clearing any rename latch — the
    /// tests below exercise the store's real behavior on top of these.
    fn put_entity(conn: &DbConn, spec: EntitySpec, id: i64, name: &str) {
        conn.get()
            .unwrap()
            .execute(
                &format!(
                    "INSERT INTO {} (id, name) VALUES (?1, ?2)
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, user_renamed = 0",
                    spec.lookup
                ),
                params![id, name],
            )
            .unwrap();
    }

    fn genre_metadata(genres: Vec<ScraperEntity>) -> ScraperMetadata {
        ScraperMetadata {
            genres,
            ..Default::default()
        }
    }

    #[test]
    fn test_resolve_entities_substitutes_alias_names() {
        let (conn, _tmp) = setup_db();
        add_entity_alias(&conn, KIND_GENRE, "Role Playing Game", -3).unwrap();
        // The canonical row's own spelling wins on resolution.
        put_entity(&conn, GENRES, -3, "RPG");

        let resolved = resolve_entities(
            &conn,
            KIND_GENRE,
            &[entity(2620, "Role Playing Game"), entity(2406, "Adventure")],
        );
        assert_eq!(resolved[0], entity(-3, "RPG"));
        // Unknown names pass through untouched.
        assert_eq!(resolved[1], entity(2406, "Adventure"));
        // A canonical row the lookup has lost keeps the incoming name
        // rather than dropping the reference.
        add_entity_alias(&conn, KIND_GENRE, "CRPG", -9).unwrap();
        let resolved = resolve_entities(&conn, KIND_GENRE, &[entity(1, "CRPG")]);
        assert_eq!(resolved[0], entity(-9, "CRPG"));
    }

    #[test]
    fn test_rename_latches_against_store_freshening() {
        let (conn, _tmp) = setup_db();
        let game = add_game(&conn, GameKind::Steam, TrophySource::Gse, "", "", "", "Game").unwrap();
        put_entity(&conn, GENRES, 2620, "Role Playing Game");
        rename_entity(&conn, KIND_GENRE, 2620, "RPG").unwrap();
        // The source's next spelling loses to the user's rename.
        crate::store_scraper_metadata(
            &conn,
            game,
            &genre_metadata(vec![entity(2620, "Role Playing Game")]),
        )
        .unwrap();
        assert_eq!(
            crate::metadata::entity_search(&conn, GENRES.lookup, "RPG")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            crate::metadata::entity_search(&conn, GENRES.lookup, "Role Playing")
                .unwrap()
                .len(),
            0
        );
        // An untouched row keeps freshening from the source.
        put_entity(&conn, GENRES, 2406, "Adventures");
        crate::store_scraper_metadata(
            &conn,
            game,
            &genre_metadata(vec![entity(2406, "Adventure")]),
        )
        .unwrap();
        assert_eq!(
            crate::metadata::entity_search(&conn, GENRES.lookup, "Adventure")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_merge_moves_references_and_leaves_an_alias() {
        let (conn, _tmp) = setup_db();
        let a = add_game(&conn, GameKind::Steam, TrophySource::Gse, "", "", "", "One").unwrap();
        let b = add_game(&conn, GameKind::Steam, TrophySource::Gse, "", "", "", "Two").unwrap();
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
        assert_eq!(entity_usage(&conn, KIND_GENRE).unwrap().get(&2620), Some(&2));
        assert_eq!(entity_usage(&conn, KIND_GENRE).unwrap().get(&-3), None);
        // The absorbed spelling stays behind as an alias of the
        // survivor: the next match bringing "RPG" resolves to 2620.
        assert!(entity_aliases(&conn, KIND_GENRE, 2620)
            .unwrap()
            .iter()
            .any(|(_, text)| text == "RPG"));
        let resolved = resolve_entities(&conn, KIND_GENRE, &[entity(-3, "RPG")]);
        assert_eq!(resolved[0].id, "2620");
        // An explicit alias outranks the merge trail — the user said
        // so, and the user is who the table serves.
        add_entity_alias(&conn, KIND_GENRE, "Role Playing Game", -3).unwrap();
        let resolved = resolve_entities(&conn, KIND_GENRE, &[entity(2620, "Role Playing Game")]);
        assert_eq!(resolved[0].id, "-3");
    }

    #[test]
    fn test_merge_prefers_the_real_id_whichever_way_it_was_asked() {
        let (conn, _tmp) = setup_db();
        // Picking the local row as the target: the real id survives.
        assert_eq!(merge_entities(&conn, KIND_GENRE, -3, 2620).unwrap(), 2620);
        // Two locals, or two reals: the user's target wins.
        assert_eq!(merge_entities(&conn, KIND_GENRE, -7, -3).unwrap(), -7);
        assert_eq!(merge_entities(&conn, KIND_GENRE, 2620, 2406).unwrap(), 2620);
        // Merging a row into itself is refused.
        assert!(merge_entities(&conn, KIND_GENRE, 2620, 2620).is_err());
    }
}
