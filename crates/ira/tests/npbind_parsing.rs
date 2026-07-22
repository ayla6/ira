//! Tests for npbind.dat binary parsing.
//!
//! Run with: cargo test --test npbind_parsing

use ira_platforms::ps4;

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
    data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    std::fs::write(tmp.path(), &data).unwrap();
    let result = ps4::parse_npbind(tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bad npbind magic"));
}

#[test]
fn test_parse_npbind_valid() {
    let mut data = vec![0u8; 0x80];
    data[0..4].copy_from_slice(&[0xD2, 0x94, 0xA0, 0x18]);
    data[0x18..0x20].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    data.extend_from_slice(&[0x00, 0x10, 0x00, 0x0D]);
    data.extend_from_slice(b"NPWR11866_00\x00");
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    data.extend(std::iter::repeat_n(0u8, 0x98));

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &data).unwrap();
    let result = ps4::parse_npbind(tmp.path()).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0], "NPWR11866_00");
}
