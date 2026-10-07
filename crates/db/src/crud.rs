use crate::{err, DbConn};
use ira_models::{GameEntry, GameKind, TrophySource};
use rusqlite::params;

/// One row insert: match ids (`steam_id`, `trophy_id`), the
/// platform-native product id, and the system scope. Bundled so the
/// parameter list stays readable as id columns come and go.
pub struct NewGame<'a> {
    pub kind: GameKind,
    pub trophy_source: TrophySource,
    pub steam_id: &'a str,
    pub trophy_id: &'a str,
    pub native_id: &'a str,
    pub platform_id: &'a str,
    pub title: &'a str,
}

pub fn add_game(conn: &DbConn, game: NewGame<'_>) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    let kind = game.kind.as_str();
    let trophy_source = game.trophy_source.as_str();
    let NewGame {
        steam_id,
        trophy_id,
        native_id,
        platform_id,
        title,
        ..
    } = game;
    if !steam_id.is_empty() {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, trophy_id, native_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(steam_id, platform_id) WHERE steam_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, trophy_id, native_id, platform_id, title],
        ).map_err(err)?;
    } else {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, trophy_id, native_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(native_id, platform_id) WHERE native_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, trophy_id, native_id, platform_id, title],
        ).map_err(err)?;
    }
    Ok(c.last_insert_rowid())
}

pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET title = ?1 WHERE id = ?2", params![title, id])?;
    Ok(())
}

pub fn update_game_ids(
    conn: &DbConn,
    id: i64,
    steam_id: &str,
    trophy_id: &str,
    trophy_source: TrophySource,
    platform_id: &str,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    tx.execute(
        "UPDATE games SET steam_id = ?1, trophy_id = ?2, trophy_source = ?3, platform_id = ?4 WHERE id = ?5",
        params![steam_id, trophy_id, trophy_source.as_str(), platform_id, id],
    ).map_err(err)?;
    // The Steam identity may have moved: any earlier garnish consult
    // answered for a different store entry.
    tx.execute("DELETE FROM steam_garnish WHERE game_id = ?1", params![id])
        .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(())
}

/// Overwrite the platform-native id (title id, disc serial, emulator app
/// id) without touching the achievement keys.
pub fn update_native_id(conn: &DbConn, id: i64, native_id: &str) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET native_id = ?1 WHERE id = ?2", params![native_id, id])?;
    Ok(())
}

pub fn update_sort_title(conn: &DbConn, id: i64, sort_title: &str) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET sort_title = ?1 WHERE id = ?2", params![sort_title, id])?;
    Ok(())
}

pub fn update_game_folder(conn: &DbConn, id: i64, game_folder: &str) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET game_folder = ?1 WHERE id = ?2", params![game_folder, id])?;
    Ok(())
}

pub fn update_game_kind(conn: &DbConn, id: i64, kind: GameKind) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET kind = ?1 WHERE id = ?2", params![kind.as_str(), id])?;
    Ok(())
}

pub fn update_achievement_counts(
    conn: &DbConn,
    id: i64,
    earned: i64,
    total: i64,
    mtime: i64,
) -> Result<(), String> {
    crate::execute(
        conn,
        "UPDATE games SET cached_earned_count = ?1, cached_total_count = ?2, cached_achievement_mtime = ?3 WHERE id = ?4",
        params![earned, total, mtime, id],
    )?;
    Ok(())
}

pub fn set_manual_unmatch(conn: &DbConn, id: i64, value: bool) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET manual_unmatch = ?1 WHERE id = ?2", params![value, id])?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    crate::query_all(
        conn,
        &format!("SELECT {} FROM games ORDER BY CASE WHEN sort_title != '' THEN sort_title ELSE title END", crate::GAME_COLUMNS),
        [],
        crate::game_entry_from_row,
    )
}

/// Same rows as `load_all_games` without the title sort. List builds sort by
/// `sort_key` in Rust anyway, and maintenance passes filter in Rust, so the
/// SQLite sort is pure overhead on those paths.
pub fn load_all_games_unordered(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    crate::query_all(
        conn,
        &format!("SELECT {} FROM games", crate::GAME_COLUMNS),
        [],
        crate::game_entry_from_row,
    )
}

