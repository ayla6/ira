use crate::DbConn;

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
    if let Some(found) = scan_companies(conn, &tokens) {
        return Some(found);
    }
    let c = match crate::lock_db(conn) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("steam_company_entity: failed to lock db for {name}: {e}");
            return None;
        }
    };
    let id = next_local_id(&c, "scraper_companies");
    let name = name.trim().to_string();
    // A concurrent allocation of the same company loses the insert and
    // the re-scan below hands back the winner, whatever spelling it kept.
    let _ = c.execute(
        "INSERT INTO scraper_companies (id, name) VALUES (?1, ?2)",
        rusqlite::params![id, name],
    );
    scan_locals(&c, &tokens).or(Some(ira_models::ScraperEntity {
        id: id.to_string(),
        name,
    }))
}

/// One below the smallest local id in `table` — the next free negative
/// id in the namespace ScreenScraper's positive ids can never reach.
fn next_local_id(
    c: &r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
    table: &str,
) -> i64 {
    let min: i64 = c
        .query_row(
            &format!("SELECT COALESCE(MIN(id), 0) FROM {table} WHERE id < 0"),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    min - 1
}

/// The whole-table token scan behind [`steam_company_entity`]: real
/// ScreenScraper ids first, then locals oldest-first.
fn scan_companies(
    conn: &DbConn,
    tokens: &[String],
) -> Option<ira_models::ScraperEntity> {
    let c = match crate::lock_db(conn) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("steam_company_entity: failed to lock db for scan: {e}");
            return None;
        }
    };
    let mut stmt = match c.prepare(
        "SELECT id, name FROM scraper_companies ORDER BY (id < 0), id DESC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            eprintln!("steam_company_entity: scan prepare failed: {e}");
            return None;
        }
    };
    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("steam_company_entity: scan query failed: {e}");
            return None;
        }
    };
    for (id, name) in rows.filter_map(Result::ok) {
        if ira_models::company_tokens(&name) == *tokens {
            return Some(ira_models::ScraperEntity {
                id: id.to_string(),
                name,
            });
        }
    }
    None
}

/// The local-only re-scan after a mint: hands back the concurrent
/// winner when two threads minted the same company at once.
fn scan_locals(
    c: &r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
    tokens: &[String],
) -> Option<ira_models::ScraperEntity> {
    let mut stmt = c
        .prepare("SELECT id, name FROM scraper_companies WHERE id < 0 ORDER BY id DESC")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    let found: Option<(i64, String)> = rows
        .filter_map(Result::ok)
        .find(|(_, n)| ira_models::company_tokens(n) == *tokens);
    found.map(|(local_id, local_name)| ira_models::ScraperEntity {
        id: local_id.to_string(),
        name: local_name,
    })
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
    match crate::entity_search(conn, table, name) {
        Ok(found) => {
            if let Some(entity) = found
                .iter()
                .find(|e| ira_models::company_tokens(&e.name) == tokens)
            {
                return Some(entity.clone());
            }
        }
        Err(e) => {
            eprintln!("local_entity: search failed in {table} for {name}: {e}");
        }
    }
    let c = match crate::lock_db(conn) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("local_entity: failed to lock db for {name}: {e}");
            return None;
        }
    };
    let id = next_local_id(&c, table);
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
    use super::super::init_db;
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_steam_company_entity_allocates_and_reuses_locals() {
        let tmp = TempDir::new().unwrap();
        let conn = init_db(&tmp.path().join("test.db").to_string_lossy());
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
