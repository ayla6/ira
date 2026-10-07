use crate::{err, DbConn};
use ira_models::GameDisc;
use rusqlite::params;

/// Returns all disc rom_paths for retro games on the given platform.
/// Used to check whether a scanned ROM is already known (in the DB)
/// without spawning `ira-disc-info` subprocesses.
pub fn get_disc_paths_for_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    Ok(get_disc_owners_for_platform(conn, platform_id)?
        .into_keys()
        .collect())
}

/// Returns each known disc path and the game row it belongs to.
pub fn get_disc_owners_for_platform(
    conn: &DbConn,
    platform_id: &str,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let rows: Vec<(String, i64)> = crate::query_all(
        conn,
        &format!(
            "SELECT gd.rom_path, gd.game_id FROM game_discs gd
         JOIN games g ON gd.game_id = g.id
         WHERE g.kind = '{}' AND g.platform_id = ?1",
            ira_models::GameKind::Retro.as_str()
        ),
        params![platform_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(rows.into_iter().collect())
}

pub fn get_discs(conn: &DbConn, game_id: i64) -> Result<Vec<GameDisc>, String> {
    crate::query_all(
        conn,
        "SELECT id, game_id, disc_number, rom_path, label FROM game_discs WHERE game_id = ?1 ORDER BY disc_number",
        params![game_id],
        |row| {
            Ok(GameDisc {
                id: row.get(0)?,
                game_id: row.get(1)?,
                disc_number: row.get(2)?,
                rom_path: row.get(3)?,
                label: row.get(4)?,
            })
        },
    )
}

/// Disc rows for every listed game in one query per 500-id chunk, grouped by
/// game id. The stale-ROM cleanup asked per game; this is the same rows for
/// one round-trip.
pub fn get_discs_for_games(
    conn: &DbConn,
    game_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<GameDisc>>, String> {
    let mut grouped: std::collections::HashMap<i64, Vec<GameDisc>> =
        std::collections::HashMap::new();
    if game_ids.is_empty() {
        return Ok(grouped);
    }
    for chunk in game_ids.chunks(crate::IN_QUERY_CHUNK_SIZE) {
        let placeholders = crate::positional_placeholders(chunk.len());
        let sql = format!(
            "SELECT id, game_id, disc_number, rom_path, label FROM game_discs WHERE game_id IN ({placeholders}) ORDER BY game_id, disc_number"
        );
        let rows: Vec<GameDisc> = crate::query_all(
            conn,
            &sql,
            rusqlite::params_from_iter(chunk.iter()),
            |row| {
                Ok(GameDisc {
                    id: row.get(0)?,
                    game_id: row.get(1)?,
                    disc_number: row.get(2)?,
                    rom_path: row.get(3)?,
                    label: row.get(4)?,
                })
            },
        )?;
        for disc in rows {
            grouped.entry(disc.game_id).or_default().push(disc);
        }
    }
    Ok(grouped)
}

pub fn add_disc(conn: &DbConn, disc: &GameDisc) -> Result<i64, String> {
    let c = crate::lock_db(conn)?;
    c.execute(
        "INSERT INTO game_discs (game_id, disc_number, rom_path, label) VALUES (?1, ?2, ?3, ?4)",
        params![disc.game_id, disc.disc_number, disc.rom_path, disc.label],
    )
    .map_err(err)?;
    Ok(c.last_insert_rowid())
}

pub fn delete_discs(conn: &DbConn, game_id: i64) -> Result<(), String> {
    crate::execute(conn, "DELETE FROM game_discs WHERE game_id = ?1", params![game_id])?;
    Ok(())
}

pub fn get_default_disc(conn: &DbConn, game_id: i64) -> Result<Option<i64>, String> {
    crate::query_optional_scalar(
        conn,
        "SELECT disc_id FROM game_default_disc WHERE game_id = ?1",
        params![game_id],
    )
}

pub fn set_default_disc(conn: &DbConn, game_id: i64, disc_id: Option<i64>) -> Result<(), String> {
    if let Some(did) = disc_id {
        crate::execute(
            conn,
            "INSERT INTO game_default_disc (game_id, disc_id) VALUES (?1, ?2)
             ON CONFLICT(game_id) DO UPDATE SET disc_id = excluded.disc_id",
            params![game_id, did],
        )?;
    } else {
        crate::execute(conn, "DELETE FROM game_default_disc WHERE game_id = ?1", params![game_id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_discs_for_games_matches_single_queries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = crate::init_db(&tmp.path().join("test.db").to_string_lossy());
        let game_a = crate::add_game(
            &conn,
            crate::NewGame {
                kind: ira_models::GameKind::Retro,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "a",
                platform_id: "psx",
                title: "A",
            },
        )
        .unwrap();
        let game_b = crate::add_game(
            &conn,
            crate::NewGame {
                kind: ira_models::GameKind::Retro,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "b",
                platform_id: "psx",
                title: "B",
            },
        )
        .unwrap();
        for (game_id, path) in [(game_a, "a1.bin"), (game_a, "a2.bin"), (game_b, "b1.bin")] {
            add_disc(
                &conn,
                &ira_models::GameDisc {
                    id: 0,
                    game_id,
                    disc_number: 1,
                    rom_path: path.to_string(),
                    label: "Disc 1".to_string(),
                },
            )
            .unwrap();
        }

        let grouped = get_discs_for_games(&conn, &[game_a, game_b, 99999]).unwrap();
        assert_eq!(grouped[&game_a].len(), 2);
        assert_eq!(grouped[&game_b].len(), 1);
        assert!(!grouped.contains_key(&99999));
        assert_eq!(get_discs(&conn, game_a).unwrap().len(), 2);
        assert!(get_discs_for_games(&conn, &[]).unwrap().is_empty());
    }
}
