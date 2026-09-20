use crate::{err, DbConn};
use rusqlite::params;

pub fn store_game_metadata(
    conn: &DbConn,
    game_id: i64,
    release_date: &str,
    release_timestamp: i64,
    metacritic_score: i64,
    steam_review_score: i64,
    steam_review_count: i64,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET
            release_date = ?1,
            release_timestamp = ?2,
            metacritic_score = ?3,
            steam_review_score = ?4,
            steam_review_count = ?5
         WHERE id = ?6",
        params![
            release_date,
            release_timestamp,
            metacritic_score,
            steam_review_score,
            steam_review_count,
            game_id
        ],
    )
    .map_err(err)?;
    Ok(())
}

/// Upsert one ScreenScraper entity table: ids are stable, and a name
/// the source has since corrected should win — unless the user renamed
/// the row, which latches it against freshening.
fn store_entities(
    conn: &rusqlite::Connection,
    table: &str,
    entities: &[(i64, String)],
) -> Result<(), String> {
    for (id, name) in entities {
        conn.execute(
            &format!(
                "INSERT INTO {table} (id, name) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name
                 WHERE user_renamed = 0"
            ),
            params![id, name],
        )
        .map_err(err)?;
    }
    Ok(())
}

/// Store what a ScreenScraper match found. The entry keeps only the
/// references — company/genre/classification ids — while the names land
/// in the `scraper_*` lookup tables, in the same transaction: every
/// response carries names inline with ids, so the definitions are never
/// missing. `release_timestamp` is the parsed `YYYY-MM-DD` (or bare
/// `YYYY`) date as a Unix timestamp, 0 when undated. Empty pieces never
/// overwrite what a previous scrape or Steam enrichment stored.
pub fn store_scraper_metadata(
    conn: &DbConn,
    game_id: i64,
    metadata: &ira_models::ScraperMetadata,
) -> Result<(), String> {
    // Aliases resolve first: a name the alias table knows stores as its
    // canonical entity, so one declared alias fixes every future match
    // and refetch at the only choke point every writer shares.
    let mut metadata = metadata.clone();
    super::entities::resolve_metadata_aliases(conn, &mut metadata);
    let metadata = &metadata;

    // One row per company with role flags: a studio credited as both
    // developer and publisher is a single company, not two.
    let mut companies: std::collections::BTreeMap<i64, (String, bool, bool)> = Default::default();
    for entity in &metadata.developers {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies
                .entry(id)
                .or_insert((entity.name.trim().to_string(), false, false))
                .1 = true;
        }
    }
    for entity in &metadata.publishers {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies
                .entry(id)
                .or_insert((entity.name.trim().to_string(), false, false))
                .2 = true;
        }
    }
    let genres: Vec<(i64, String)> = metadata
        .genres
        .iter()
        .filter_map(|genre| genre.id.parse::<i64>().ok().map(|id| (id, genre.name.clone())))
        .collect();
    let families: Vec<(i64, String)> = metadata
        .families
        .iter()
        .filter_map(|famille| famille.id.parse::<i64>().ok().map(|id| (id, famille.name.clone())))
        .collect();
    let synopses_json = serde_json::to_string(&metadata.synopses).unwrap_or_default();
    let dates_json = serde_json::to_string(
        &metadata
            .release_dates
            .iter()
            .cloned()
            .collect::<std::collections::HashMap<String, String>>(),
    )
    .unwrap_or_default();

    // Immediate: this store runs on enricher threads while the matcher's
    // worker writes too, and a deferred transaction that read first only
    // finds out at write time — as an immediate "database is locked".
    let mut c = crate::lock_db(conn)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let company_rows: Vec<(i64, String)> = companies
        .iter()
        .map(|(id, (name, _, _))| (*id, name.clone()))
        .collect();
    store_entities(&tx, "scraper_companies", &company_rows)?;
    tx.execute(
        "DELETE FROM scraper_game_companies WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, (_, is_developer, is_publisher)) in &companies {
        tx.execute(
            "INSERT INTO scraper_game_companies
                (game_id, company_id, is_developer, is_publisher)
             VALUES (?1, ?2, ?3, ?4)",
            params![game_id, id, *is_developer as i64, *is_publisher as i64],
        )
        .map_err(err)?;
    }
    tx.execute(
        "DELETE FROM scraper_game_genres WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, name) in &genres {
        store_entities(&tx, "scraper_genres", &[(*id, name.clone())])?;
        tx.execute(
            "INSERT OR IGNORE INTO scraper_game_genres (game_id, genre_id) VALUES (?1, ?2)",
            params![game_id, id],
        )
        .map_err(err)?;
    }
    tx.execute(
        "DELETE FROM scraper_game_families WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, name) in &families {
        store_entities(&tx, "scraper_families", &[(*id, name.clone())])?;
        tx.execute(
            "INSERT OR IGNORE INTO scraper_game_families (game_id, family_id) VALUES (?1, ?2)",
            params![game_id, id],
        )
        .map_err(err)?;
    }
    tx.execute(
        "DELETE FROM scraper_game_classifications WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for class in &metadata.classifications {
        tx.execute(
            "INSERT OR IGNORE INTO scraper_game_classifications
                (game_id, kind, value) VALUES (?1, ?2, ?3)",
            params![
                game_id,
                ira_models::ratings::canonical_kind(&class.kind),
                class.value
            ],
        )
        .map_err(err)?;
    }
    reconcile_local_companies(&tx, &companies)?;
    tx.execute(
        "UPDATE games SET
            screenscraper_id = ?1,
            release_date = CASE WHEN ?2 != '' THEN ?2 ELSE release_date END,
            release_timestamp = CASE WHEN ?3 != 0 THEN ?3 ELSE release_timestamp END,
            release_dates = CASE WHEN ?4 != '' THEN ?4 ELSE release_dates END,
            players = CASE WHEN ?5 != '' THEN ?5 ELSE players END,
            screenscraper_rating = CASE WHEN ?6 > 0 THEN ?6 ELSE screenscraper_rating END,
            synopsis = CASE WHEN ?7 != '' THEN ?7 ELSE synopsis END
         WHERE id = ?8",
        params![
            metadata.ss_id,
            metadata.release_date,
            metadata.release_timestamp,
            dates_json,
            metadata.players,
            metadata.rating,
            synopses_json,
            game_id
        ],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(())
}

