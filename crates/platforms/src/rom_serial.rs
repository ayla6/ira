use std::path::{Path, PathBuf};

use ira_db::DbConn;

struct DiscInfo {
    serial: Option<String>,
    title: Option<String>,
}

/// File types the disc reader can extract a serial from. Consoles whose
/// ROMs are plain cartridges (nes, snes, gba, nds, …) can never yield one,
/// so spawning the reader for them is pure waste.
const DISC_EXTENSIONS: &[&str] = &[
    "bin", "cue", "chd", "pbp", "iso", "ecm", "gcm", "cso", "rvz", "wia", "wud", "wux", "mdf",
    "img", "gz",
];

fn is_disc_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| DISC_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn disc_info_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let bin = dir.join("ira-disc-info");
    if bin.is_file() {
        Some(bin)
    } else {
        None
    }
}

fn read_disc_info(path: &Path) -> Option<DiscInfo> {
    let bin = disc_info_binary()?;
    let output = std::process::Command::new(&bin).arg(path).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();

    if stdout == "null" || stdout.is_empty() {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    Some(DiscInfo {
        serial: v
            .get("serial")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        title: v
            .get("title")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
    })
}

pub fn read_serial(path: &Path) -> Option<String> {
    read_disc_info(path).and_then(|info| info.serial)
}

pub fn read_title(path: &Path) -> Option<String> {
    read_disc_info(path).and_then(|info| info.title)
}

/// Reads a ROM's disc serial through the cache: unchanged files answer
/// from the database instead of spawning the disc reader, and files that
/// cannot hold a serial never spawn it at all. Probe results — including
/// "no serial" — are cached per file size and mtime.
pub fn read_serial_cached(conn: &DbConn, path: &Path) -> Option<String> {
    if !is_disc_extension(path) {
        return None;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return None;
    };
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let path_str = path.to_string_lossy().into_owned();

    if let Some(cached) = ira_db::lookup_rom_serial(conn, &path_str, size, mtime) {
        return (!cached.serial.is_empty()).then_some(cached.serial);
    }

    let info = read_disc_info(path);
    let serial = info.as_ref().and_then(|i| i.serial.clone());
    let title = info.and_then(|i| i.title).unwrap_or_default();
    if let Err(e) = ira_db::store_rom_serial(
        conn,
        &path_str,
        size,
        mtime,
        serial.as_deref().unwrap_or(""),
        &title,
    ) {
        eprintln!("Failed to cache disc serial: {e}");
    }
    serial
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> DbConn {
        let tmp = tempfile::tempdir().unwrap();
        let conn = ira_db::init_db(tmp.path().join("ira.db").to_str().unwrap());
        std::mem::forget(tmp);
        conn
    }

    #[test]
    fn test_read_serial_nonexistent() {
        assert!(read_serial(std::path::Path::new("/nonexistent.chd")).is_none());
    }

    #[test]
    fn test_cached_read_skips_non_disc_extensions() {
        let conn = db();
        let rom = tempfile::NamedTempFile::with_suffix(".sfc").unwrap();
        std::fs::write(rom.path(), b"cartridge data").unwrap();
        assert!(read_serial_cached(&conn, rom.path()).is_none());
    }

    #[test]
    fn test_cached_read_answers_from_cache_without_binary() {
        let conn = db();
        let rom = tempfile::NamedTempFile::with_suffix(".iso").unwrap();
        std::fs::write(rom.path(), b"disc data").unwrap();
        let meta = std::fs::metadata(rom.path()).unwrap();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        ira_db::store_rom_serial(
            &conn,
            &rom.path().to_string_lossy(),
            meta.len() as i64,
            mtime,
            "SLUS-21008",
            "KATAMARI",
        )
        .unwrap();
        assert_eq!(
            read_serial_cached(&conn, rom.path()).as_deref(),
            Some("SLUS-21008")
        );
    }
}
