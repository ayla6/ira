use crate::{update_field, DbConn};
use rusqlite::params;

pub fn set_game_hidden(conn: &DbConn, id: i64, hidden: bool) -> Result<(), String> {
    update_field(conn, id, "hidden", &hidden)
}

pub fn set_logo_settings(conn: &DbConn, id: i64, position: &str, size: i32) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET logo_position = ?1, logo_size = ?2 WHERE id = ?3",
        params![position, size, id],
    )
    .map_err(|e| e.to_string())?;
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
    .map_err(|e| e.to_string())?;
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

/// Stores the ROM identification hashes used by No-Intro (CRC32) and
/// RetroAchievements (per-console hash) for name-independent matching.
pub fn set_rom_hashes(
    conn: &DbConn,
    id: i64,
    rom_crc32: &str,
    rom_hash: &str,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET rom_crc32 = ?1, rom_hash = ?2 WHERE id = ?3",
        params![rom_crc32, rom_hash, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
