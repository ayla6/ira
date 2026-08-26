use crate::{lock_db, DbConn};
use rusqlite::params;

/// Creates the disc-serial cache table; rows are keyed by ROM path and
/// validated against the file's size and mtime on lookup.
pub fn create_rom_serials_table(conn: &DbConn) {
    let c = lock_db(conn).expect("db lock");
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS rom_serials (
            rom_path TEXT PRIMARY KEY,
            size INTEGER NOT NULL,
            mtime INTEGER NOT NULL,
            serial TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT ''
        );",
    )
    .expect("create rom_serials table");
}

/// A disc serial cached for one ROM file version.
pub struct RomSerial {
    pub serial: String,
    pub title: String,
}

/// Returns the cached serial for `path` when the file still has the same
/// size and mtime, so rescans never re-run the disc reader on unchanged
/// files. `None` means "not cached", including when a previous probe found
/// no serial.
pub fn lookup_rom_serial(conn: &DbConn, path: &str, size: i64, mtime: i64) -> Option<RomSerial> {
    let c = lock_db(conn).ok()?;
    c.query_row(
        "SELECT serial, title FROM rom_serials
         WHERE rom_path = ?1 AND size = ?2 AND mtime = ?3",
        params![path, size, mtime],
        |row| {
            Ok(RomSerial {
                serial: row.get(0)?,
                title: row.get(1)?,
            })
        },
    )
    .ok()
}

/// Caches the probe result for one ROM file version; an empty serial means
/// the file was probed and holds none, which is cached too.
pub fn store_rom_serial(
    conn: &DbConn,
    path: &str,
    size: i64,
    mtime: i64,
    serial: &str,
    title: &str,
) -> Result<(), String> {
    let c = lock_db(conn)?;
    c.execute(
        "INSERT INTO rom_serials (rom_path, size, mtime, serial, title)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(rom_path) DO UPDATE SET
             size = excluded.size, mtime = excluded.mtime,
             serial = excluded.serial, title = excluded.title",
        params![path, size, mtime, serial, title],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> DbConn {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::init_db(tmp.path().join("ira.db").to_str().unwrap());
        std::mem::forget(tmp);
        conn
    }

    #[test]
    fn test_lookup_requires_matching_size_and_mtime() {
        let conn = db();
        store_rom_serial(&conn, "/roms/game.iso", 100, 5, "SLUS-1", "G").unwrap();

        let hit = lookup_rom_serial(&conn, "/roms/game.iso", 100, 5).unwrap();
        assert_eq!(hit.serial, "SLUS-1");
        assert!(lookup_rom_serial(&conn, "/roms/game.iso", 101, 5).is_none());
        assert!(lookup_rom_serial(&conn, "/roms/game.iso", 100, 6).is_none());
        assert!(lookup_rom_serial(&conn, "/roms/other.iso", 100, 5).is_none());
    }

    #[test]
    fn test_store_overwrites_previous_version() {
        let conn = db();
        store_rom_serial(&conn, "/roms/game.iso", 100, 5, "OLD", "").unwrap();
        store_rom_serial(&conn, "/roms/game.iso", 200, 9, "NEW", "T").unwrap();
        let hit = lookup_rom_serial(&conn, "/roms/game.iso", 200, 9).unwrap();
        assert_eq!(hit.serial, "NEW");
        assert!(lookup_rom_serial(&conn, "/roms/game.iso", 100, 5).is_none());
    }

    #[test]
    fn test_store_caches_missing_serial() {
        let conn = db();
        store_rom_serial(&conn, "/roms/game.iso", 1, 1, "", "").unwrap();
        let hit = lookup_rom_serial(&conn, "/roms/game.iso", 1, 1).unwrap();
        assert_eq!(hit.serial, "");
    }
}
