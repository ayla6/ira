use super::state::SharedState;
use crate::Game;
use ira_models::GroupSelection;
use std::collections::HashSet;

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let (sort_mode, sort_descending, search, show_hidden, selected_group, games, group_members) = {
        let s = state.borrow();
        (
            s.cfg.sort_mode,
            s.cfg.sort_descending,
            s.search_query.to_lowercase(),
            s.cfg.show_hidden_games,
            s.selected_group.clone(),
            s.games.clone(),
            s.group_members.clone(),
        )
    };

    let collection_game_ids: HashSet<i64> = match &selected_group {
        GroupSelection::Collection(group_id) => {
            group_members.get(group_id).cloned().unwrap_or_default()
        }
        GroupSelection::Uncategorized => group_members.values().flatten().copied().collect(),
        _ => HashSet::new(),
    };

    let mut games: Vec<Game> = games
        .iter()
        .filter(|g| !g.hidden || show_hidden)
        .filter(|g| {
            if !search.is_empty() {
                g.name_lower.contains(&search)
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
        if sort_descending {
            ord.reverse()
        } else {
            ord
        }
    });
    games
}
