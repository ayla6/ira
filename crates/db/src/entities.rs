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
pub(crate) fn spec(kind: &str) -> EntitySpec {
    match kind {
        KIND_GENRE => GENRES,
        KIND_FAMILY => FAMILIES,
        _ => COMPANIES,
    }
}

/// The company-role predicate fragment for the entity index queries:
/// `None` means every company, `Some` narrows to one credit column.
fn role_filter_and(role: Option<&str>) -> String {
    match role {
        Some(role) => format!("AND gc.{role} = 1"),
        None => String::new(),
    }
}

/// Same predicate for queries joining from the lookup side, where the
/// junction alias makes the fragment a `WHERE` instead of an `AND`.
fn role_filter_where(role: Option<&str>) -> String {
    match role {
        Some(role) => format!("WHERE gc.{role} = 1"),
        None => String::new(),
    }
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
    crate::execute(
        conn,
        "INSERT INTO scraper_aliases (kind, alias, target_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(kind, alias) DO UPDATE SET target_id = excluded.target_id",
        params![kind, alias, target_id],
    )?;
    Ok(())
}

pub fn remove_entity_alias(conn: &DbConn, alias_id: i64) -> Result<(), String> {
    crate::execute(conn, "DELETE FROM scraper_aliases WHERE id = ?1", params![alias_id])?;
    Ok(())
}

