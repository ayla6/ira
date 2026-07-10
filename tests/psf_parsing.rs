//! Tests for PSF (param.sfo) binary parsing.
//!
//! Run with: cargo test --test psf_parsing

use achievement_viewer::platforms::ps4;

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
