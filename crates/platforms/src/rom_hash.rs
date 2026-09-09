//! Content hashing for ROM files.

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
    let mut hasher = Md5::new();
    std::io::copy(&mut reader, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
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
}
