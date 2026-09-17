use crate::{err, update_field, DbConn};
use rusqlite::params;

pub fn set_game_hidden(conn: &DbConn, id: i64, hidden: bool) -> Result<(), String> {
    update_field(conn, id, "hidden", &hidden)
}

/// Marks whether the source scan that owns this row still finds the game's
/// files. A vanished row keeps its playtime and trophies but game loads
/// ignore it entirely — it is not the user's hidden flag.
pub fn set_game_vanished(conn: &DbConn, id: i64, vanished: bool) -> Result<(), String> {
    update_field(conn, id, "vanished", &vanished)
}

pub fn set_logo_settings(conn: &DbConn, id: i64, position: &str, size: i32) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET logo_position = ?1, logo_size = ?2 WHERE id = ?3",
        params![position, size, id],
    )
    .map_err(err)?;
    Ok(())
}

pub fn set_sgdb_id(conn: &DbConn, id: i64, sgdb_id: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET sgdb_id = ?1 WHERE id = ?2",
        params![
            if sgdb_id.is_empty() {
                None
            } else {
                Some(sgdb_id)
            },
            id
        ],
    )
    .map_err(err)?;
    Ok(())
}

pub fn set_shadps4_version(conn: &DbConn, id: i64, version: &str) -> Result<(), String> {
    update_field(conn, id, "shadps4_version", &version)
}

pub fn set_last_played(conn: &DbConn, id: i64, timestamp: i64) -> Result<(), String> {
    update_field(conn, id, "last_played", &timestamp)
}

pub fn set_ra_core(conn: &DbConn, id: i64, core: &str) -> Result<(), String> {
    update_field(conn, id, "ra_core", &core)
}

pub fn set_emulator_override(conn: &DbConn, id: i64, emulator: &str) -> Result<(), String> {
    update_field(conn, id, "emulator_override", &emulator)
}

pub fn set_rom_path(conn: &DbConn, id: i64, rom_path: &str) -> Result<(), String> {
    update_field(conn, id, "rom_path", &rom_path)
}

/// Stores the ROM's content hash, used for name-independent RA matching
/// and to reattach a game whose ROM reappears under a new name or path.
/// Set one key of the games row's hashes object, keeping the others: the
/// RA pass and the content pass each only know their own key.
/// Detach a game from its ScreenScraper match: the metadata stays, but
/// the mass matcher will offer the game again and searches are allowed.
pub fn clear_screenscraper_match(conn: &DbConn, id: i64) -> Result<(), String> {
    update_field(conn, id, "screenscraper_id", &"")
}

pub fn set_title_trusted(conn: &DbConn, id: i64, trusted: bool) -> Result<(), String> {
    update_field(conn, id, "title_trusted", &(trusted as i64))
}

pub fn set_hash_key(conn: &DbConn, id: i64, key: &str, value: &str) -> Result<(), String> {
    let mut c = crate::lock_db(conn)?;
    // One immediate transaction around the read-modify-write: the SS
    // batch and the RA pass store different keys from separate pooled
    // connections, and without it each writer erases the other's key.
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let current: Option<String> = tx
        .query_row(
            "SELECT hashes FROM games WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(err)?;
    let mut hashes: ira_models::RomHashes =
        serde_json::from_str(&current.unwrap_or_default()).unwrap_or_default();
    match key {
        "md5" => hashes.md5 = value.to_string(),
        "ra_md5" => hashes.ra_md5 = value.to_string(),
        "size" => hashes.size = value.parse().unwrap_or(0),
        other => return Err(format!("unknown hash key {other}")),
    }
    let json = serde_json::to_string(&hashes).map_err(err)?;
    tx.execute("UPDATE games SET hashes = ?1 WHERE id = ?2", params![json, id])
        .map_err(err)?;
    tx.commit().map_err(err)
}
