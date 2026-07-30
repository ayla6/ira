use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::detect::{find_subsequence, MAKESELF_FILESIZE_MARKER, MAKESELF_OFFSET_MARKER};

/// Split a GOG makeself/mojosetup installer into its `data.zip` component.
///
/// A GOG Linux installer is a shell script (the makeself header) followed by
/// a gzipped tar (mojosetup) followed by `data.zip` (the actual game files).
/// This function parses the header to find the offsets, then writes `data.zip`
/// to `out_dir/data.zip` and returns its path.
///
/// Port of `references/gogapi-rs/src/extract.rs:229`, adapted to use pure
/// string search instead of regex.
pub fn split_gog_installer(installer: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let mut file = fs::File::open(installer).map_err(|e| format!("open installer: {e}"))?;

    let mut head = vec![0u8; 10_240];
    let n = file
        .read(&mut head)
        .map_err(|e| format!("read header: {e}"))?;
    let head = &head[..n];
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek start: {e}"))?;

    let line_count = parse_offset_lines(head)?;
    let filesizes = parse_filesizes(head)?;

    let mut script_size: u64 = 0;
    let mut reader = BufReader::new(file);
    for _ in 0..line_count {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|e| format!("read script line: {e}"))?;
        if bytes == 0 {
            break;
        }
        script_size += bytes as u64;
    }

    let data_offset = script_size + filesizes as u64;
    reader
        .seek(SeekFrom::Start(data_offset))
        .map_err(|e| format!("seek to data: {e}"))?;

    fs::create_dir_all(out_dir).map_err(|e| format!("create out dir: {e}"))?;
    let zip_path = out_dir.join("data.zip");
    let mut zip_file =
        fs::File::create(&zip_path).map_err(|e| format!("create data.zip: {e}"))?;
    let mut buf = vec![0u8; 65536];
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|e| format!("read data: {e}"))?;
        if read == 0 {
            break;
        }
        zip_file
            .write_all(&buf[..read])
            .map_err(|e| format!("write data.zip: {e}"))?;
    }

    Ok(zip_path)
}

fn parse_offset_lines(head: &[u8]) -> Result<u64, String> {
    let marker_pos = find_subsequence(head, MAKESELF_OFFSET_MARKER)
        .ok_or_else(|| "makeself offset marker not found".to_string())?;
    let rest = &head[marker_pos + MAKESELF_OFFSET_MARKER.len()..];
    let end = rest
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .ok_or_else(|| "offset line count not terminated".to_string())?;
    let digits = &rest[..end];
    std::str::from_utf8(digits)
        .map_err(|e| format!("offset digits utf8: {e}"))?
        .parse::<u64>()
        .map_err(|e| format!("parse offset line count: {e}"))
}

fn parse_filesizes(head: &[u8]) -> Result<u64, String> {
    let marker_pos = find_subsequence(head, MAKESELF_FILESIZE_MARKER)
        .ok_or_else(|| "makeself filesizes marker not found".to_string())?;
    let rest = &head[marker_pos + MAKESELF_FILESIZE_MARKER.len()..];
    let end = rest
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .ok_or_else(|| "filesizes value not terminated".to_string())?;
    let digits = &rest[..end];
    std::str::from_utf8(digits)
        .map_err(|e| format!("filesizes digits utf8: {e}"))?
        .parse::<u64>()
        .map_err(|e| format!("parse filesizes: {e}"))
}

/// Extract `data.zip` (as produced by `split_gog_installer`) into `dest`.
///
/// GOG's zip layout uses paths like `data/noarch/game/start.sh` and
/// `meta/info`. We:
///  - Skip entries whose path contains `meta` or `scripts`.
///  - Strip leading `noarch/` and `data/` path components.
///  - Preserve Unix file permissions.
///  - Call `progress(current, total)` per file for UI status.
pub fn extract_data_zip(
    zip_path: &Path,
    dest: &Path,
    progress: impl Fn(usize, usize),
) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("open data.zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("open zip archive: {e}"))?;
    let total = archive.len();

    fs::create_dir_all(dest).map_err(|e| format!("create dest: {e}"))?;

    for i in 0..total {
        progress(i + 1, total);
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;
        let raw_name = entry.name().to_string();

        if raw_name.contains("meta") || raw_name.contains("scripts") {
            continue;
        }

        let stripped = strip_gog_prefixes(&raw_name);
        if stripped.is_empty() || stripped == "/" {
            continue;
        }

        let out_path = dest.join(stripped);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("create dir {:?}: {e}", out_path))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create parent {:?}: {e}", parent))?;
        }

        let mut out_file =
            fs::File::create(&out_path).map_err(|e| format!("create {:?}: {e}", out_path))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("write {:?}: {e}", out_path))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let perms = fs::Permissions::from_mode(mode);
            let _ = fs::set_permissions(&out_path, perms);
        }
    }

    Ok(())
}

