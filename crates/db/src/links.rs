use crate::{err, DbConn};
use rusqlite::params;

/// Games whose playtime and session history read as one: every member row
/// carries the same `group_id`, and a game belongs to at most one link.
/// Written only through [`link_games`] / [`unlink_game`]; reads merge at
/// display time, so the games' own playtime columns stay untouched.
const LINK_TABLE: &str = "game_playtime_links";

/// Drop links that no longer link anything — a group shrunk below two
/// members by unlinks or game deletions.
pub(crate) fn prune_incomplete_groups(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute(
        &format!(
            "DELETE FROM {LINK_TABLE} WHERE group_id IN (
                 SELECT group_id FROM {LINK_TABLE} GROUP BY group_id HAVING COUNT(*) < 2)"
        ),
        [],
    )
    .map_err(err)?;
    Ok(())
}

/// Link the given games together. Games already in another link leave it,
/// so the same game never counts twice by accident.
pub fn link_games(conn: &DbConn, game_ids: &[i64]) -> Result<(), String> {
    if game_ids.len() < 2 {
        return Err("A playtime link needs at least two games".to_string());
    }
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    for id in game_ids {
        tx.execute(
            &format!("DELETE FROM {LINK_TABLE} WHERE game_id = ?1"),
            params![id],
        )
        .map_err(err)?;
    }
    let next: i64 = tx
        .query_row(
            &format!("SELECT COALESCE(MAX(group_id), 0) + 1 FROM {LINK_TABLE}"),
            [],
            |row| row.get(0),
        )
        .map_err(err)?;
    for id in game_ids {
        tx.execute(
            &format!("INSERT INTO {LINK_TABLE} (game_id, group_id) VALUES (?1, ?2)"),
            params![id, next],
        )
        .map_err(err)?;
    }
    prune_incomplete_groups(&tx)?;
    tx.commit().map_err(err)?;
    Ok(())
}

/// Take `game_id` out of its link; the remaining members stay linked.
pub fn unlink_game(conn: &DbConn, game_id: i64) -> Result<(), String> {
    let c = crate::lock_db(conn)?;
    let tx = c.unchecked_transaction().map_err(err)?;
    tx.execute(
        &format!("DELETE FROM {LINK_TABLE} WHERE game_id = ?1"),
        params![game_id],
    )
    .map_err(err)?;
    prune_incomplete_groups(&tx)?;
    tx.commit().map_err(err)?;
    Ok(())
}

/// Every link membership as `(game_id, group_id)`.
pub fn playtime_links(conn: &DbConn) -> Result<Vec<(i64, i64)>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT game_id, group_id FROM {LINK_TABLE} ORDER BY game_id"
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

