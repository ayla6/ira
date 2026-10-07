use crate::{err, lock_db, DbConn};

/// Runs a query expected to return at most one row, mapping it through
/// `map`. `Ok(None)` means no row matched; any other error is reported.
pub(crate) fn query_optional<T, P>(
    conn: &DbConn,
    sql: &str,
    params: P,
    map: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Option<T>, String>
where
    P: rusqlite::Params,
{
    let c = lock_db(conn)?;
    match c.query_row(sql, params, map) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

/// [`query_optional`] for single-column rows.
pub(crate) fn query_optional_scalar<T, P>(
    conn: &DbConn,
    sql: &str,
    params: P,
) -> Result<Option<T>, String>
where
    T: rusqlite::types::FromSql,
    P: rusqlite::Params,
{
    query_optional(conn, sql, params, |row| row.get(0))
}

/// Runs a query returning any number of rows, mapping each through `map.
/// Consolidates the lock/prepare/collect pattern previously repeated in
/// every list getter.
pub(crate) fn query_all<T, P>(
    conn: &DbConn,
    sql: &str,
    params: P,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String>
where
    P: rusqlite::Params,
{
    let c = lock_db(conn)?;
    let mut stmt = c.prepare(sql).map_err(err)?;
    let rows = stmt.query_map(params, map).map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

/// [`query_all`] for single-column rows.
pub(crate) fn query_scalar_all<T, P>(
    conn: &DbConn,
    sql: &str,
    params: P,
) -> Result<Vec<T>, String>
where
    T: rusqlite::types::FromSql,
    P: rusqlite::Params,
{
    query_all(conn, sql, params, |row| row.get(0))
}

/// Runs a single INSERT/UPDATE/DELETE statement, consolidating the
/// lock/execute pattern previously repeated in every small setter.
pub(crate) fn execute(conn: &DbConn, sql: &str, params: impl rusqlite::Params) -> Result<usize, String> {
    let c = lock_db(conn)?;
    c.execute(sql, params).map_err(err)
}

/// Chunk size for batched `IN (...)` lookups: well under the SQLite variable
/// limit, large enough that a thousand-game library needs two round-trips.
pub(crate) const IN_QUERY_CHUNK_SIZE: usize = 500;

/// Sequential `?N` placeholders separated by ", ".
pub(crate) fn positional_placeholders(count: usize) -> String {
    (1..=count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::init_db;
    use rusqlite::params;

    #[test]
    fn test_query_optional_scalar_missing_row_is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = init_db(&tmp.path().join("test.db").to_string_lossy());
        let hit: Option<i64> =
            query_optional_scalar(&conn, "SELECT id FROM games WHERE id = ?1", params![7])
                .unwrap();
        assert_eq!(hit, None);
    }
}
