#[test]
fn test_open_read_only_never_creates_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing/ira.db");
    assert!(ira_db::open_read_only(&path).is_err());
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn test_open_read_only_reads_library_and_rejects_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ira.db");
    let writable = ira_db::init_db(path.to_str().unwrap());
    let read_only = ira_db::open_read_only(&path).unwrap();
    assert!(ira_db::load_all_games_unordered(&read_only)
        .unwrap()
        .is_empty());
    let connection = read_only.get().unwrap();
    assert!(connection.execute("DELETE FROM games", []).is_err());
    drop(writable);
}
