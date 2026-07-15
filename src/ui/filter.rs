use crate::models::GroupSelection;
use crate::Game;
use std::collections::HashSet;
use super::state::SharedState;

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let (sort_mode, sort_descending, search, show_hidden, selected_group, games, db) = {
        let s = state.borrow();
        (
            s.cfg.sort_mode,
            s.cfg.sort_descending,
            s.search_query.to_lowercase(),
            s.cfg.show_hidden_games,
            s.selected_group.clone(),
            s.games.clone(),
            s.db.clone(),
        )
    };

    let collection_game_ids: HashSet<i64> = match &selected_group {
        GroupSelection::Collection(group_id) => {
            crate::db::get_game_ids_in_group(&db, *group_id).unwrap_or_else(|e| {
                eprintln!("Failed to load group members: {}", e);
                Vec::new()
            }).into_iter().collect()
        }
        GroupSelection::Uncategorized => {
            let groups = crate::db::get_all_groups(&db).unwrap_or_default();
            let mut ids = HashSet::new();
            for g in &groups {
                let group_ids = crate::db::get_game_ids_in_group(&db, g.id).unwrap_or_default();
                ids.extend(group_ids);
            }
            ids
        }
        _ => HashSet::new(),
    };

    let mut games: Vec<Game> = games
        .iter()
        .filter(|g| !g.hidden || show_hidden)
        .filter(|g| {
            if !search.is_empty() {
                g.name.to_lowercase().contains(&search)
            } else {
                match &selected_group {
                    GroupSelection::AllGames => true,
                    GroupSelection::Collection(_) => collection_game_ids.contains(&g.db_id),
                    GroupSelection::Uncategorized => !collection_game_ids.contains(&g.db_id),
                }
            }
        })
        .cloned()
        .collect();

    games.sort_by(|a, b| {
        let ord = sort_mode.compare(a, b);
        if sort_descending { ord.reverse() } else { ord }
    });
    games
}
