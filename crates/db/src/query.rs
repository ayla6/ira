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