/// Every game of one kind, unsorted. Presence reconciles and Steam cleanup
/// only inspect their own kind; loading the whole library to filter in Rust
/// parses every row's JSON for nothing.
pub fn load_games_by_kind(conn: &DbConn, kind: GameKind) -> Result<Vec<GameEntry>, String> {
    load_games_by_kinds(conn, &[kind])
}

/// Every game of any listed kind, unsorted. Chunked so long kind lists never
/// approach the SQLite variable limit.
pub fn load_games_by_kinds(
    conn: &DbConn,
    kinds: &[GameKind],
) -> Result<Vec<GameEntry>, String> {
    if kinds.is_empty() {
        return Ok(Vec::new());
    }
    let mut games = Vec::new();
    for chunk in kinds.chunks(crate::IN_QUERY_CHUNK_SIZE) {
        let placeholders = crate::positional_placeholders(chunk.len());
        let sql = format!(
            "SELECT {} FROM games WHERE kind IN ({placeholders})",
            crate::GAME_COLUMNS
        );
        let kind_strs: Vec<&str> = chunk.iter().map(|kind| kind.as_str()).collect();
        let mut rows: Vec<GameEntry> = crate::query_all(
            conn,
            &sql,
            rusqlite::params_from_iter(kind_strs.iter()),
            crate::game_entry_from_row,
        )?;
        games.append(&mut rows);
    }
    Ok(games)
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    tx.execute("DELETE FROM games WHERE id = ?1", params![id])
        .map_err(err)?;
    // The playtime-link row follows via CASCADE; a link left with a
    // single member is no longer a link.
    crate::links::prune_incomplete_groups(&tx)?;
    tx.commit().map_err(err)?;
    Ok(())
}

/// Add playtime earned outside Ira to a game's total — hours imported from
/// another launcher or a console version of the same game. Unlike session
/// time this has no session row behind it, so it never appears in the play
/// history chart; it just raises the total.
pub fn add_playtime(conn: &DbConn, id: i64, hours: f64) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET playtime = playtime + ?1 WHERE id = ?2", params![hours, id])?;
    Ok(())
}

/// Cache of the resolved API-emulator DLL folder (Steam/GOG), empty if unknown.
pub fn set_api_dll_folder(conn: &DbConn, id: i64, folder: &str) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET api_dll_folder = ?1 WHERE id = ?2", params![folder, id])?;
    Ok(())
}

