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
    if let Some(entity) = &metadata.developer {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies.push((id, entity.name.clone()));
        }
    }
    if let Some(entity) = &metadata.publisher {
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
    let developer_id = metadata.developer.as_ref().map(|e| e.id.clone()).unwrap_or_default();
    let publisher_id = metadata.publisher.as_ref().map(|e| e.id.clone()).unwrap_or_default();
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

/// The company name a ScreenScraper id refers to, per the lookup table.
pub fn scraper_company_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_companies WHERE id = ?1",
        params![id],
    )
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
                developer: Some(ira_models::ScraperEntity {
                    id: "2911".into(),
                    name: "Chunsoft".into(),
                }),
                publisher: Some(ira_models::ScraperEntity {
                    id: "1299".into(),
                    name: "Enix".into(),
                }),
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
        // The entry keeps only the references; the names live in the
        // lookup tables.
        assert_eq!(entry.developer_id, "2911");
        assert_eq!(entry.publisher_id, "1299");
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
                developer: Some(company()),
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
                developer: Some(ira_models::ScraperEntity {
                    id: "2911".into(),
                    name: "ChunSoft".into(),
                }),
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
                developer: Some(ira_models::ScraperEntity {
                    id: "1".into(),
                    name: "id".into(),
                }),
                publisher: Some(ira_models::ScraperEntity {
                    id: "2".into(),
                    name: "pub".into(),
                }),
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
        assert_eq!(entry.developer_id, "1");
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
}
