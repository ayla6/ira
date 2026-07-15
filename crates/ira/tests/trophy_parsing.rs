//! Tests for trophy XML parsing.
//!
//! Run with: cargo test --test trophy_parsing

use ira_platforms::ps4;

#[test]
fn test_parse_trop_xml_empty() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "").unwrap();
    let defs = ps4::parse_trop_xml(tmp.path());
    assert!(defs.is_empty());
}

#[test]
fn test_parse_trop_xml_nonexistent() {
    let defs = ps4::parse_trop_xml(std::path::Path::new("/nonexistent/TROP.XML"));
    assert!(defs.is_empty());
}

#[test]
fn test_parse_trop_xml_simple() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<trophyconf>\n    <trophy id=\"0\" ttype=\"P\" hidden=\"no\">\n        <name>Platinum Trophy</name>\n        <detail>Unlock all other trophies</detail>\n    </trophy>\n    <trophy id=\"1\" ttype=\"G\" hidden=\"no\">\n        <name>Gold Trophy</name>\n        <detail>Do something cool</detail>\n    </trophy>\n    <trophy id=\"2\" ttype=\"B\" hidden=\"yes\">\n        <name>Secret Bronze</name>\n        <detail>Hidden achievement</detail>\n    </trophy>\n</trophyconf>";

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), xml).unwrap();
    let defs = ps4::parse_trop_xml(tmp.path());

    assert_eq!(defs.len(), 3);
    assert_eq!(defs[0].id, "0");
    assert_eq!(defs[0].ttype, 'P');
    assert_eq!(defs[0].name, "Platinum Trophy");
    assert!(!defs[0].hidden);

    assert_eq!(defs[1].id, "1");
    assert_eq!(defs[1].ttype, 'G');
    assert_eq!(defs[1].name, "Gold Trophy");

    assert_eq!(defs[2].id, "2");
    assert_eq!(defs[2].ttype, 'B');
    assert!(defs[2].hidden);
}

#[test]
fn test_parse_user_trophies_empty() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "").unwrap();
    let unlocks = ps4::parse_user_trophies(tmp.path());
    assert!(unlocks.is_empty());
}

#[test]
fn test_parse_user_trophies_simple() {
    let xml = "<?xml version=\"1.0\"?>\n<trophies>\n    <trophy id=\"0\" unlockstate=\"true\" timestamp=\"1234567890\"></trophy>\n    <trophy id=\"1\" unlockstate=\"false\" timestamp=\"0\"></trophy>\n    <trophy id=\"2\" unlockstate=\"true\" timestamp=\"1234567891\"></trophy>\n</trophies>";

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), xml).unwrap();
    let unlocks = ps4::parse_user_trophies(tmp.path());

    assert_eq!(unlocks.len(), 3);
    assert!(unlocks.get("0").unwrap().0);
    assert_eq!(unlocks.get("0").unwrap().1, 1234567890);
    assert!(!unlocks.get("1").unwrap().0);
    assert!(unlocks.get("2").unwrap().0);
}
