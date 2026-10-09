use crate::entities::{KIND_COMPANY, KIND_FAMILY, KIND_GENRE};
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

/// One row per company with role flags: a studio credited as both
/// developer and publisher shares a single company row.
fn collect_companies(
    developers: &[ira_models::ScraperEntity],
    publishers: &[ira_models::ScraperEntity],
) -> std::collections::BTreeMap<i64, (String, bool, bool)> {
    let mut companies: std::collections::BTreeMap<i64, (String, bool, bool)> =
        Default::default();
    for entity in developers {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies
                .entry(id)
                .or_insert((entity.name.trim().to_string(), false, false))
                .1 = true;
        }
    }
    for entity in publishers {
        if let Ok(id) = entity.id.parse::<i64>() {
            companies
                .entry(id)
                .or_insert((entity.name.trim().to_string(), false, false))
                .2 = true;
        }
    }
    companies
}

/// Id/name pairs with parseable ids — unparseable rows never reach the
/// integer-keyed lookup tables.
fn parse_id_entities(entities: &[ira_models::ScraperEntity]) -> Vec<(i64, String)> {
    entities
        .iter()
        .filter_map(|entity| {
            entity
                .id
                .parse::<i64>()
                .ok()
                .map(|id| (id, entity.name.clone()))
        })
        .collect()
}

fn store_company_links(
    tx: &rusqlite::Transaction,
    game_id: i64,
    companies: &std::collections::BTreeMap<i64, (String, bool, bool)>,
) -> Result<(), String> {
    let company_rows: Vec<(i64, String)> = companies
        .iter()
        .map(|(id, (name, _, _))| (*id, name.clone()))
        .collect();
    store_entities(tx, "scraper_companies", &company_rows)?;
    tx.execute(
        "DELETE FROM scraper_game_companies WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, (_, is_developer, is_publisher)) in companies {
        tx.execute(
            "INSERT INTO scraper_game_companies
                (game_id, company_id, is_developer, is_publisher)
             VALUES (?1, ?2, ?3, ?4)",
            params![game_id, id, *is_developer as i64, *is_publisher as i64],
        )
        .map_err(err)?;
    }
    Ok(())
}

fn store_genre_links(
    tx: &rusqlite::Transaction,
    game_id: i64,
    genres: &[(i64, String)],
) -> Result<(), String> {
    store_entities(tx, "scraper_genres", genres)?;
    tx.execute(
        "DELETE FROM scraper_game_genres WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, _) in genres {
        tx.execute(
            "INSERT OR IGNORE INTO scraper_game_genres (game_id, genre_id) VALUES (?1, ?2)",
            params![game_id, id],
        )
        .map_err(err)?;
    }
    Ok(())
}

fn store_family_links(
    tx: &rusqlite::Transaction,
    game_id: i64,
    families: &[(i64, String)],
) -> Result<(), String> {
    store_entities(tx, "scraper_families", families)?;
    tx.execute(
        "DELETE FROM scraper_game_families WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for (id, _) in families {
        tx.execute(
            "INSERT OR IGNORE INTO scraper_game_families (game_id, family_id) VALUES (?1, ?2)",
            params![game_id, id],
        )
        .map_err(err)?;
    }
    Ok(())
}

fn store_classification_rows(
    tx: &rusqlite::Transaction,
    game_id: i64,
    classifications: &[ira_models::ScraperClassification],
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM scraper_game_classifications WHERE game_id = ?1",
        params![game_id],
    )
    .map_err(err)?;
    for class in classifications {
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
    Ok(())
}