/// The company entity for a Steam-side name: a cached ScreenScraper
/// company when one's identifying tokens match, else a freshly allocated
/// local company under a negative id — the namespace ScreenScraper's
/// positive ids can never reach. `None` only for names with no
/// identifying tokens at all.
pub fn steam_company_entity(conn: &DbConn, name: &str) -> Option<ira_models::ScraperEntity> {
    let tokens = ira_models::company_tokens(name);
    if tokens.is_empty() {
        return None;
    }
    // Identity scan over the whole table — a LIKE prefilter cannot work
    // here because the query spelling may be longer or shorter than the
    // stored one ("Team Salvato Inc." vs "Team Salvato"). Real
    // ScreenScraper ids beat local ones, and the oldest local wins among
    // spellings.
    {
        let c = crate::lock_db(conn).ok()?;
        let Ok(mut stmt) = c.prepare(
            "SELECT id, name FROM scraper_companies ORDER BY (id < 0), id DESC",
        ) else {
            return None;
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) else {
            return None;
        };
        for pair in rows.filter_map(Result::ok) {
            if ira_models::company_tokens(&pair.1) == tokens {
                return Some(ira_models::ScraperEntity {
                    id: pair.0.to_string(),
                    name: pair.1,
                });
            }
        }
    }
    let c = crate::lock_db(conn).ok()?;
    let min: i64 = c
        .query_row(
            "SELECT COALESCE(MIN(id), 0) FROM scraper_companies WHERE id < 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let id = min - 1;
    let name = name.trim().to_string();
    // A concurrent allocation of the same company loses the insert and
    // the re-scan below hands back the winner, whatever spelling it kept.
    let _ = c.execute(
        "INSERT INTO scraper_companies (id, name) VALUES (?1, ?2)",
        rusqlite::params![id, name],
    );
    // A concurrent allocation of the same company loses the insert and
    // the re-scan below hands back the winner, whatever spelling it kept.
    let locals: Vec<(i64, String)> = {
        let Ok(mut stmt) =
            c.prepare("SELECT id, name FROM scraper_companies WHERE id < 0 ORDER BY id DESC")
        else {
            return None;
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) else {
            return None;
        };
        rows.filter_map(Result::ok).collect()
    };
    if let Some((local_id, local_name)) = locals
        .iter()
        .find(|(_, n)| ira_models::company_tokens(n) == tokens)
    {
        return Some(ira_models::ScraperEntity {
            id: local_id.to_string(),
            name: local_name.clone(),
        });
    }
    Some(ira_models::ScraperEntity {
        id: id.to_string(),
        name,
    })
}

/// Local companies graduate the day a real ScreenScraper match carries
/// the same studio: every game referencing the negative id swaps to the
/// proper one and the local row goes away. Runs as a side effect of
/// storing match metadata, so it never surfaces as an event.
fn reconcile_local_companies(
    tx: &rusqlite::Transaction,
    companies: &std::collections::BTreeMap<i64, (String, bool, bool)>,
) -> Result<(), String> {
    let locals: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, name FROM scraper_companies WHERE id < 0")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    if locals.is_empty() {
        return Ok(());
    }
    for (id, (name, _, _)) in companies {
        if *id < 0 {
            continue;
        }
        let tokens = ira_models::company_tokens(name);
        if tokens.is_empty() {
            continue;
        }
        for (local_id, local_name) in &locals {
            if ira_models::company_tokens(local_name) != tokens {
                continue;
            }
            // Games whose real row already exists just adopt the local
            // row's flags; the rest move over whole.
            tx.execute(
                "UPDATE scraper_game_companies AS dst
                 SET is_developer = MAX(dst.is_developer,
                     COALESCE((SELECT src.is_developer FROM scraper_game_companies src
                               WHERE src.game_id = dst.game_id AND src.company_id = ?1), 0)),
                     is_publisher = MAX(dst.is_publisher,
                     COALESCE((SELECT src.is_publisher FROM scraper_game_companies src
                               WHERE src.game_id = dst.game_id AND src.company_id = ?1), 0))
                 WHERE company_id = ?2
                   AND EXISTS (SELECT 1 FROM scraper_game_companies src
                               WHERE src.game_id = dst.game_id AND src.company_id = ?1)",
                rusqlite::params![id, local_id],
            )
            .map_err(err)?;
            tx.execute(
                "UPDATE OR REPLACE scraper_game_companies SET company_id = ?1
                 WHERE company_id = ?2",
                rusqlite::params![id, local_id],
            )
            .map_err(err)?;
            tx.execute(
                "DELETE FROM scraper_companies WHERE id = ?1",
                rusqlite::params![local_id],
            )
            .map_err(err)?;
        }
    }
    Ok(())
}

