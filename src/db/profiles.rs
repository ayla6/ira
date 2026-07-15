use crate::db::DbConn;
use crate::models::WineProfile;
use rusqlite::params;

pub fn add_profile(
    conn: &DbConn,
    name: &str,
    wine_version: &str,
    custom_wine_path: &str,
    prefix: &str,
    arch: &str,
) -> Result<i64, String> {
    let c = crate::db::lock_db(conn)?;
    c.execute(
        "INSERT INTO wine_profiles (name, wine_version, custom_wine_path, prefix, arch) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![name, wine_version, custom_wine_path, prefix, arch],
    ).map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

pub fn get_all_profiles(conn: &DbConn) -> Result<Vec<WineProfile>, String> {
    let c = crate::db::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, name, wine_version, custom_wine_path, prefix, arch FROM wine_profiles ORDER BY name",
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(WineProfile {
            id: row.get(0)?,
            name: row.get(1)?,
            wine_version: row.get(2)?,
            custom_wine_path: row.get(3)?,
            prefix: row.get(4)?,
            arch: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn update_profile(
    conn: &DbConn,
    id: i64,
    name: &str,
    wine_version: &str,
    custom_wine_path: &str,
    prefix: &str,
    arch: &str,
) -> Result<(), String> {
    let c = crate::db::lock_db(conn)?;
    c.execute(
        "UPDATE wine_profiles SET name = ?1, wine_version = ?2, custom_wine_path = ?3, prefix = ?4, arch = ?5 WHERE id = ?6",
        params![name, wine_version, custom_wine_path, prefix, arch, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_profile(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::db::lock_db(conn)?;
    c.execute("DELETE FROM wine_profiles WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_profile(conn: &DbConn, id: i64) -> Result<Option<WineProfile>, String> {
    let c = crate::db::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, name, wine_version, custom_wine_path, prefix, arch FROM wine_profiles WHERE id = ?1",
    ).map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(WineProfile {
            id: row.get(0)?,
            name: row.get(1)?,
            wine_version: row.get(2)?,
            custom_wine_path: row.get(3)?,
            prefix: row.get(4)?,
            arch: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next() {
        Ok(Some(row.map_err(|e| e.to_string())?))
    } else {
        Ok(None)
    }
}