/// The aliases pointing at one entity, `(rowid, text)`, alphabetical.
pub fn entity_aliases(
    conn: &DbConn,
    kind: &str,
    target_id: i64,
) -> Result<Vec<(i64, String)>, String> {
    crate::query_all(
        conn,
        "SELECT id, alias FROM scraper_aliases
             WHERE kind = ?1 AND target_id = ?2
             ORDER BY alias COLLATE NOCASE",
        params![kind, target_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// Deduplicated non-empty alias spellings to look up — one batch instead
/// of one query per entity.
fn wanted_alias_names(entities: &[ira_models::ScraperEntity]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entity in entities {
        let alias = entity.name.trim();
        if !alias.is_empty() && seen.insert(alias.to_lowercase()) {
            out.push(alias.to_string());
        }
    }
    out
}

/// Lowercased alias text to canonical target id, via one batched query.
/// `None` means the lookup itself failed (caller falls back to input).
fn load_alias_map(
    c: &r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
    spec: EntitySpec,
    kind: &str,
    wanted: &[String],
) -> Option<std::collections::HashMap<String, i64>> {
    let placeholders = wanted.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let mut alias_params: Vec<&dyn rusqlite::types::ToSql> = vec![&spec.kind];
    for name in wanted {
        alias_params.push(name);
    }
    let alias_sql = format!(
        "SELECT alias, target_id FROM scraper_aliases WHERE kind = ?1 AND alias IN ({placeholders}) COLLATE NOCASE"
    );
    let mut stmt = match c.prepare(&alias_sql) {
        Ok(stmt) => stmt,
        Err(e) => {
            eprintln!("resolve_entities: alias lookup failed for kind {kind}: {e}");
            return None;
        }
    };
    let rows = match stmt.query_map(alias_params.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("resolve_entities: alias lookup failed for kind {kind}: {e}");
            return None;
        }
    };
    let mut alias_map: std::collections::HashMap<String, i64> = Default::default();
    for row in rows.filter_map(Result::ok) {
        alias_map.insert(row.0.to_lowercase(), row.1);
    }
    Some(alias_map)
}

/// Canonical target id to current lookup-table name. Missing rows stay
/// missing — the caller keeps the incoming name rather than dropping data.
fn load_name_map(
    c: &r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
    spec: EntitySpec,
    kind: &str,
    targets: &[i64],
) -> std::collections::HashMap<i64, String> {
    let placeholders = targets.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let target_params: Vec<&dyn rusqlite::types::ToSql> =
        targets.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
    let mut name_map: std::collections::HashMap<i64, String> = Default::default();
    match c.prepare(&format!(
        "SELECT id, name FROM {} WHERE id IN ({placeholders})",
        spec.lookup
    )) {
        Ok(mut stmt) => match stmt.query_map(target_params.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) {
            Ok(rows) => {
                for (id, name) in rows.filter_map(Result::ok) {
                    name_map.insert(id, name);
                }
            }
            Err(e) => {
                eprintln!("resolve_entities: name lookup failed for kind {kind}: {e}");
            }
        },
        Err(e) => {
            eprintln!("resolve_entities: name lookup failed for kind {kind}: {e}");
        }
    }
    name_map
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
        eprintln!("resolve_entities: failed to lock db for kind {kind}");
        return entities.to_vec();
    };
    // Batch the alias and name lookups: two queries total no matter how
    // many entities arrive, instead of two per entity.
    let wanted = wanted_alias_names(entities);
    if wanted.is_empty() {
        return entities.to_vec();
    }
    let Some(alias_map) = load_alias_map(&c, spec, kind, &wanted) else {
        return entities.to_vec();
    };
    if alias_map.is_empty() {
        return entities.to_vec();
    }
    let targets: Vec<i64> = {
        let mut seen = std::collections::HashSet::new();
        alias_map.values().filter(|id| seen.insert(**id)).copied().collect()
    };
    let name_map = load_name_map(&c, spec, kind, &targets);
    entities
        .iter()
        .map(|entity| {
            let alias = entity.name.trim();
            let Some(target_id) = alias_map.get(&alias.to_lowercase()).copied() else {
                return entity.clone();
            };
            let name = name_map.get(&target_id).cloned().unwrap_or_else(|| entity.name.clone());
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
    crate::execute(
        conn,
        "INSERT INTO scraper_fetch_log (kind, fetched_at) VALUES (?1, ?2)
         ON CONFLICT(kind) DO UPDATE SET fetched_at = excluded.fetched_at",
        params![kind, at],
    )?;
    Ok(())
}

/// The alphabetically-first entity name of `kind` credited to each
/// game — the group-by index. `role` narrows companies to one credit
/// ("is_developer" / "is_publisher"); other kinds take `None`.
pub fn game_entity_names(
    conn: &DbConn,
    kind: &str,
    role: Option<&str>,
) -> Result<std::collections::HashMap<i64, String>, String> {
    let spec = spec(kind);
    let role_sql = role_filter_and(role);
    let rows: Vec<(i64, String)> = crate::query_all(
        conn,
        &format!(
            "SELECT gc.game_id, MIN(c.name)
             FROM {junction} gc
             JOIN {lookup} c ON c.id = gc.{col}
             WHERE 1 = 1 {role_sql}
             GROUP BY gc.game_id",
            junction = spec.junction,
            lookup = spec.lookup,
            col = spec.column,
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(rows.into_iter().collect())
}

/// Every credited name per game, not just one: the auto-group value
/// rules match when any of a game's genres (or studios, or families)
/// agrees, unlike the group-by category pick which needs a single name.
pub fn game_entity_names_all(
    conn: &DbConn,
    kind: &str,
    role: Option<&str>,
) -> Result<std::collections::HashMap<i64, Vec<String>>, String> {
    let spec = spec(kind);
    let role_sql = role_filter_and(role);
    let rows: Vec<(i64, String)> = crate::query_all(
        conn,
        &format!(
            "SELECT gc.game_id, c.name
             FROM {junction} gc
             JOIN {lookup} c ON c.id = gc.{col}
             WHERE 1 = 1 {role_sql}
             ORDER BY c.name COLLATE NOCASE",
            junction = spec.junction,
            lookup = spec.lookup,
            col = spec.column,
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut map: std::collections::HashMap<i64, Vec<String>> = Default::default();
    for (game_id, name) in rows {
        map.entry(game_id).or_default().push(name);
    }
    Ok(map)
}

/// The distinct names a dimension has on record, for the rule editor's
/// value pickers.
pub fn distinct_entity_names(
    conn: &DbConn,
    kind: &str,
    role: Option<&str>,
) -> Result<Vec<String>, String> {
    let spec = spec(kind);
    let role_sql = role_filter_where(role);
    crate::query_scalar_all(
        conn,
        &format!(
            "SELECT DISTINCT c.name FROM {lookup} c
             JOIN {junction} gc ON gc.{col} = c.id {role_sql}
             ORDER BY c.name COLLATE NOCASE",
            junction = spec.junction,
            lookup = spec.lookup,
            col = spec.column,
        ),
        [],
    )
}

/// A user rename: the new spelling lands and latches the row, so the
/// store's freshen-from-source pass stops overwriting it.
pub fn rename_entity(conn: &DbConn, kind: &str, id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("empty name".to_string());
    }
    crate::execute(
        conn,
        &format!(
            "UPDATE {} SET name = ?1, user_renamed = 1 WHERE id = ?2",
            spec(kind).lookup
        ),
        params![name, id],
    )?;
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
    crate::query_all(
        conn,
        &format!(
            "SELECT id, name FROM {}
             WHERE name LIKE '%' || ?1 || '%' COLLATE NOCASE
             ORDER BY name COLLATE NOCASE",
            spec.lookup
        ),
        params![filter.trim()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// How many games reference each entity of a kind, for the manager's
/// subtitles — the number that makes a duplicate worth merging.
pub fn entity_usage(
    conn: &DbConn,
    kind: &str,
) -> Result<std::collections::HashMap<i64, i64>, String> {
    let spec = spec(kind);
    let rows: Vec<(i64, i64)> = crate::query_all(
        conn,
        &format!(
            "SELECT {col}, COUNT(*) FROM {junction} GROUP BY {col}",
            col = spec.column,
            junction = spec.junction,
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(rows.into_iter().collect())
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
        let game = add_game(&conn, crate::NewGame { kind: GameKind::Steam, trophy_source: TrophySource::Gse, steam_id: "", trophy_id: "", native_id: "", platform_id: "", title: "Game" }).unwrap();
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
            crate::entity_search(&conn, GENRES.lookup, "RPG")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            crate::entity_search(&conn, GENRES.lookup, "Role Playing")
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
            crate::entity_search(&conn, GENRES.lookup, "Adventure")
                .unwrap()
                .len(),
            1
        );
    }
}
