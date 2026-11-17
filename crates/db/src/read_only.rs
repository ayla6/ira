use crate::DbConn;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;
use std::{path::Path, time::Duration};

/// Opens an existing library without creating tables or changing its contents.
pub fn open_read_only(path: &Path) -> Result<DbConn, String> {
    if !path.is_file() {
        return Err(format!(
            "Library database does not exist: {}",
            path.display()
        ));
    }
    let manager = SqliteConnectionManager::file(path)
        .with_flags(OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX);
    r2d2::Pool::builder()
        .max_size(2)
        .min_idle(Some(1))
        .connection_timeout(Duration::from_secs(1))
        .build(manager)
        .map_err(|error| format!("Open read-only library: {error}"))
}