/// Merge a fresh ScreenScraper answer into a stored record, filling only
/// gaps — a refetch never overwrites what's already there, and a match's
/// id stays its own. Returns whether anything was stored.
pub fn merge_missing_scraper_metadata(
    conn: &DbConn,
    game_id: i64,
    fresh: &ira_models::ScraperMetadata,
) -> Result<bool, String> {
    let mut meta = scraper_metadata_for_game(conn, game_id)?.unwrap_or_default();
    if !meta.fill_gaps(fresh) {
        return Ok(false);
    }
    store_scraper_metadata(conn, game_id, &meta)?;
    Ok(true)
}

/// Remember that a game's ScreenScraper search came up empty, so the mass
/// matcher stops re-asking for a ROM the source does not know. Manual
/// picks and later matches clear it again.
pub fn tombstone_scraper_miss(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO scraper_misses (game_id, checked_at) VALUES (?1, ?2)
         ON CONFLICT(game_id) DO UPDATE SET checked_at = excluded.checked_at",
        params![game_id, chrono::Utc::now().timestamp()],
    )
    .map_err(err)?;
    Ok(())
}

/// The games whose ScreenScraper search already came up empty.
pub fn scraper_missed_ids(conn: &DbConn) -> Result<Vec<i64>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare("SELECT game_id FROM scraper_misses").map_err(err)?;
    let ids = stmt
        .query_map([], |row| row.get(0))
        .map_err(err)?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(err)?;
    Ok(ids)
}

/// Forget a game's miss — something matched it after all.
pub fn clear_scraper_miss(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM scraper_misses WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    Ok(())
}

