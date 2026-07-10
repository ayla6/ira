//! Tests for npbind.dat binary parsing.
//!
//! Run with: cargo test --test npbind_parsing

use achievement_viewer::platforms::ps4;

#[test]
fn test_parse_npbind_short_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"short").unwrap();
    let result = ps4::parse_npbind(tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too short"));
}

#[test]
fn test_parse_npbind_bad_magic() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut data = vec![0u8; 0x90];
    // Wrong magic
    data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    std::fs::write(tmp.path(), &data).unwrap();
    let result = ps4::parse_npbind(tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bad npbind magic"));
}