/// Cache of whether the game's UFS saves have been centralized.
pub fn set_saves_centralized(conn: &DbConn, id: i64, centralized: bool) -> Result<(), String> {
    crate::execute(conn, "UPDATE games SET saves_centralized = ?1 WHERE id = ?2", params![centralized as i64, id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::find_by_db_id;
    use super::super::init_db;
    use super::*;
    use ira_models::{GameKind, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let conn = init_db(&db_path_str);
        (conn, tmp)
    }

    #[test]
    fn test_add_game_inserts_new_game() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        assert!(id > 0);
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.title, "Test Game");
        assert_eq!(game.steam_id, "12345");
        assert_eq!(game.kind, GameKind::Steam);
    }

    #[test]
    fn test_add_playtime_accumulates_on_the_total() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        add_playtime(&conn, id, 12.5).unwrap();
        add_playtime(&conn, id, 1.5).unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert!((game.playtime - 14.0).abs() < 0.001);
    }

    #[test]
    fn test_add_game_conflict_updates_existing() {
        let (conn, _tmp) = setup_db();
        let id1 = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "",
            },
        )
        .unwrap();
        let id2 = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Updated Title",
            },
        )
        .unwrap();
        assert_eq!(id1, id2);
        let game = find_by_db_id(&conn, id1).unwrap().unwrap();
        assert_eq!(game.title, "Updated Title");
    }

    #[test]
    fn test_update_game_title() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        update_game_title(&conn, id, "New Title").unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.title, "New Title");
    }

    #[test]
    fn test_update_game_kind_preserves_game_metadata() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Wine,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        update_game_folder(&conn, id, "/games/test").unwrap();
        let launch = ira_models::GameLaunchConfig {
            exe: "/games/test/game.exe".to_string(),
            ..Default::default()
        };
        let wine = ira_models::WineConfig {
            enabled: true,
            prefix: "/games/test/prefix".to_string(),
            ..Default::default()
        };
        crate::save_game_config(&conn, id, &launch, &wine, None).unwrap();
        update_game_kind(&conn, id, GameKind::Linux).unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.kind, GameKind::Linux);
        assert_eq!(game.steam_id, "12345");
        assert_eq!(game.game_folder, "/games/test");
        assert_eq!(game.title, "Test Game");
        let (saved_launch, saved_wine, _) = crate::get_game_config(&conn, id).unwrap().unwrap();
        assert_eq!(saved_launch.exe, launch.exe);
        assert_eq!(saved_wine.prefix, wine.prefix);
        assert!(saved_wine.enabled);
    }

    #[test]
    fn test_update_game_ids() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        update_game_ids(&conn, id, "67890", "game123", TrophySource::Ra, "ps4").unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.steam_id, "67890");
        // The RA key lands in its own column; the platform-native id is
        // preserved untouched.
        assert_eq!(game.trophy_id, "game123");
        assert_eq!(game.native_id, "");
        assert_eq!(game.trophy_source, TrophySource::Ra);
        assert_eq!(game.platform_id, "ps4");
    }

    #[test]
    fn test_delete_game_removes_entry() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "12345",
                trophy_id: "",
                native_id: "",
                platform_id: "",
                title: "Test Game",
            },
        )
        .unwrap();
        remove_game(&conn, id).unwrap();
        let game = find_by_db_id(&conn, id).unwrap();
        assert!(game.is_none());
    }

    #[test]
    fn test_api_dll_folder_cache_defaults_empty() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Wine,
                trophy_source: TrophySource::Gse,
                steam_id: "",
                trophy_id: "",
                native_id: "g1",
                platform_id: "g1",
                title: "Cached Game",
            },
        )
        .unwrap();
        assert_eq!(super::super::get_api_dll_folder(&conn, id).unwrap(), "");
    }

    #[test]
    fn test_set_and_get_api_dll_folder() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Wine,
                trophy_source: TrophySource::Gse,
                steam_id: "",
                trophy_id: "",
                native_id: "g1",
                platform_id: "g1",
                title: "Cached Game",
            },
        )
        .unwrap();
        set_api_dll_folder(&conn, id, "/games/Game/bin/win64").unwrap();
        assert_eq!(
            super::super::get_api_dll_folder(&conn, id).unwrap(),
            "/games/Game/bin/win64"
        );
        set_api_dll_folder(&conn, id, "").unwrap();
        assert_eq!(super::super::get_api_dll_folder(&conn, id).unwrap(), "");
    }

    #[test]
    fn test_set_and_get_saves_centralized() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Wine,
                trophy_source: TrophySource::Gse,
                steam_id: "",
                trophy_id: "",
                native_id: "g1",
                platform_id: "g1",
                title: "Cached Game",
            },
        )
        .unwrap();
        assert!(!super::super::get_saves_centralized(&conn, id).unwrap());
        set_saves_centralized(&conn, id, true).unwrap();
        assert!(super::super::get_saves_centralized(&conn, id).unwrap());
        set_saves_centralized(&conn, id, false).unwrap();
        assert!(!super::super::get_saves_centralized(&conn, id).unwrap());
    }

    #[test]
    fn test_kind_filtered_loads_match_full_scan() {
        let (conn, _tmp) = setup_db();
        let steam_id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Steam,
                trophy_source: TrophySource::Gse,
                steam_id: "1",
                trophy_id: "",
                native_id: "1",
                platform_id: "steam",
                title: "Steam Game",
            },
        )
        .unwrap();
        let retro_id = add_game(
            &conn,
            NewGame {
                kind: GameKind::Retro,
                trophy_source: TrophySource::Ra,
                steam_id: "",
                trophy_id: "",
                native_id: "r1",
                platform_id: "snes",
                title: "Retro Game",
            },
        )
        .unwrap();
        let unordered = load_all_games_unordered(&conn).unwrap();
        assert_eq!(unordered.len(), 2);
        let steam = load_games_by_kind(&conn, GameKind::Steam).unwrap();
        assert_eq!(steam.len(), 1);
        assert_eq!(steam[0].id, steam_id);
        let both = load_games_by_kinds(&conn, &[GameKind::Steam, GameKind::Retro]).unwrap();
        assert_eq!(both.len(), 2);
        let mut ids: Vec<i64> = both.iter().map(|entry| entry.id).collect();
        ids.sort_unstable();
        let mut expected = vec![retro_id, steam_id];
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert!(load_games_by_kinds(&conn, &[]).unwrap().is_empty());
    }
}