/// The company name a ScreenScraper id refers to, per the lookup table.
pub fn scraper_company_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_companies WHERE id = ?1",
        params![id],
    )
}

/// What a ScreenScraper match stored for one game, with every id expanded
/// back through the lookup tables. `None` when the game was never matched.
pub fn scraper_metadata_for_game(
    conn: &DbConn,
    game_id: i64,
) -> Result<Option<ira_models::ScraperMetadata>, String> {
    let row = crate::query_optional(
        conn,
        "SELECT screenscraper_id, release_date, release_timestamp, release_dates,
                players, screenscraper_rating, synopsis
         FROM games WHERE id = ?1",
        params![game_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, String>(6)?,
            ))
        },
    )?;
    let Some((
        ss_id,
        release_date,
        release_timestamp,
        release_dates,
        players,
        rating,
        synopsis,
    )) = row
    else {
        return Ok(None);
    };
    let dates: std::collections::HashMap<String, String> = if release_dates.is_empty() {
        Default::default()
    } else {
        serde_json::from_str(&release_dates).unwrap_or_default()
    };
    let mut release_dates: Vec<(String, String)> = dates.into_iter().collect();
    release_dates.sort();
    let synopses: Vec<(String, String)> = if synopsis.is_empty() {
        Default::default()
    } else {
        serde_json::from_str(&synopsis).unwrap_or_default()
    };
    Ok(Some(ira_models::ScraperMetadata {
        ss_id,
        release_date,
        release_timestamp,
        release_dates,
        developers: game_companies(conn, game_id, true)?,
        publishers: game_companies(conn, game_id, false)?,
        genres: game_genres(conn, game_id)?,
        families: game_families(conn, game_id)?,
        players,
        rating,
        classifications: game_classifications(conn, game_id)?,
        synopses,
    }))
}

