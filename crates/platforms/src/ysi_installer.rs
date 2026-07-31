use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Metadata parsed from a YSI (YAD Simple Installer) script header.
#[derive(Debug, Clone)]
pub struct YsiMetadata {
    pub app: String,
    pub icon: String,
    pub ver: String,
    pub appurl: String,
    pub appsz: u64,
    pub arcsz: u64,
    pub ysisz: u64,
    pub picsz: u64,
    pub yadsz: u64,
    pub pvsz: u64,
    pub zstdsz: u64,
}

/// Offset of the embedded archive in the installer file.
fn archive_offset(meta: &YsiMetadata) -> u64 {
    let ysisk = meta.ysisz + 1;
    let yadsk = ysisk + meta.yadsz;
    let picsk = yadsk + meta.picsz;
    let pvsk = picsk + meta.pvsz;
    pvsk + meta.zstdsz
}

/// Parse YSI metadata from the first lines of an installer script.
/// Reads up to 700 lines (matching the robust_extract.sh approach).
pub fn parse_ysi_metadata(path: &Path) -> Result<YsiMetadata, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read installer: {e}"))?;

    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for line in content.lines().take(700) {
        // Match lines like: app="Metaphor ReFantazio" or arcsz=40428517976
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let rest = &line[eq_pos + 1..];
            // Strip surrounding quotes
            let value = if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
                rest[1..rest.len() - 1].to_string()
            } else {
                rest.trim().to_string()
            };
            // Only keep known variables
            if matches!(key.as_str(), "app" | "icon" | "ver" | "appurl" | "appsz" | "arcsz" | "ysisz" | "picsz" | "yadsz" | "pvsz" | "zstdsz") {
                vars.insert(key, value);
            }
        }
    }

    let get_num = |key: &str| -> Result<u64, String> {
        vars.get(key)
            .ok_or_else(|| format!("Missing YSI variable: {key}"))?
            .parse::<u64>()
            .map_err(|_| format!("Invalid value for {key}"))
    };

    Ok(YsiMetadata {
        app: vars.get("app").cloned().unwrap_or_default(),
        icon: vars.get("icon").cloned().unwrap_or_default(),
        ver: vars.get("ver").cloned().unwrap_or_default(),
        appurl: vars.get("appurl").cloned().unwrap_or_default(),
        appsz: get_num("appsz")?,
        arcsz: get_num("arcsz")?,
        ysisz: get_num("ysisz")?,
        picsz: get_num("picsz")?,
        yadsz: get_num("yadsz")?,
        pvsz: get_num("pvsz")?,
        zstdsz: get_num("zstdsz")?,
    })
}

/// Check if a file is a valid YSI installer by attempting to parse metadata.
pub fn is_ysi_installer(path: &Path) -> bool {
    parse_ysi_metadata(path).is_ok()
}

/// Extraction progress callback. Receives bytes extracted and total bytes.
pub type ProgressFn = Box<dyn Fn(u64, u64) + Send>;

/// Extract a YSI installer's embedded archive to a destination directory.
///
/// The archive is a `.tar.zst` embedded at the end of the script. We:
/// 1. Seek to the archive offset
/// 2. Read `arcsz` bytes
/// 3. Decompress with zstd
/// 4. Extract with tar to `dest_dir`
///
/// The `progress` callback is called periodically with (bytes_read, total_bytes).
/// The `pause` flag can be checked to pause extraction (returns true to abort).
pub fn extract_ysi_installer(
    installer_path: &Path,
    dest_dir: &Path,
    progress: Option<ProgressFn>,
    should_pause: Option<&std::sync::atomic::AtomicBool>,
    should_cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<PathBuf, String> {
    let meta = parse_ysi_metadata(installer_path)?;
    let offset = archive_offset(&meta);

    let mut file = std::fs::File::open(installer_path)
        .map_err(|e| format!("Failed to open installer: {e}"))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Failed to seek to archive: {e}"))?;

    // Read the compressed archive
    let mut compressed = Vec::with_capacity(meta.arcsz as usize);
    file.take(meta.arcsz)
        .read_to_end(&mut compressed)
        .map_err(|e| format!("Failed to read archive: {e}"))?;

    // Verify we got the right amount of data
    if compressed.len() as u64 != meta.arcsz {
        // Some installers have slightly larger archives than declared
        eprintln!(
            "YSI: archive size mismatch (declared {}, got {}), proceeding with actual size",
            meta.arcsz,
            compressed.len()
        );
    }

    let total = compressed.len() as u64;

    // Decompress zstd
    let decoder = zstd::Decoder::with_buffer(&compressed[..])
        .map_err(|e| format!("Failed to create zstd decoder: {e}"))?;

    // Extract tar stream
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create destination: {e}"))?;

    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries()
        .map_err(|e| format!("Failed to read tar entries: {e}"))?;

    let mut bytes_extracted: u64 = 0;
    let app_name = meta.app.clone();

    for entry in entries {
        // Check for cancellation
        if let Some(cancel) = should_cancel {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Extraction cancelled".to_string());
            }
        }

        // Check for pause (spin-wait)
        if let Some(pause) = should_pause {
            while pause.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if let Some(cancel) = should_cancel {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Extraction cancelled".to_string());
                    }
                }
            }
        }

        let mut entry = entry.map_err(|e| format!("Failed to read tar entry: {e}"))?;
        let size = entry.header().size().unwrap_or(0);
        let entry_path = entry.path()
            .map_err(|e| format!("Failed to get entry path: {e}"))?
            .to_path_buf();

        // Security: prevent path traversal
        if entry_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            eprintln!("YSI: skipping path traversal attempt: {:?}", entry_path);
            continue;
        }

        entry.unpack_in(dest_dir)
            .map_err(|e| format!("Failed to extract {:?}: {e}", entry_path))?;

        bytes_extracted += size;
        if let Some(ref cb) = progress {
            cb(bytes_extracted, total);
        }
    }

    if let Some(ref cb) = progress {
        cb(total, total);
    }

    let game_dir = dest_dir.join(&app_name);
    Ok(game_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ysi_metadata_basic() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"#!/usr/bin/env sh
app="Test Game"
icon="game/game/icon.png"
ver="1.0.0"
appurl=http://example.com
appsz=1000000
arcsz=500000
ysisz=100
picsz=200
yadsz=300
pvsz=400
zstdsz=500
"#).unwrap();

        let meta = parse_ysi_metadata(tmp.path()).unwrap();
        assert_eq!(meta.app, "Test Game");
        assert_eq!(meta.ver, "1.0.0");
        assert_eq!(meta.appsz, 1000000);
        assert_eq!(meta.arcsz, 500000);
    }

    #[test]
    fn test_parse_ysi_metadata_missing_var() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"#!/usr/bin/env sh
app="Test Game"
"#).unwrap();

        let result = parse_ysi_metadata(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing"));
    }

    #[test]
    fn test_archive_offset() {
        let meta = YsiMetadata {
            app: String::new(),
            icon: String::new(),
            ver: String::new(),
            appurl: String::new(),
            appsz: 0,
            arcsz: 0,
            ysisz: 100,
            picsz: 200,
            yadsz: 300,
            pvsz: 400,
            zstdsz: 500,
        };
        // ysisk = 101, yadsk = 401, picsk = 601, pvsk = 1001, offset = 1501
        assert_eq!(archive_offset(&meta), 1501);
    }

    #[test]
    fn test_is_ysi_installer_negative() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"not a script").unwrap();
        assert!(!is_ysi_installer(tmp.path()));
    }
}