/// The members of `game_id`'s link, the game itself included; empty when
/// it is not linked.
pub fn link_members(conn: &DbConn, game_id: i64) -> Result<Vec<i64>, String> {
    let c = crate::lock_db(conn)?;
    let group: Option<i64> = c
        .query_row(
            &format!("SELECT group_id FROM {LINK_TABLE} WHERE game_id = ?1"),
            params![game_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(err(e)),
        })?;
    let Some(group) = group else {
        return Ok(Vec::new());
    };
    let mut stmt = c
        .prepare(&format!(
            "SELECT game_id FROM {LINK_TABLE} WHERE group_id = ?1 ORDER BY game_id"
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map(params![group], |row| row.get(0))
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

/// Each link's members with their titles, for management UIs: one inner
/// vec per link, sorted by the first member's title.
pub fn link_groups_with_titles(conn: &DbConn) -> Result<Vec<Vec<(i64, String)>>, String> {
    let c = crate::lock_db(conn)?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT group_id, l.game_id, title FROM {LINK_TABLE} l
             JOIN games ON games.id = l.game_id
             ORDER BY group_id, title COLLATE NOCASE"
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?;
    let rows = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
    // Rows arrive ordered by group, so one link's members are consecutive.
    let mut groups: Vec<Vec<(i64, String)>> = Vec::new();
    let mut current_group: Option<i64> = None;
    for (group, game_id, title) in rows {
        if current_group != Some(group) {
            groups.push(Vec::new());
            current_group = Some(group);
        }
        if let Some(members) = groups.last_mut() {
            members.push((game_id, title));
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::super::add_game;
    use super::super::init_db;
    use super::*;
    use ira_models::{GameKind, TrophySource};
    use tempfile::TempDir;

    fn setup_db() -> (DbConn, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = init_db(&db_path.to_string_lossy());
        (conn, tmp)
    }

    fn game(conn: &DbConn, title: &str) -> i64 {
        add_game(
            conn,
            GameKind::Steam,
            TrophySource::Gse,
            "",
            "",
            "",
            title,
        )
        .unwrap()
    }

    #[test]
    fn test_link_games_shares_one_group() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "Elden Ring");
        let b = game(&conn, "Elden Ring Switch");
        link_games(&conn, &[a, b]).unwrap();

        assert_eq!(link_members(&conn, a).unwrap(), vec![a, b]);
        assert_eq!(link_members(&conn, b).unwrap(), vec![a, b]);
        assert_eq!(playtime_links(&conn).unwrap().len(), 2);
    }

    #[test]
    fn test_link_needs_two_games() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "Solo");
        assert!(link_games(&conn, &[a]).is_err());
        assert!(link_members(&conn, a).unwrap().is_empty());
    }

    #[test]
    fn test_link_moves_games_out_of_older_links() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "A");
        let b = game(&conn, "B");
        let c = game(&conn, "C");
        link_games(&conn, &[a, b]).unwrap();
        // Relinking B with C must pull it out of A's link.
        link_games(&conn, &[b, c]).unwrap();

        assert!(link_members(&conn, a).unwrap().is_empty());
        assert_eq!(link_members(&conn, b).unwrap(), vec![b, c]);
        // A's abandoned single-member link is pruned.
        assert_eq!(playtime_links(&conn).unwrap().len(), 2);
    }

    #[test]
    fn test_unlink_keeps_the_rest_linked() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "A");
        let b = game(&conn, "B");
        let c = game(&conn, "C");
        link_games(&conn, &[a, b, c]).unwrap();
        unlink_game(&conn, b).unwrap();

        assert_eq!(link_members(&conn, a).unwrap(), vec![a, c]);
        assert!(link_members(&conn, b).unwrap().is_empty());
    }

    #[test]
    fn test_unlink_last_member_prunes_the_link() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "A");
        let b = game(&conn, "B");
        link_games(&conn, &[a, b]).unwrap();
        unlink_game(&conn, a).unwrap();

        assert!(link_members(&conn, b).unwrap().is_empty());
        assert!(playtime_links(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_deleting_a_game_drops_it_from_the_link() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "A");
        let b = game(&conn, "B");
        link_games(&conn, &[a, b]).unwrap();
        super::super::remove_game(&conn, a).unwrap();

        assert!(link_members(&conn, b).unwrap().is_empty());
        assert!(playtime_links(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_groups_with_titles_orders_links_and_members() {
        let (conn, _tmp) = setup_db();
        let a = game(&conn, "Alpha");
        let b = game(&conn, "Beta");
        let c = game(&conn, "Ceres");
        let d = game(&conn, "Delta");
        link_games(&conn, &[b, a]).unwrap();
        link_games(&conn, &[d, c]).unwrap();

        let groups = link_groups_with_titles(&conn).unwrap();
        assert_eq!(groups.len(), 2);
        let titles: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.iter().map(|(_, t)| t.as_str()).collect())
            .collect();
        assert!(titles.contains(&vec!["Alpha", "Beta"]));
        assert!(titles.contains(&vec!["Ceres", "Delta"]));
    }
}
