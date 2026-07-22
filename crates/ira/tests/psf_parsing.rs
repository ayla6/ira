//! Tests for PSF (param.sfo) binary parsing.
//!
//! Run with: cargo test --test psf_parsing

use ira_platforms::ps4;

#[test]
fn test_parse_psf_invalid_magic() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"NOTPSF\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
    let result = ps4::parse_psf(tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bad PSF magic"));
}

#[test]
fn test_parse_psf_empty_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"").unwrap();
    let result = ps4::parse_psf(tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too short"));
}

#[test]
fn test_psf_get_title_empty() {
    let map = std::collections::HashMap::new();
    assert_eq!(ps4::psf_get_title(&map), "");
    assert_eq!(ps4::psf_get_title_id(&map), "");
}

#[test]
fn test_parse_psf_valid() {
    let mut data = Vec::new();
    data.extend_from_slice(b"\x00PSF");
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&52u32.to_le_bytes());
    data.extend_from_slice(&67u32.to_le_bytes());
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0x0204u16.to_le_bytes());
    data.extend_from_slice(&9u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&6u16.to_le_bytes());
    data.extend_from_slice(&0x0204u16.to_le_bytes());
    data.extend_from_slice(&10u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&9u32.to_le_bytes());
    data.extend_from_slice(b"TITLE\0");
    data.extend_from_slice(b"TITLE_ID\0");
    data.extend_from_slice(b"TestGame\0");
    data.extend_from_slice(b"CUSA00001\0");

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &data).unwrap();
    let result = ps4::parse_psf(tmp.path()).unwrap();

    assert_eq!(result.len(), 2);
    match result.get("TITLE").unwrap() {
        ps4::PsfValue::Text(s) => assert_eq!(s, "TestGame"),
        _ => panic!("expected Text"),
    }
    match result.get("TITLE_ID").unwrap() {
        ps4::PsfValue::Text(s) => assert_eq!(s, "CUSA00001"),
        _ => panic!("expected Text"),
    }
    assert_eq!(ps4::psf_get_title(&result), "TestGame");
    assert_eq!(ps4::psf_get_title_id(&result), "CUSA00001");
}
