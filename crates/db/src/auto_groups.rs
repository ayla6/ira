//! Auto groups: named rule sets stored as JSON criteria. Membership is
//! never stored — it is derived from the live games wherever the rules
//! are evaluated — so a match, a play session, or an edit reshapes every
//! auto group without a single write.

use crate::{err, DbConn};
use ira_models::{AutoGroup, AutoNode};
use rusqlite::params;
use rusqlite::OptionalExtension;

fn group_from_row(row: &rusqlite::Row) -> rusqlite::Result<AutoGroup> {
    let id: i64 = row.get(0)?;
    let name: String = row.get(1)?;
    let json: String = row.get(2)?;
    let root: AutoNode = serde_json::from_str(&json).unwrap_or_default();
    Ok(AutoGroup { id, name, root })
}

pub fn get_all_auto_groups(conn: &DbConn) -> Result<Vec<AutoGroup>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT id, name, criteria FROM auto_groups ORDER BY name COLLATE NOCASE")
        .map_err(err)?;
    let groups = stmt
        .query_map([], group_from_row)
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(groups)
}

pub fn get_auto_group(conn: &DbConn, id: i64) -> Result<Option<AutoGroup>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT id, name, criteria FROM auto_groups WHERE id = ?1")
        .map_err(err)?;
    let group = stmt
        .query_row(params![id], group_from_row)
        .optional()
        .map_err(err)?;
    Ok(group)
}

/// A name collision loses to the UNIQUE constraint and surfaces as the
/// error string, the same way user groups behave.
pub fn create_auto_group(conn: &DbConn, name: &str, root: &AutoNode) -> Result<i64, String> {
    let json = serde_json::to_string(root).map_err(err)?;
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO auto_groups (name, criteria) VALUES (?1, ?2)",
        params![name, json],
    )
    .map_err(err)?;
    Ok(c.last_insert_rowid())
}

/// The whole rule set is rewritten on every save — the editor hands back
/// the full list, so a diff would only re-encode it.
pub fn update_auto_group(conn: &DbConn, id: i64, name: &str, root: &AutoNode) -> Result<(), String> {
    let json = serde_json::to_string(root).map_err(err)?;
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE auto_groups SET name = ?1, criteria = ?2 WHERE id = ?3",
        params![name, json, id],
    )
    .map_err(err)?;
    Ok(())
}

pub fn delete_auto_group(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM auto_groups WHERE id = ?1", params![id])
        .map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> DbConn {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(4).build(manager).unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch(
                "CREATE TABLE auto_groups (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name TEXT NOT NULL UNIQUE,
                     criteria TEXT NOT NULL DEFAULT '[]'
                 );",
            )
            .unwrap();
        }
        pool
    }

    fn criterion() -> AutoNode {
        AutoNode::Rule(ira_models::AutoCriterion {
            dimension: ira_models::AutoDimension::Genre,
            values: vec!["JRPG".into()],
            ..Default::default()
        })
    }

    #[test]
    fn test_create_and_load_auto_group_keeps_criteria() {
        let db = temp_db();
        let id = create_auto_group(&db, "JRPGs", &criterion()).unwrap();
        assert!(id > 0);
        let groups = get_all_auto_groups(&db).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "JRPGs");
        assert_eq!(groups[0].root, criterion());
    }

    #[test]
    fn test_update_auto_group_rewrites_name_and_rules() {
        let db = temp_db();
        let id = create_auto_group(&db, "Old", &criterion()).unwrap();
        let changed = AutoNode::Rule(ira_models::AutoCriterion {
            dimension: ira_models::AutoDimension::Playtime,
            min_hours: Some(2.0),
            ..Default::default()
        });
        update_auto_group(&db, id, "New", &changed).unwrap();
        let group = get_auto_group(&db, id).unwrap().unwrap();
        assert_eq!(group.name, "New");
        assert_eq!(group.root, changed);
    }

    #[test]
    fn test_delete_auto_group() {
        let db = temp_db();
        let id = create_auto_group(&db, "Temp", &AutoNode::None).unwrap();
        delete_auto_group(&db, id).unwrap();
        assert!(get_all_auto_groups(&db).unwrap().is_empty());
        assert!(get_auto_group(&db, id).unwrap().is_none());
    }

    #[test]
    fn test_duplicate_name_is_rejected() {
        let db = temp_db();
        create_auto_group(&db, "Same", &AutoNode::None).unwrap();
        assert!(create_auto_group(&db, "Same", &AutoNode::None).is_err());
    }

    #[test]
    fn test_broken_criteria_json_degrades_to_no_rules() {
        let db = temp_db();
        let c = crate::lock_db(&db).unwrap();
        c.execute(
            "INSERT INTO auto_groups (name, criteria) VALUES ('Broken', 'not json')",
            [],
        )
        .unwrap();
        let groups = get_all_auto_groups(&db).unwrap();
        assert_eq!(groups.len(), 1);
        assert!(matches!(groups[0].root, AutoNode::None));
    }
}
