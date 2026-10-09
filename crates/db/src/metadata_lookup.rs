use crate::DbConn;
use rusqlite::params;

/// The shared shape of the company/genre/family reads and the picker searches.
fn query_entities(
    conn: &DbConn,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<ira_models::ScraperEntity>, String> {
    crate::query_all(
        conn,
        sql,
        params,
        |row| {
            Ok(ira_models::ScraperEntity {
                id: row.get::<_, i64>(0)?.to_string(),
                name: row.get(1)?,
            })
        },
    )
}

/// Both company credits in one query: a studio that is both developer
/// and publisher surfaces in both lists from the same junction row.
fn game_companies_both(
    conn: &DbConn,
    game_id: i64,
) -> Result<
    (
        Vec<ira_models::ScraperEntity>,
        Vec<ira_models::ScraperEntity>,
    ),
    String,
> {
    let rows: Vec<(i64, String, i64, i64)> = crate::query_all(
        conn,
        "SELECT c.id, c.name, gc.is_developer, gc.is_publisher
         FROM scraper_game_companies gc
         JOIN scraper_companies c ON c.id = gc.company_id
         WHERE gc.game_id = ?1
         ORDER BY c.name",
        params![game_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut developers = Vec::new();
    let mut publishers = Vec::new();
    for (id, name, is_developer, is_publisher) in rows {
        if is_developer != 0 {
            developers.push(ira_models::ScraperEntity {
                id: id.to_string(),
                name: name.clone(),
            });
        }
        if is_publisher != 0 {
            publishers.push(ira_models::ScraperEntity {
                id: id.to_string(),
                name,
            });
        }
    }
    Ok((developers, publishers))
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
    let rows: Vec<ira_models::ScraperClassification> = crate::query_all(
        conn,
        "SELECT kind, value FROM scraper_game_classifications
         WHERE game_id = ?1
         ORDER BY kind",
        params![game_id],
        |row| {
            Ok(ira_models::ScraperClassification {
                kind: row.get(0)?,
                value: row.get(1)?,
            })
        },
    )?;
    Ok(rows
        .into_iter()
        .map(|class| ira_models::ScraperClassification {
            kind: ira_models::ratings::canonical_kind(&class.kind),
            value: class.value,
        })
        .collect())
}

pub fn scraper_company_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_companies WHERE id = ?1",
        params![id],
    )
}

pub fn scraper_genre_name(conn: &DbConn, id: i64) -> Result<Option<String>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT name FROM scraper_genres WHERE id = ?1",
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
        serde_json::from_str(&release_dates).unwrap_or_else(|e| {
            eprintln!("scraper_metadata_for_game: corrupt release_dates JSON for game {game_id}: {e}");
            Default::default()
        })
    };
    let mut release_dates: Vec<(String, String)> = dates.into_iter().collect();
    release_dates.sort();
    let synopses: Vec<(String, String)> = if synopsis.is_empty() {
        Default::default()
    } else {
        serde_json::from_str(&synopsis).unwrap_or_else(|e| {
            eprintln!("scraper_metadata_for_game: corrupt synopsis JSON for game {game_id}: {e}");
            Default::default()
        })
    };
    let (developers, publishers) = game_companies_both(conn, game_id)?;
    Ok(Some(ira_models::ScraperMetadata {
        ss_id,
        release_date,
        release_timestamp,
        release_dates,
        developers,
        publishers,
        genres: game_genres(conn, game_id)?,
        families: game_families(conn, game_id)?,
        players,
        rating,
        classifications: game_classifications(conn, game_id)?,
        synopses,
    }))
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

#[cfg(test)]
mod tests {
    use super::super::{add_game, init_db, store_scraper_metadata};
    use super::*;
    use ira_models::{GameKind, TrophySource};
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
                kind: GameKind::Retro,
                trophy_source: TrophySource::Empty,
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
    fn test_scraper_metadata_for_game_expands_ids() {
        let (conn, _tmp) = setup_db();
        let id = test_game(&conn, "Round Trip");

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
}
