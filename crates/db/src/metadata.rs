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

/// Upsert one ScreenScraper entity table: ids are stable, but a name the
/// source has since corrected should win.
fn store_entities(
    conn: &rusqlite::Connection,
    table: &str,
    entities: &[(i64, String)],
) -> Result<(), String> {
    for (id, name) in entities {
        conn.execute(
            &format!(
                "INSERT INTO {table} (id, name) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name"
            ),
            params![id, name],
        )
        .map_err(err)?;
    }
    Ok(())
}

/// The id list of one entity field, as the JSON array the games row
/// stores — `[]` when the field is empty, so a deliberate clear still
/// overwrites a previous value.
fn entity_ids_json(entities: &[ira_models::ScraperEntity]) -> String {
    serde_json::to_string(
        &entities
            .iter()
            .map(|entity| entity.id.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}

/// Upsert the age-rating boards: ids are stable, the board kind and the
/// entry name follow the source.
fn store_classifications(
    conn: &rusqlite::Connection,
    entities: &[(i64, String, String)],
) -> Result<(), String> {
    for (id, kind, name) in entities {
        conn.execute(
            "INSERT INTO scraper_classifications (id, kind, name) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, name = excluded.name",
            params![id, kind, name],
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
    let mut companies: Vec<(i64, String)> = Vec::new();
    for entity in metadata.developers.iter().chain(&metadata.publishers) {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies.push((id, entity.name.clone()));
        }
    }
    let genres: Vec<(i64, String)> = metadata
        .genres
        .iter()
        .filter_map(|genre| genre.id.parse::<i64>().ok().map(|id| (id, genre.name.clone())))
        .collect();
    let classifications: Vec<(i64, String, String)> = metadata
        .classifications
        .iter()
        .filter_map(|c| c.id.parse::<i64>().ok().map(|id| (id, c.kind.clone(), c.name.clone())))
        .collect();
    let developer_id = entity_ids_json(&metadata.developers);
    let publisher_id = entity_ids_json(&metadata.publishers);
    let genre_ids = serde_json::to_string(
        &genres.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let classification_ids = serde_json::to_string(
        &classifications.iter().map(|(id, _, _)| id.to_string()).collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let synopses_json = serde_json::to_string(&metadata.synopses).unwrap_or_default();
    let dates_json = serde_json::to_string(
        &metadata
            .release_dates
            .iter()
            .cloned()
            .collect::<std::collections::HashMap<String, String>>(),
    )
    .unwrap_or_default();

    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    store_entities(&tx, "scraper_companies", &companies)?;
    reconcile_local_companies(&tx, &companies)?;
    store_entities(&tx, "scraper_genres", &genres)?;
    store_classifications(&tx, &classifications)?;
    tx.execute(
        "UPDATE games SET
            screenscraper_id = ?1,
            release_date = CASE WHEN ?2 != '' THEN ?2 ELSE release_date END,
            release_timestamp = CASE WHEN ?3 != 0 THEN ?3 ELSE release_timestamp END,
            release_dates = CASE WHEN ?4 != '' THEN ?4 ELSE release_dates END,
            developer_id = CASE WHEN ?5 != '' THEN ?5 ELSE developer_id END,
            publisher_id = CASE WHEN ?6 != '' THEN ?6 ELSE publisher_id END,
            genre_ids = CASE WHEN ?7 != '' THEN ?7 ELSE genre_ids END,
            players = CASE WHEN ?8 != '' THEN ?8 ELSE players END,
            screenscraper_rating = CASE WHEN ?9 > 0 THEN ?9 ELSE screenscraper_rating END,
            classification_ids = CASE WHEN ?10 != '' THEN ?10 ELSE classification_ids END,
            synopsis = CASE WHEN ?11 != '' THEN ?11 ELSE synopsis END
         WHERE id = ?12",
        params![
            metadata.ss_id,
            metadata.release_date,
            metadata.release_timestamp,
            dates_json,
            developer_id,
            publisher_id,
            genre_ids,
            metadata.players,
            metadata.rating,
            classification_ids,
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
    companies: &[(i64, String)],
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
    for (id, name) in companies {
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
            let (from, to) = (format!("\"{local_id}\""), format!("\"{id}\""));
            let like = format!("%{from}%");
            tx.execute(
                "UPDATE games SET
                    developer_id = replace(developer_id, ?1, ?2),
                    publisher_id = replace(publisher_id, ?1, ?2)
                 WHERE developer_id LIKE ?3 OR publisher_id LIKE ?3",
                rusqlite::params![from, to, like],
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
                developer_id, publisher_id, genre_ids, classification_ids,
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
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, String>(10)?,
            ))
        },
    )?;
    let Some((
        ss_id,
        release_date,
        release_timestamp,
        release_dates,
        developer_id,
        publisher_id,
        genre_ids,
        classification_ids,
        players,
        rating,
        synopsis,
    )) = row
    else {
        return Ok(None);
    };
    if ss_id.is_empty() {
        return Ok(None);
    }
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
        developers: expand_entities(conn, &developer_id, scraper_company_name)?,
        publishers: expand_entities(conn, &publisher_id, scraper_company_name)?,
        genres: expand_entities(conn, &genre_ids, scraper_genre_name)?,
        players,
        rating,
        classifications: classification_ids
            .split_json_ids()
            .into_iter()
            .filter_map(|id| {
                let numeric = id.parse::<i64>().ok()?;
                scraper_classification(conn, numeric)
                    .ok()
                    .flatten()
                    .map(|(kind, name)| ira_models::ScraperClassification { kind, id, name })
            })
            .collect(),
        synopses,
    }))
}

/// The entities behind one stored id array; ids whose lookup row is gone
/// resolve to nothing.
fn expand_entities(
    conn: &DbConn,
    ids_json: &str,
    lookup: impl Fn(&DbConn, i64) -> Result<Option<String>, String>,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    Ok(ids_json
        .split_json_ids()
        .into_iter()
        .filter_map(|id| {
            let numeric = id.parse::<i64>().ok()?;
            let name = lookup(conn, numeric).ok().flatten()?;
            Some(ira_models::ScraperEntity { id, name })
        })
        .collect())
}

/// The id list of a games-row JSON column, tolerating the empty string a
/// fresh row carries.
trait SplitJsonIds {
    fn split_json_ids(&self) -> Vec<String>;
}

impl SplitJsonIds for str {
    fn split_json_ids(&self) -> Vec<String> {
        if self.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<String>>(self).unwrap_or_default()
        }
    }
}

/// Local companies whose name contains the filter, case-insensitive and
/// alphabetically capped — the search index is the cache itself, since the
/// source has no company list endpoint.
pub fn scraper_companies_search(
    conn: &DbConn,
    filter: &str,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(
            "SELECT id, name FROM scraper_companies
             WHERE name LIKE '%' || ?1 || '%' COLLATE NOCASE
             ORDER BY name LIMIT 60",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![filter.trim()], |row| {
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

/// The genre name a ScreenScraper id refers to, per the lookup table.
pub fn scraper_genre_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_genres WHERE id = ?1",
        params![id],
    )
}

/// The board kind and entry name a ScreenScraper classification id
/// refers to, per the lookup table.
pub fn scraper_classification(
    conn: &DbConn,
    id: i64,
) -> Result<Option<(String, String)>, String> {
    crate::query_optional(
        conn,
        "SELECT kind, name FROM scraper_classifications WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
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
                players: "1-4".into(),
                rating: 18.0,
                classifications: vec![ira_models::ScraperClassification {
                    kind: "PEGI".into(),
                    id: "279".into(),
                    name: "PEGI:3".into(),
                }],
                synopses: vec![("en".into(), "The first two quests.".into())],
            },
        )
        .unwrap();

        let entry: GameEntry = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(entry.screenscraper_id, "2124");
        assert_eq!(entry.release_date, "1993-12-18");
        assert!(entry.release_timestamp > 0);
        // The entry keeps only the references, as id arrays; the names
        // live in the lookup tables.
        assert_eq!(entry.developer_id, r#"["2911"]"#);
        assert_eq!(entry.publisher_id, r#"["1299"]"#);
        assert_eq!(entry.genre_ids, r#"["2620"]"#);
        assert_eq!(entry.classification_ids, r#"["279"]"#);
        assert_eq!(entry.players, "1-4");
        assert_eq!(entry.screenscraper_rating, 18.0);
        assert_eq!(scraper_company_name(&conn, 2911).unwrap().as_deref(), Some("Chunsoft"));
        assert_eq!(scraper_company_name(&conn, 1299).unwrap().as_deref(), Some("Enix"));
        assert_eq!(
            scraper_genre_name(&conn, 2620).unwrap().as_deref(),
            Some("Role Playing Game")
        );
        assert_eq!(
            scraper_classification(&conn, 279).unwrap(),
            Some(("PEGI".to_string(), "PEGI:3".to_string()))
        );
        // Unknown ids resolve to nothing.
        assert_eq!(scraper_company_name(&conn, 1).unwrap(), None);
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

        // Never matched: nothing to read back.
        assert!(scraper_metadata_for_game(&conn, id).unwrap().is_none());

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
        assert_eq!(entry.developer_id, r#"["1"]"#);
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
    fn test_scraper_release_timestamp_parses_both_shapes() {
        let full = scraper_release_timestamp("1993-12-18");
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
        let stored_before: Vec<String> = {
            let developer_id: String = conn
                .get()
                .unwrap()
                .query_row(
                    "SELECT developer_id FROM games WHERE id = ?1",
                    rusqlite::params![game_id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&developer_id).unwrap()
        };
        assert!(stored_before.contains(&local.id));

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
        store_scraper_metadata(&conn, 7, &real_meta).unwrap();
        let stored: Vec<String> = {
            let developer_id: String = conn
                .get()
                .unwrap()
                .query_row(
                    "SELECT developer_id FROM games WHERE id = ?1",
                    rusqlite::params![game_id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&developer_id).unwrap()
        };
        assert!(stored.contains(&"123".to_string()), "local {local_id} must migrate to 123");
        assert!(!stored.contains(&local.id));
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