fn update_game_scraper_fields(
    tx: &rusqlite::Transaction,
    game_id: i64,
    metadata: &ira_models::ScraperMetadata,
    dates_json: &str,
    synopses_json: &str,
) -> Result<(), String> {
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
    // and refetch at the only choke point every writer shares. Only the
    // four entity lists resolve — the scalars are borrowed, never cloned.
    let developers = crate::resolve_entities(conn, KIND_COMPANY, &metadata.developers);
    let publishers = crate::resolve_entities(conn, KIND_COMPANY, &metadata.publishers);
    let genre_entities = crate::resolve_entities(conn, KIND_GENRE, &metadata.genres);
    let family_entities = crate::resolve_entities(conn, KIND_FAMILY, &metadata.families);
    let companies = collect_companies(&developers, &publishers);
    let genres = parse_id_entities(&genre_entities);
    let families = parse_id_entities(&family_entities);
    let synopses_json = serde_json::to_string(&metadata.synopses).unwrap_or_else(|e| {
        eprintln!("store_scraper_metadata: synopses encode failed for game {game_id}: {e}");
        Default::default()
    });
    let dates_json = serde_json::to_string(
        &metadata
            .release_dates
            .iter()
            .cloned()
            .collect::<std::collections::HashMap<String, String>>(),
    )
    .unwrap_or_else(|e| {
        eprintln!("store_scraper_metadata: dates encode failed for game {game_id}: {e}");
        Default::default()
    });

    // Immediate: this store runs on enricher threads while the matcher's
    // worker writes too, and a deferred transaction that read first only
    // finds out at write time — as an immediate "database is locked".
    let mut c = crate::lock_db(conn)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    store_company_links(&tx, game_id, &companies)?;
    store_genre_links(&tx, game_id, &genres)?;
    store_family_links(&tx, game_id, &families)?;
    store_classification_rows(&tx, game_id, &metadata.classifications)?;
    reconcile_local_companies(&tx, &companies)?;
    update_game_scraper_fields(&tx, game_id, metadata, &dates_json, &synopses_json)?;
    tx.commit().map_err(err)?;
    Ok(())
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
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)?
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
    let mut meta = crate::scraper_metadata_for_game(conn, game_id)?.unwrap_or_default();
    if !meta.fill_gaps(fresh) {
        return Ok(false);
    }
    store_scraper_metadata(conn, game_id, &meta)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::{add_game, find_by_db_id, init_db};
    use super::*;
    use ira_models::{GameEntry, GameKind, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = init_db(&db_path.to_string_lossy());
        (conn, tmp)
    }

    fn test_game(conn: &DbConn, title: &str) -> i64 {
        add_game(
            conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title,
            },
        )
        .unwrap()
    }

    #[test]
    fn test_store_scraper_metadata_persists_every_field() {
        let (conn, _tmp) = setup_db();
        let id = test_game(&conn, "Dragon Quest I & II");

        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "2124".into(),
                release_date: "1993-12-18".into(),
                release_timestamp: crate::scraper_release_timestamp("1993-12-18"),
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
        assert_eq!(crate::scraper_company_name(&conn, 2911).unwrap().as_deref(), Some("Chunsoft"));
        assert_eq!(crate::scraper_company_name(&conn, 1299).unwrap().as_deref(), Some("Enix"));
        assert_eq!(
            crate::scraper_genre_name(&conn, 2620).unwrap().as_deref(),
            Some("Role Playing Game")
        );
        // Unknown ids resolve to nothing.
        assert_eq!(crate::scraper_company_name(&conn, 1).unwrap(), None);
        let metadata = crate::scraper_metadata_for_game(&conn, id).unwrap().unwrap();
        assert_eq!(
            metadata.families,
            vec![ira_models::ScraperEntity {
                id: "732".to_string(),
                name: "Dragon Quest".to_string()
            }]
        );
        assert_eq!(crate::search_families(&conn, "quest").unwrap().len(), 1);
        let synopses: Vec<(String, String)> = serde_json::from_str(&entry.synopsis).unwrap();
        assert_eq!(synopses, vec![("en".to_string(), "The first two quests.".to_string())]);
        let dates: std::collections::HashMap<String, String> =
            serde_json::from_str(&entry.release_dates).unwrap();
        assert_eq!(dates.get("us").map(String::as_str), Some("1993-12-18"));
    }

    #[test]
    fn test_store_scraper_metadata_refreshes_entity_names() {
        let (conn, _tmp) = setup_db();
        let id = test_game(&conn, "Matched");
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
        assert_eq!(crate::scraper_company_name(&conn, 2911).unwrap().as_deref(), Some("ChunSoft"));
    }

    #[test]
    fn test_store_scraper_metadata_keeps_existing_release_dates() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            crate::NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "1",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Dated",
            },
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

        // A dated hit replaces it.
        store_scraper_metadata(
            &conn,
            id,
            &ira_models::ScraperMetadata {
                ss_id: "9".into(),
                release_date: "1993-12-18".into(),
                release_timestamp: crate::scraper_release_timestamp("1993-12-18"),
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
    fn test_board_aliases_collapse_into_one_row() {
        let (conn, _tmp) = setup_db();
        let game_id = test_game(&conn, "Twice Rated");
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
        let metadata = crate::scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
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
        let game_id = test_game(&conn, "Poisoned Game");
        // The epoch string counts as a hole, not as data.
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
        let metadata = crate::scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
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
        let metadata = crate::scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
        assert_eq!(metadata.release_date, "2015-09-15");
    }

    #[test]
    fn test_storing_a_match_reconciles_local_companies() {
        let (conn, _tmp) = setup_db();
        let game_id = test_game(&conn, "Whoever's Game");
        let local = crate::steam_company_entity(&conn, "Whoever Studios").unwrap();
        let local_id: i64 = local.id.parse().unwrap();

        let meta = ira_models::ScraperMetadata {
            ss_id: "1".into(),
            developers: vec![local.clone()],
            ..Default::default()
        };
        store_scraper_metadata(&conn, game_id, &meta).unwrap();
        let metadata = crate::scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
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
        let metadata = crate::scraper_metadata_for_game(&conn, game_id).unwrap().unwrap();
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