/// Strip leading `noarch/` and `data/` components from a GOG zip entry path.
fn strip_gog_prefixes(path: &str) -> String {
    let mut p = path;
    loop {
        if let Some(rest) = p.strip_prefix("noarch/") {
            p = rest;
        } else if let Some(rest) = p.strip_prefix("data/") {
            p = rest;
        } else if let Some(rest) = p.strip_prefix("/") {
            p = rest;
        } else {
            break;
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    fn make_test_zip(path: &Path, entries: &[(&str, &[u8], bool)]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data, is_dir) in entries {
            if *is_dir {
                zip.add_directory(*name, opts).unwrap();
            } else {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn test_strip_gog_prefixes() {
        assert_eq!(strip_gog_prefixes("data/noarch/game/start.sh"), "game/start.sh");
        assert_eq!(strip_gog_prefixes("noarch/game/start.sh"), "game/start.sh");
        assert_eq!(strip_gog_prefixes("data/game/start.sh"), "game/start.sh");
        assert_eq!(strip_gog_prefixes("game/start.sh"), "game/start.sh");
        assert_eq!(strip_gog_prefixes("/data/noarch/game/"), "game/");
    }

    #[test]
    fn test_extract_data_zip_skips_meta_and_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("data.zip");
        make_test_zip(
            &zip_path,
            &[
                ("data/noarch/game/start.sh", b"#!/bin/sh\nexit 0\n", false),
                ("data/noarch/game/data.bin", b"BINARY", false),
                ("meta/info", b"metadata", false),
                ("scripts/install.sh", b"script", false),
            ],
        );

        let dest = tmp.path().join("out");
        extract_data_zip(&zip_path, &dest, |_, _| {}).unwrap();

        assert!(dest.join("game/start.sh").exists());
        assert!(dest.join("game/data.bin").exists());
        assert!(!dest.join("meta").exists());
        assert!(!dest.join("scripts").exists());
    }

    #[test]
    fn test_extract_data_zip_preserves_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("data.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        zip.start_file("data/noarch/game/start.sh", opts).unwrap();
        zip.write_all(b"#!/bin/sh\n").unwrap();
        zip.finish().unwrap();

        let dest = tmp.path().join("out");
        extract_data_zip(&zip_path, &dest, |_, _| {}).unwrap();

        #[cfg(unix)]
        {
            let perms = fs::metadata(dest.join("game/start.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(perms & 0o777, 0o755);
        }
    }

    #[test]
    fn test_extract_data_zip_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("data.zip");
        make_test_zip(
            &zip_path,
            &[
                ("data/noarch/game/", b"", true),
                ("data/noarch/game/sub/deep.txt", b"deep", false),
            ],
        );

        let dest = tmp.path().join("out");
        extract_data_zip(&zip_path, &dest, |_, _| {}).unwrap();

        assert!(dest.join("game/sub/deep.txt").exists());
    }

    #[test]
    fn test_extract_data_zip_progress_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("data.zip");
        make_test_zip(
            &zip_path,
            &[
                ("data/noarch/a.txt", b"a", false),
                ("data/noarch/b.txt", b"b", false),
                ("data/noarch/c.txt", b"c", false),
            ],
        );

        let dest = tmp.path().join("out");
        let calls = std::cell::RefCell::new(Vec::new());
        extract_data_zip(&zip_path, &dest, |cur, total| {
            calls.borrow_mut().push((cur, total));
        })
        .unwrap();

        let calls = calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], (1, 3));
        assert_eq!(calls[2], (3, 3));
    }

    #[test]
    fn test_split_gog_installer_extracts_data_zip() {
        let tmp = tempfile::tempdir().unwrap();

        let script = b"#!/bin/sh\n\
umask 077\n\
filesizes=\"12\"\n\
keep=\"n\"\n\
offset=`head -n 5 \"$0\" | wc -c | tr -d \" \"`\n\
echo done\n";
        let mojo: &[u8] = b"FAKEMOJO!!!"; // 11 bytes, but filesizes says 12
        let mojo_padded: Vec<u8> = {
            let mut v = mojo.to_vec();
            v.push(0); // pad to 12 bytes
            v
        };

        let mut data_zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut data_zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored);
            zip.start_file("data/noarch/game/start.sh", opts).unwrap();
            zip.write_all(b"#!/bin/sh\n").unwrap();
            zip.finish().unwrap();
        }

        let installer_path = tmp.path().join("gog_installer.sh");
        let mut f = fs::File::create(&installer_path).unwrap();
        f.write_all(script).unwrap();
        f.write_all(&mojo_padded).unwrap();
        f.write_all(&data_zip_bytes).unwrap();

        let out_dir = tmp.path().join("split");
        let zip_path = split_gog_installer(&installer_path, &out_dir).unwrap();

        assert!(zip_path.exists());
        assert_eq!(zip_path.file_name().unwrap(), "data.zip");

        let dest = tmp.path().join("extracted");
        extract_data_zip(&zip_path, &dest, |_, _| {}).unwrap();
        assert!(dest.join("game/start.sh").exists());
    }

    #[test]
    fn test_parse_offset_lines() {
        let head = b"offset=`head -n 519 \"$0\" | wc -c | tr -d \" \"`\n";
        assert_eq!(parse_offset_lines(head).unwrap(), 519);
    }

    #[test]
    fn test_parse_filesizes() {
        let head = b"filesizes=\"877150\"\n";
        assert_eq!(parse_filesizes(head).unwrap(), 877150);
    }
}
