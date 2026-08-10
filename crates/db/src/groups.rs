use crate::DbConn;
use ira_models::Group;
use rusqlite::params;

pub fn create_group(conn: &DbConn, name: &str) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute("INSERT INTO groups (name) VALUES (?1)", params![name])
        .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn rename_group(conn: &DbConn, id: i64, name: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE groups SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_group(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM game_groups WHERE group_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM groups WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_all_groups(conn: &DbConn) -> Result<Vec<Group>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT id, name FROM groups ORDER BY name COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let groups = stmt
        .query_map([], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    groups
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn add_game_to_group(conn: &DbConn, game_id: i64, group_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT OR IGNORE INTO game_groups (game_id, group_id) VALUES (?1, ?2)",
        params![game_id, group_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_game_from_group(conn: &DbConn, game_id: i64, group_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "DELETE FROM game_groups WHERE game_id = ?1 AND group_id = ?2",
        params![game_id, group_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_groups_for_game(conn: &DbConn, game_id: i64) -> Result<Vec<Group>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(
            "SELECT g.id, g.name FROM groups g
             JOIN game_groups gg ON g.id = gg.group_id
             WHERE gg.game_id = ?1
             ORDER BY g.name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let groups = stmt
        .query_map(params![game_id], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    groups
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_game_ids_in_group(conn: &DbConn, group_id: i64) -> Result<Vec<i64>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare("SELECT game_id FROM game_groups WHERE group_id = ?1")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map(params![group_id], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    ids.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
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
                "CREATE TABLE games (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL);
                 CREATE TABLE groups (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE);
                 CREATE TABLE game_groups (
                     game_id INTEGER NOT NULL,
                     group_id INTEGER NOT NULL,
                     PRIMARY KEY (game_id, group_id)
                 );",
            )
            .unwrap();
        }
        pool
    }

    #[test]
    fn test_create_and_load_group() {
        let db = temp_db();
        let id = create_group(&db, "Favorites").unwrap();
        assert!(id > 0);
        let groups = get_all_groups(&db).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Favorites");
    }

    #[test]
    fn test_rename_group() {
        let db = temp_db();
        let id = create_group(&db, "Old Name").unwrap();
        rename_group(&db, id, "New Name").unwrap();
        let groups = get_all_groups(&db).unwrap();
        assert_eq!(groups[0].name, "New Name");
    }

    #[test]
    fn test_delete_group() {
        let db = temp_db();
        let id = create_group(&db, "Temp").unwrap();
        delete_group(&db, id).unwrap();
        let groups = get_all_groups(&db).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_add_remove_game_from_group() {
        let db = temp_db();
        let group_id = create_group(&db, "Favorites").unwrap();
        add_game_to_group(&db, 1, group_id).unwrap();
        add_game_to_group(&db, 2, group_id).unwrap();

        let ids = get_game_ids_in_group(&db, group_id).unwrap();
        assert_eq!(ids.len(), 2);

        remove_game_from_group(&db, 1, group_id).unwrap();
        let ids = get_game_ids_in_group(&db, group_id).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], 2);
    }

    #[test]
    fn test_get_groups_for_game() {
        let db = temp_db();
        let g1 = create_group(&db, "Favorites").unwrap();
        let g2 = create_group(&db, "Completed").unwrap();
        add_game_to_group(&db, 1, g1).unwrap();
        add_game_to_group(&db, 1, g2).unwrap();

        let groups = get_groups_for_game(&db, 1).unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_add_duplicate_is_ignored() {
        let db = temp_db();
        let group_id = create_group(&db, "Favorites").unwrap();
        add_game_to_group(&db, 1, group_id).unwrap();
        add_game_to_group(&db, 1, group_id).unwrap();
        let ids = get_game_ids_in_group(&db, group_id).unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_delete_group_cascades() {
        let db = temp_db();
        let group_id = create_group(&db, "Favorites").unwrap();
        add_game_to_group(&db, 1, group_id).unwrap();
        delete_group(&db, group_id).unwrap();
        let ids = get_game_ids_in_group(&db, group_id).unwrap();
        assert!(ids.is_empty());
    }
}
