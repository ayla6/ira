//! Content hashing for ROM files.

use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};

/// Streams a file through MD5 and returns the lowercase hex digest. This is
/// the ROM's content identity: it lets a scan recognize a game whose file
/// reappears under a different name or path. On consoles whose
/// RetroAchievements digest covers only part of the data (NDS computes its
/// own in [`crate::nds`]) this is not the RA hash, and RA lookups with it
/// simply miss into the title-based fallback.
pub fn file_md5(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    reader_md5(&mut reader)
}

fn reader_md5(reader: &mut dyn Read) -> Option<String> {
    let mut hasher = Md5::new();
    std::io::copy(reader, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

/// [`content_md5`] plus the byte count of the same stream — the pair
/// ScreenScraper's exact search wants (`md5` + `romtaille`). The count is
/// of the inner ROM for archives, never of the container.
pub fn content_md5_and_size(path: &Path, pick: &dyn Fn(&str) -> bool) -> Option<(String, u64)> {
    let mut bytes = 0u64;
    let mut hasher = Md5::new();
    crate::archives::with_entry_reader(path, pick, |reader| {
        let mut buf = [0u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buf).unwrap_or(0);
            if read == 0 {
                break Some(());
            }
            hasher.update(&buf[..read]);
            bytes += read as u64;
        }
    })?;
    Some((format!("{:x}", hasher.finalize()), bytes))
}

/// [`file_md5`] of the ROM *inside* a `.zip`/`.7z`/`.zst` container, or of
/// the file itself for anything else. A digest of the container bytes is
/// useless for RetroAchievements — no game ever lists it — so archives are
/// hashed through their single decompressed entry, selected by `pick`.
pub fn content_md5(path: &Path, pick: &dyn Fn(&str) -> bool) -> Option<String> {
    crate::archives::with_entry_reader(path, pick, |reader| reader_md5(reader))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_md5_known_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rom.bin");

        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            file_md5(&path).as_deref(),
            Some("d41d8cd98f00b204e9800998ecf8427e")
        );

        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            file_md5(&path).as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72")
        );
    }

    #[test]
    fn test_file_md5_distinguishes_content() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("b.bin");
        std::fs::write(&a, [1u8; 4096]).unwrap();
        std::fs::write(&b, [2u8; 4096]).unwrap();
        assert_ne!(file_md5(&a), file_md5(&b));
    }

    #[test]
    fn test_file_md5_missing_file_is_none() {
        assert_eq!(file_md5(Path::new("/does/not/exist.bin")), None);
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn is_gba(name: &str) -> bool {
        name.ends_with(".gba")
    }

    #[test]
    fn test_content_md5_plain_file_equals_file_md5() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rom.gba");
        std::fs::write(&path, b"plain rom").unwrap();
        assert_eq!(content_md5(&path, &is_gba), file_md5(&path));
        assert!(content_md5(&path, &is_gba).is_some());
    }

    #[test]
    fn test_content_md5_zip_hashes_entry_not_container() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("rom.zip");
        write_zip(&archive, &[("readme.txt", b"not a rom"), ("rom.gba", b"rom inside")]);
        let plain = tmp.path().join("rom.gba");
        std::fs::write(&plain, b"rom inside").unwrap();

        let hash = content_md5(&archive, &is_gba).unwrap();
        assert_eq!(Some(hash.clone()), file_md5(&plain));
        assert_ne!(Some(hash), file_md5(&archive));
    }

    #[test]
    fn test_content_md5_zip_without_matching_entry_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("rom.zip");
        write_zip(&archive, &[("readme.txt", b"not a rom")]);
        assert_eq!(content_md5(&archive, &is_gba), None);
    }
}