/// Runs an `id, name` lookup-table query and maps each row to a
/// `ScraperEntity` — the shared shape of the company/genre/family reads
/// and the picker searches.
fn query_entities(
    conn: &DbConn,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(sql).map_err(err)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(ira_models::ScraperEntity {
                id: row.get::<_, i64>(0)?.to_string(),
                name: row.get(1)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows)
}

/// The companies credited to a game through one junction row per
/// company — a studio that is both developer and publisher surfaces in
/// both lists from the same row.
fn game_companies(
    conn: &DbConn,
    game_id: i64,
    developer: bool,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    let flag = if developer { "is_developer" } else { "is_publisher" };
    query_entities(
        conn,
        &format!(
            "SELECT c.id, c.name FROM scraper_game_companies gc
             JOIN scraper_companies c ON c.id = gc.company_id
             WHERE gc.game_id = ?1 AND gc.{flag} = 1
             ORDER BY c.name"
        ),
        params![game_id],
    )
}

/// The genres of a game, stored order first.
fn game_genres(conn: &DbConn, game_id: i64) -> Result<Vec<ira_models::ScraperEntity>, String> {
    query_entities(
        conn,
        "SELECT g.id, g.name FROM scraper_game_genres gg
         JOIN scraper_genres g ON g.id = gg.genre_id
         WHERE gg.game_id = ?1
         ORDER BY gg.rowid",
        params![game_id],
    )
}

/// The families (series) of a game, stored order first.
fn game_families(conn: &DbConn, game_id: i64) -> Result<Vec<ira_models::ScraperEntity>, String> {
    query_entities(
        conn,
        "SELECT f.id, f.name FROM scraper_game_families gf
         JOIN scraper_families f ON f.id = gf.family_id
         WHERE gf.game_id = ?1
         ORDER BY gf.rowid",
        params![game_id],
    )
}

/// The age-rating boards of a game, board name alphabetical.
fn game_classifications(
    conn: &DbConn,
    game_id: i64,
) -> Result<Vec<ira_models::ScraperClassification>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(
            "SELECT kind, value FROM scraper_game_classifications
             WHERE game_id = ?1
             ORDER BY kind",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![game_id], |row| {
            Ok(ira_models::ScraperClassification {
                kind: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|class| ira_models::ScraperClassification {
            kind: ira_models::ratings::canonical_kind(&class.kind),
            value: class.value,
        })
        .collect())
}

pub fn scraper_companies_search(
    conn: &DbConn,
    filter: &str,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    entity_search(conn, "scraper_companies", filter)
}

/// Genres whose name contains the filter, for the picker's search.
pub fn search_genres(conn: &DbConn, filter: &str) -> Result<Vec<ira_models::ScraperEntity>, String> {
    entity_search(conn, "scraper_genres", filter)
}

/// Families (series) whose name contains the filter, for the picker's
/// search. The cache fills as games get matched.
pub fn search_families(
    conn: &DbConn,
    filter: &str,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    entity_search(conn, "scraper_families", filter)
}

/// A LIKE search over one of the scraper lookup tables, for the pickers.
pub(crate) fn entity_search(
    conn: &DbConn,
    table: &str,
    filter: &str,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    query_entities(
        conn,
        &format!(
            "SELECT id, name FROM {table}
             WHERE name LIKE '%' || ?1 || '%' COLLATE NOCASE
             ORDER BY name LIMIT 60"
        ),
        params![filter.trim()],
    )
}

/// The genre entity for a hand-typed name: a cached ScreenScraper genre
/// when one's tokens match, else a freshly allocated local genre under a
/// negative id. `None` only for names with no identifying tokens.
pub fn local_genre_entity(conn: &DbConn, name: &str) -> Option<ira_models::ScraperEntity> {
    local_entity(conn, "scraper_genres", name)
}

/// The family entity for a hand-typed series name — same rules as the
/// genre mint.
pub fn local_family_entity(conn: &DbConn, name: &str) -> Option<ira_models::ScraperEntity> {
    local_entity(conn, "scraper_families", name)
}

/// The shared mint behind the genre and family pickers: the cache's own
/// row when one's tokens match, else a fresh local row under a negative
/// id — the namespace the source's positive ids can never reach.
/// `None` only for names with no identifying tokens.
fn local_entity(conn: &DbConn, table: &str, name: &str) -> Option<ira_models::ScraperEntity> {
    let tokens = ira_models::company_tokens(name);
    if tokens.is_empty() {
        return None;
    }
    if let Ok(found) = entity_search(conn, table, name) {
        if let Some(entity) = found.iter().find(|e| ira_models::company_tokens(&e.name) == tokens)
        {
            return Some(entity.clone());
        }
    }
    let c = crate::lock_db(conn).ok()?;
    let min: i64 = c
        .query_row(
            &format!("SELECT COALESCE(MIN(id), 0) FROM {table} WHERE id < 0"),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let id = min - 1;
    let name = name.trim().to_string();
    let _ = c.execute(
        &format!("INSERT INTO {table} (id, name) VALUES (?1, ?2)"),
        rusqlite::params![id, name],
    );
    Some(ira_models::ScraperEntity {
        id: id.to_string(),
        name,
    })
}

/// The genre name a ScreenScraper id refers to, per the lookup table.
pub fn scraper_genre_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_genres WHERE id = ?1",
        params![id],
    )
}

