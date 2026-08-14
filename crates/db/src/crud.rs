use crate::DbConn;
use ira_models::{GameEntry, GameKind, TrophySource};
use rusqlite::params;

pub fn add_game(
    conn: &DbConn,
    kind: GameKind,
    trophy_source: TrophySource,
    steam_id: &str,
    game_id: &str,
    platform_id: &str,
    title: &str,
) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    let kind = kind.as_str();
    let trophy_source = trophy_source.as_str();
    if !steam_id.is_empty() {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(steam_id) WHERE steam_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, game_id, platform_id, title],
        ).map_err(|e| e.to_string())?;
    } else {
        c.execute(
            "INSERT INTO games (kind, trophy_source, steam_id, game_id, platform_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(game_id, platform_id) WHERE game_id != '' DO UPDATE SET title = excluded.title WHERE games.title = '' AND excluded.title != ''",
            params![kind, trophy_source, steam_id, game_id, platform_id, title],
        ).map_err(|e| e.to_string())?;
    }
    Ok(c.last_insert_rowid())
}

pub fn update_game_title(conn: &DbConn, id: i64, title: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET title = ?1 WHERE id = ?2",
        params![title, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_game_ids(
    conn: &DbConn,
    id: i64,
    steam_id: &str,
    game_id: &str,
    trophy_source: TrophySource,
    platform_id: &str,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET steam_id = ?1, game_id = ?2, trophy_source = ?3, platform_id = ?4 WHERE id = ?5",
        params![steam_id, game_id, trophy_source.as_str(), platform_id, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_sort_title(conn: &DbConn, id: i64, sort_title: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET sort_title = ?1 WHERE id = ?2",
        params![sort_title, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_game_folder(conn: &DbConn, id: i64, game_folder: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET game_folder = ?1 WHERE id = ?2",
        params![game_folder, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_game_kind(conn: &DbConn, id: i64, kind: GameKind) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET kind = ?1 WHERE id = ?2",
        params![kind.as_str(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_achievement_counts(
    conn: &DbConn,
    id: i64,
    earned: i64,
    total: i64,
    mtime: i64,
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET cached_earned_count = ?1, cached_total_count = ?2, cached_achievement_mtime = ?3 WHERE id = ?4",
        params![earned, total, mtime, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_manual_unmatch(conn: &DbConn, id: i64, value: bool) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET manual_unmatch = ?1 WHERE id = ?2",
        params![value, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_all_games(conn: &DbConn) -> Result<Vec<GameEntry>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c.prepare(&format!("SELECT {} FROM games ORDER BY CASE WHEN sort_title != '' THEN sort_title ELSE title END", crate::GAME_COLUMNS))
        .map_err(|e| e.to_string())?;
    let entries = stmt
        .query_map([], crate::game_entry_from_row)
        .map_err(|e| e.to_string())?;

    entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn remove_game(conn: &DbConn, id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute("DELETE FROM games WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Merge duplicate game rows into `canonical_id` while keeping their related data.
pub fn merge_duplicate_games(
    conn: &DbConn,
    canonical_id: i64,
    duplicate_ids: &[i64],
) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;

    for &duplicate_id in duplicate_ids {
        if duplicate_id == canonical_id {
            continue;
        }
        tx.execute(
            "UPDATE games
             SET title = CASE WHEN title = '' THEN (SELECT title FROM games WHERE id = ?2) ELSE title END,
                  sort_title = CASE WHEN sort_title = '' THEN (SELECT sort_title FROM games WHERE id = ?2) ELSE sort_title END,
                  hidden = MAX(hidden, (SELECT hidden FROM games WHERE id = ?2)),
                  logo_position = CASE WHEN logo_position = 'bottom-left' THEN (SELECT logo_position FROM games WHERE id = ?2) ELSE logo_position END,
                  logo_size = CASE WHEN logo_size = 50 THEN (SELECT logo_size FROM games WHERE id = ?2) ELSE logo_size END,
                  playtime = playtime + (SELECT playtime FROM games WHERE id = ?2),
                  last_played = MAX(last_played, (SELECT last_played FROM games WHERE id = ?2))
             WHERE id = ?1",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO game_groups (game_id, group_id)
             SELECT ?1, group_id FROM game_groups WHERE game_id = ?2",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM game_groups WHERE game_id = ?1",
            params![duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE game_variants SET game_id = ?1 WHERE game_id = ?2",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE play_sessions SET game_id = ?1 WHERE game_id = ?2",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE game_discs SET game_id = ?1 WHERE game_id = ?2",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE game_configs SET game_id = ?1
             WHERE game_id = ?2 AND NOT EXISTS (
                 SELECT 1 FROM game_configs WHERE game_id = ?1
             )",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM game_configs WHERE game_id = ?1",
            params![duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE game_default_variant SET game_id = ?1
             WHERE game_id = ?2 AND NOT EXISTS (
                 SELECT 1 FROM game_default_variant WHERE game_id = ?1
             )",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM game_default_variant WHERE game_id = ?1",
            params![duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE game_default_disc SET game_id = ?1
             WHERE game_id = ?2 AND NOT EXISTS (
                 SELECT 1 FROM game_default_disc WHERE game_id = ?1
             )",
            params![canonical_id, duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM game_default_disc WHERE game_id = ?1",
            params![duplicate_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM games WHERE id = ?1", params![duplicate_id])
            .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Cache of the resolved API-emulator DLL folder (Steam/GOG), empty if unknown.
pub fn set_api_dll_folder(conn: &DbConn, id: i64, folder: &str) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET api_dll_folder = ?1 WHERE id = ?2",
        params![folder, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Cache of whether the game's UFS saves have been centralized.
pub fn set_saves_centralized(conn: &DbConn, id: i64, centralized: bool) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "UPDATE games SET saves_centralized = ?1 WHERE id = ?2",
        params![centralized as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::find_by_db_id;
    use super::super::init_db;
    use super::super::{
        add_disc, add_game_to_group, create_group, get_all_groups, get_discs,
        get_sessions_for_game, record_session, update_field,
    };
    use super::*;
    use ira_models::{GameDisc, GameKind, TrophySource};
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
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Test Game",
        )
        .unwrap();
        assert!(id > 0);
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.title, "Test Game");
        assert_eq!(game.steam_id, "12345");
        assert_eq!(game.kind, GameKind::Steam);
    }

    #[test]
    fn test_add_game_conflict_updates_existing() {
        let (conn, _tmp) = setup_db();
        let id1 = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "",
        )
        .unwrap();
        let id2 = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Updated Title",
        )
        .unwrap();
        assert_eq!(id1, id2);
        let game = find_by_db_id(&conn, id1).unwrap().unwrap();
        assert_eq!(game.title, "Updated Title");
    }

    #[test]
    fn test_merge_duplicate_games_preserves_related_data() {
        let (conn, _tmp) = setup_db();
        let canonical = add_game(
            &conn,
            GameKind::Retro,
            TrophySource::Empty,
            "",
            "canonical",
            "saturn",
            "Canonical",
        )
        .unwrap();
        let duplicate = add_game(
            &conn,
            GameKind::Retro,
            TrophySource::Empty,
            "",
            "duplicate",
            "saturn",
            "Duplicate",
        )
        .unwrap();
        let group = create_group(&conn, "Favorites").unwrap();
        add_game_to_group(&conn, duplicate, group).unwrap();
        update_field(&conn, duplicate, "playtime", &0.1_f64).unwrap();
        record_session(&conn, duplicate, None, 1000, 1301).unwrap();
        add_disc(
            &conn,
            &GameDisc {
                id: 0,
                game_id: duplicate,
                disc_number: 2,
                rom_path: "disc2.chd".to_string(),
                label: "Disc 2".to_string(),
            },
        )
        .unwrap();

        merge_duplicate_games(&conn, canonical, &[duplicate]).unwrap();

        assert!(find_by_db_id(&conn, duplicate).unwrap().is_none());
        assert_eq!(get_discs(&conn, canonical).unwrap().len(), 1);
        assert_eq!(get_all_groups(&conn).unwrap()[0].id, group);
        assert_eq!(
            get_sessions_for_game(&conn, canonical, None).unwrap().len(),
            1
        );
        assert!((find_by_db_id(&conn, canonical).unwrap().unwrap().playtime - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_update_game_title() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Test Game",
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
            GameKind::Wine,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Test Game",
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
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Test Game",
        )
        .unwrap();
        update_game_ids(&conn, id, "67890", "game123", TrophySource::Ra, "ps4").unwrap();
        let game = find_by_db_id(&conn, id).unwrap().unwrap();
        assert_eq!(game.steam_id, "67890");
        assert_eq!(game.game_id, "game123");
        assert_eq!(game.trophy_source, TrophySource::Ra);
        assert_eq!(game.platform_id, "ps4");
    }

    #[test]
    fn test_delete_game_removes_entry() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Steam,
            TrophySource::Gse,
            "12345",
            "",
            "",
            "Test Game",
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
            GameKind::Wine,
            TrophySource::Gse,
            "",
            "g1",
            "g1",
            "Cached Game",
        )
        .unwrap();
        assert_eq!(super::super::get_api_dll_folder(&conn, id).unwrap(), "");
    }

    #[test]
    fn test_set_and_get_api_dll_folder() {
        let (conn, _tmp) = setup_db();
        let id = add_game(
            &conn,
            GameKind::Wine,
            TrophySource::Gse,
            "",
            "g1",
            "g1",
            "Cached Game",
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
            GameKind::Wine,
            TrophySource::Gse,
            "",
            "g1",
            "g1",
            "Cached Game",
        )
        .unwrap();
        assert!(!super::super::get_saves_centralized(&conn, id).unwrap());
        set_saves_centralized(&conn, id, true).unwrap();
        assert!(super::super::get_saves_centralized(&conn, id).unwrap());
        set_saves_centralized(&conn, id, false).unwrap();
        assert!(!super::super::get_saves_centralized(&conn, id).unwrap());
    }
}
