//! Tests for playtime parsing.
//!
//! Run with: cargo test --test playtime_parsing

use ira::platforms::ps4;

#[test]
fn test_parse_playtime_hms() {
    let hours = ps4::parse_playtime("1:30:00");
    assert!((hours - 1.5).abs() < 0.001);
}

#[test]
fn test_parse_playtime_zero() {
    let hours = ps4::parse_playtime("0:00:00");
    assert!((hours - 0.0).abs() < 0.001);
}

#[test]
fn test_parse_playtime_ms() {
    let hours = ps4::parse_playtime("30:00");
    assert!((hours - 0.5).abs() < 0.001);
}

#[test]
fn test_parse_playtime_large() {
    let hours = ps4::parse_playtime("100:00:00");
    assert!((hours - 100.0).abs() < 0.001);
}

#[test]
fn test_parse_playtime_invalid() {
    let hours = ps4::parse_playtime("not a time");
    assert!((hours - 0.0).abs() < 0.001);
}

#[test]
fn test_parse_playtime_empty() {
    let hours = ps4::parse_playtime("");
    assert!((hours - 0.0).abs() < 0.001);
}

#[test]
fn test_parse_playtime_seconds() {
    let hours = ps4::parse_playtime("0:00:30");
    // 30 seconds = 30/3600 hours
    assert!((hours - (30.0 / 3600.0)).abs() < 0.001);
}
