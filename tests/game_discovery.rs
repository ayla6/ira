//! Tests for shadPS4 game discovery.
//!
//! Run with: cargo test --test game_discovery

use achievement_viewer::platforms::ps4;

#[test]
fn test_read_install_dirs_no_config() {
    // This should return empty if no shadPS4 config exists
    let dirs = ps4::read_install_dirs();
    // Either empty (no shadPS4 installed) or non-empty (shadPS4 installed)
    // Just make sure it doesn't panic
    let _ = dirs.len();
}

#[test]
fn test_serial_to_lutris_id_stable() {
    let id1 = ps4::serial_to_lutris_id("CUSA00001");
    let id2 = ps4::serial_to_lutris_id("CUSA00001");
    assert_eq!(id1, id2, "same serial should produce same lutris_id");
}

#[test]
fn test_serial_to_lutris_id_different() {
    let id1 = ps4::serial_to_lutris_id("CUSA00001");
    let id2 = ps4::serial_to_lutris_id("CUSA00002");
    assert_ne!(id1, id2, "different serials should produce different lutris_ids");
}

#[test]
fn test_serial_to_lutris_id_negative() {
    let id = ps4::serial_to_lutris_id("CUSA12345");
    assert!(id <= -2_000_000, "synthetic lutris_id should be in negative range");
}

#[test]
fn test_is_shadps4_id() {
    let id = ps4::serial_to_lutris_id("CUSA12345");
    assert!(ps4::is_shadps4_id(id));
    assert!(!ps4::is_shadps4_id(0));
    assert!(!ps4::is_shadps4_id(100));
}