/// Parse a ScreenScraper release date — `YYYY-MM-DD` or bare `YYYY` —
/// into a Unix timestamp (0 = unparsable/absent).
pub fn scraper_release_timestamp(date: &str) -> i64 {
    let date = date.trim();
    if let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        return day.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    }
    if let Ok(day) = chrono::NaiveDate::parse_from_str(&format!("{date}-01-01"), "%Y-%m-%d") {
        return day.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::super::add_game;
    use super::super::find_by_db_id;
    use super::super::init_db;
    use super::*;
    use ira_models::{GameEntry, GameKind, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = init_db(&db_path.to_string_lossy());
        (conn, tmp)
    }

    #[test]
    fn test_store_scraper_metadata_persists_every_field() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            "Dragon Quest I & II",
        )
        .unwrap();

        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "2124".into(),
                release_date: "1993-12-18".into(),
                release_timestamp: scraper_release_timestamp("1993-12-18"),
                release_dates: vec![
                    ("jp".into(), "1993-12-18".into()),
                    ("us".into(), "1993-12-18".into()),
                ],
                developers: vec![ira_models::ScraperEntity {
                    id: "2911".into(),
                    name: "Chunsoft".into(),
                }],
                publishers: vec![ira_models::ScraperEntity {
                    id: "1299".into(),
                    name: "Enix".into(),
                }],
                genres: vec![ira_models::ScraperEntity {
                    id: "2620".into(),
                    name: "Role Playing Game".into(),
                }],
                families: vec![ira_models::ScraperEntity {
                    id: "732".into(),
                    name: "Dragon Quest".into(),
                }],
                players: "1-4".into(),
                rating: 18.0,
                classifications: vec![ira_models::ScraperClassification {
                    kind: "PEGI".into(),
                    value: "PEGI:3".into(),
                }],
                synopses: vec![("en".into(), "The first two quests.".into())],
            },
        )
        .unwrap();

        let entry: GameEntry = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(entry.screenscraper_id, "2124");
        assert_eq!(entry.release_date, "1993-12-18");
        assert!(entry.release_timestamp > 0);
        // The entry keeps only the references; the rows live in the
        // junction tables, the names in the lookup tables.
        let roles: Vec<(i64, i64, i64)> = {
            let pooled = conn.get().unwrap();
            let mut stmt = pooled
                .prepare(
                    "SELECT company_id, is_developer, is_publisher
                     FROM scraper_game_companies WHERE game_id = ?1",
                )
                .unwrap();
            stmt.query_map(params![id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect()
        };
        assert_eq!(roles, vec![(1299, 0, 1), (2911, 1, 0)]);
        assert_eq!(entry.players, "1-4");
        assert_eq!(entry.screenscraper_rating, 18.0);
        assert_eq!(scraper_company_name(&conn, 2911).unwrap().as_deref(), Some("Chunsoft"));
        assert_eq!(scraper_company_name(&conn, 1299).unwrap().as_deref(), Some("Enix"));
        assert_eq!(
            scraper_genre_name(&conn, 2620).unwrap().as_deref(),
            Some("Role Playing Game")
        );
        // Unknown ids resolve to nothing.
        assert_eq!(scraper_company_name(&conn, 1).unwrap(), None);
        let metadata = scraper_metadata_for_game(&conn, id).unwrap().unwrap();
        assert_eq!(
            metadata.families,
            vec![ira_models::ScraperEntity {
                id: "732".to_string(),
                name: "Dragon Quest".to_string()
            }]
        );
        assert_eq!(search_families(&conn, "quest").unwrap().len(), 1);
        let synopses: Vec<(String, String)> = serde_json::from_str(&entry.synopsis).unwrap();
        assert_eq!(synopses, vec![("en".to_string(), "The first two quests.".to_string())]);
        let dates: std::collections::HashMap<String, String> =
            serde_json::from_str(&entry.release_dates).unwrap();
        assert_eq!(dates.get("us").map(String::as_str), Some("1993-12-18"));
    }

    #[test]
    fn test_scraper_miss_tombstone_round_trip() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Retro,
            TrophySource::Empty,
            "",
            "",
            "",
            "Obscure Rom",
        )
        .unwrap();

        assert!(scraper_missed_ids(&conn).unwrap().is_empty());
        tombstone_scraper_miss(&conn, id).unwrap();
        assert_eq!(scraper_missed_ids(&conn).unwrap(), vec![id]);
        // Re-missing the same game stays one row, refreshed.
        tombstone_scraper_miss(&conn, id).unwrap();
        assert_eq!(scraper_missed_ids(&conn).unwrap(), vec![id]);
        // A later match clears it and the game re-enters the pool.
        clear_scraper_miss(&conn, id).unwrap();
        assert!(scraper_missed_ids(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_scraper_metadata_for_game_expands_ids() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Retro,
            TrophySource::Empty,
            "",
            "",
            "",
            "Round Trip",
        )
        .unwrap();

        // Never matched: an empty record the editors can fill by hand.
        let empty = scraper_metadata_for_game(&conn, id).unwrap().unwrap();
        assert!(empty.ss_id.is_empty());
        assert!(empty.developers.is_empty());

        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "2124".into(),
                release_date: "1993-12-18".into(),
                release_timestamp: 1,
                developers: vec![
                    ira_models::ScraperEntity { id: "2911".into(), name: "Chunsoft".into() },
                    ira_models::ScraperEntity { id: "2912".into(), name: "Nintendo".into() },
                ],
                genres: vec![ira_models::ScraperEntity {
                    id: "2620".into(),
                    name: "Role Playing Game".into(),
                }],
                players: "1-4".into(),
                synopses: vec![("en".into(), "Two quests.".into())],
                ..Default::default()
            },
        )
        .unwrap();

        let metadata = scraper_metadata_for_game(&conn, id).unwrap().unwrap();
        assert_eq!(metadata.ss_id, "2124");
        assert_eq!(
            metadata
                .developers
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Chunsoft", "Nintendo"]
        );
        assert_eq!(metadata.genres[0].name, "Role Playing Game");
        assert_eq!(metadata.players, "1-4");
        assert_eq!(
            metadata.synopses,
            vec![("en".to_string(), "Two quests.".to_string())]
        );
        // The lookup names survive the round trip through the id columns.
        assert_eq!(
            scraper_companies_search(&conn, "chun").unwrap().len(),
            1
        );
    }

    #[test]
    fn test_store_scraper_metadata_refreshes_entity_names() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            "Matched",
        )
        .unwrap();
        let company = || ira_models::ScraperEntity {
            id: "2911".into(),
            name: "Chunsoft".into(),
        };
        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "1".into(),
                developers: vec![company()],
                ..Default::default()
            },
        )
        .unwrap();
        // The source fixed a typo: the newer name wins.
        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "1".into(),
                developers: vec![ira_models::ScraperEntity {
                    id: "2911".into(),
                    name: "ChunSoft".into(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(scraper_company_name(&conn, 2911).unwrap().as_deref(), Some("ChunSoft"));
    }

    #[test]
    fn test_store_scraper_metadata_keeps_existing_release_dates() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "1",
            "",
            "",
            "Dated",
        )
        .unwrap();
        store_game_metadata(&conn, id, "15 Sep, 2014", 1410000000, 90, 8, 1000).unwrap();

        // An undated, rating-less ScreenScraper hit must not erase what
        // Steam enrichment already stored.
        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "9".into(),
                developers: vec![ira_models::ScraperEntity {
                    id: "1".into(),
                    name: "id".into(),
                }],
                publishers: vec![ira_models::ScraperEntity {
                    id: "2".into(),
                    name: "pub".into(),
                }],
                genres: vec![ira_models::ScraperEntity {
                    id: "3".into(),
                    name: "g".into(),
                }],
                players: "1".into(),
                synopses: vec![("en".into(), "s".into())],
                ..Default::default()
            },
        )
        .unwrap();
        let entry = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(entry.release_date, "15 Sep, 2014");
        assert_eq!(entry.release_timestamp, 1410000000);
        assert_eq!(entry.screenscraper_rating, -1.0);
        assert_eq!(
            scraper_company_name(&conn, 1).unwrap().as_deref(),
            Some("id")
        );

        // A dated hit replaces it.
        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "9".into(),
                release_date: "1993-12-18".into(),
                release_timestamp: scraper_release_timestamp("1993-12-18"),
                rating: 15.0,
                ..Default::default()
            },
        )
        .unwrap();
        let entry = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(entry.release_date, "1993-12-18");
        assert!(entry.release_timestamp != 1410000000);
        assert_eq!(entry.screenscraper_rating, 15.0);
    }

    #[test]
    fn test_scraper_release_timestamp_parses_both_shapes() {        let full = scraper_release_timestamp("1993-12-18");
        assert_eq!(
            chrono::DateTime::from_timestamp(full, 0)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "1993-12-18"
        );
        let year_only = scraper_release_timestamp("1993");
        assert_eq!(
            chrono::DateTime::from_timestamp(year_only, 0)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "1993-01-01"
        );
        assert_eq!(scraper_release_timestamp(""), 0);
        assert_eq!(scraper_release_timestamp("TBA"), 0);
    }


    #[test]
    fn test_steam_company_entity_allocates_and_reuses_locals() {
        let (conn, _tmp) = setup_db();
        // First sighting allocates a negative id under the store's
        // spelling; the second spelling of the same studio reuses it.
        let first = steam_company_entity(&conn, "Whoever Studios").unwrap();
        assert!(first.id.parse::<i64>().unwrap() < 0);
        assert_eq!(first.name, "Whoever Studios");
        let second = steam_company_entity(&conn, "Whoever Studios LLC").unwrap();
        assert_eq!(second.id, first.id);
        // The cache's real ids win over locals.
        conn.get()
            .unwrap()
            .execute(
                "INSERT INTO scraper_companies (id, name) VALUES (54774, 'Team Salvato')",
                [],
            )
            .unwrap();
        let salvato = steam_company_entity(&conn, "Team Salvato Inc.").unwrap();
        assert_eq!(salvato.id, "54774");
        // A filler-only name agrees with nothing.
        assert!(steam_company_entity(&conn, "LLC").is_none());
    }

    #[test]
    fn test_board_aliases_collapse_into_one_row() {
        let (conn, _tmp) = setup_db();
        let game_id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            "Twice Rated",
        )
        .unwrap();
        // The same board arriving under two spellings — ScreenScraper's
        // and Steam's — is one rating, not two.
        let meta = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            classifications: vec![
                ira_models::ScraperClassification {
                    kind: "USK".into(),
                    value: "16".into(),
                },
                ira_models::ScraperClassification {
                    kind: "STEAM_GERMANY".into(),
                    value: "16".into(),
                },
                ira_models::ScraperClassification {
                    kind: "PEGI".into(),
                    value: "12".into(),
                },
            ],
            ..Default::default()
        };
        store_scraper_metadata(&conn, game_id, &meta).unwrap();
        let metadata = scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert_eq!(
            metadata
                .classifications
                .iter()
                .map(|c| (c.kind.as_str(), c.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("PEGI", "12"), ("USK", "16")]
        );
    }

    #[test]
    fn test_merge_replaces_the_epoch_date_but_never_a_real_one() {
        let (conn, _tmp) = setup_db();
        let game_id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            "Poisoned Game",
        )
        .unwrap();
        // The old diff bug stored the epoch string while the timestamp
        // column kept its real value; the merge must treat the string
        // as a hole, not as data.
        let poisoned = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            release_date: "1970-01-01".into(),
            ..Default::default()
        };
        store_scraper_metadata(&conn, game_id, &poisoned).unwrap();
        let fresh = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            release_date: "2015-09-15".into(),
            release_timestamp: 1_442_275_200,
            ..Default::default()
        };
        assert!(merge_missing_scraper_metadata(&conn, game_id, &fresh).unwrap());
        let metadata = scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert_eq!(metadata.release_date, "2015-09-15");
        assert_eq!(metadata.release_timestamp, 1_442_275_200);

        // A real stored date is never overwritten by the merge.
        let newer = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            release_date: "2020-05-01".into(),
            release_timestamp: 1_588_291_200,
            ..Default::default()
        };
        assert!(!merge_missing_scraper_metadata(&conn, game_id, &newer).unwrap());
        let metadata = scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert_eq!(metadata.release_date, "2015-09-15");
    }

    #[test]
    fn test_storing_a_match_reconciles_local_companies() {
        let (conn, _tmp) = setup_db();
        let game_id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            "Whoever's Game",
        )
        .unwrap();
        let local = steam_company_entity(&conn, "Whoever Studios").unwrap();
        let local_id: i64 = local.id.parse().unwrap();

        // A game carries the local company in its metadata.
        let meta = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            developers: vec![local.clone()],
            ..Default::default()
        };
        store_scraper_metadata(&conn, game_id, &meta).unwrap();
        let metadata = scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert!(metadata.developers.iter().any(|d| d.id == local.id));

        // A later match credits the same studio under its real
        // ScreenScraper id: the game's reference swaps and the local row
        // goes away.
        let real_meta = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            developers: vec![ira_models::ScraperEntity {
                id: "123".into(),
                name: "Whoever Studios".into(),
            }],
            ..Default::default()
        };
        store_scraper_metadata(&conn, game_id, &real_meta).unwrap();
        let metadata = scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert!(
            metadata.developers.iter().any(|d| d.id == "123"),
            "local {local_id} must migrate to 123"
        );
        assert!(!metadata.developers.iter().any(|d| d.id == local.id));
        let count: i64 = conn
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM scraper_companies WHERE id = ?1",
                rusqlite::params![local_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "the superseded local row must be gone");
    }
}

