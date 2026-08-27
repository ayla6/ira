use crate::{err, DbConn};
use ira_models::WineProfile;
use rusqlite::params;

pub fn add_profile(conn: &DbConn, profile: &WineProfile) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO wine_profiles (name, wine_version, custom_wine_path, prefix, arch, umu_enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![profile.name, profile.wine_version, profile.custom_wine_path, profile.prefix, profile.arch, profile.umu_enabled],
    ).map_err(err)?;
    Ok(c.last_insert_rowid())
}

fn wine_profile_from_row(row: &rusqlite::Row) -> rusqlite::Result<WineProfile> {
    Ok(WineProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        wine_version: row.get(2)?,
        custom_wine_path: row.get(3)?,
        prefix: row.get(4)?,
        arch: row.get(5)?,
        umu_enabled: row.get(6)?,
    })
}

pub fn get_all_profiles(conn: &DbConn) -> Result<Vec<WineProfile>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, name, wine_version, custom_wine_path, prefix, arch, umu_enabled FROM wine_profiles ORDER BY name",
    ).map_err(err)?;
    let rows = stmt.query_map([], wine_profile_from_row).map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

pub fn update_profile(conn: &DbConn, profile: &WineProfile) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE wine_profiles SET name = ?1, wine_version = ?2, custom_wine_path = ?3, prefix = ?4, arch = ?5, umu_enabled = ?6 WHERE id = ?7",
        params![profile.name, profile.wine_version, profile.custom_wine_path, profile.prefix, profile.arch, profile.umu_enabled, profile.id],
    ).map_err(err)?;
    Ok(())
}

pub fn delete_profile(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM wine_profiles WHERE id = ?1", params![id])
        .map_err(err)?;
    Ok(())
}

pub fn get_profile(conn: &DbConn, id: i64) -> Result<Option<WineProfile>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(
        "SELECT id, name, wine_version, custom_wine_path, prefix, arch, umu_enabled FROM wine_profiles WHERE id = ?1",
    ).map_err(err)?;
    let mut rows = stmt
        .query_map(params![id], wine_profile_from_row)
        .map_err(err)?;
    if let Some(row) = rows.next() {
        row.map_err(err).map(Some)
    } else {
        Ok(None)
    }
}
