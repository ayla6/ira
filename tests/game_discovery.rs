//! Tests for shadPS4 game discovery.
//!
//! Run with: cargo test --test game_discovery

use ira::platforms::ps4;

#[test]
fn test_read_install_dirs_no_config() {
    // This should return empty if no shadPS4 config exists
    let dirs = ps4::read_install_dirs();
    // Either empty (no shadPS4 installed) or non-empty (shadPS4 installed)
    // Just make sure it doesn't panic
    let _ = dirs.len();
}
