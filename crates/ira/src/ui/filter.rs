use super::search::SearchQuery;
use super::state::SharedState;
use crate::Game;
use ira_models::GroupSelection;
use std::collections::HashSet;

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let s = state.borrow();
    let search = SearchQuery::parse(&s.search_query);
    let collection_game_ids: HashSet<i64> = match &s.selected_group {
        GroupSelection::Collection(group_id) => {
            s.group_members.get(group_id).cloned().unwrap_or_default()
        }
        GroupSelection::Uncategorized => s.group_members.values().flatten().copied().collect(),
        _ => HashSet::new(),
    };

    let mut games: Vec<&Game> = s
        .games
        .iter()
        .filter(|g| !g.hidden || s.cfg.show_hidden_games)
        .filter(|g| {
            if !search.is_empty() {
                search.matches(g)
            } else {
                match &s.selected_group {
                    GroupSelection::AllGames => true,
                    GroupSelection::Collection(_) => collection_game_ids.contains(&g.db_id),
                    GroupSelection::Uncategorized => !collection_game_ids.contains(&g.db_id),
                }
            }
        })
        .collect();

    games.sort_by(|a, b| {
        let ord = s.cfg.sort_mode.compare(a, b);
        if s.cfg.sort_descending {
            ord.reverse()
        } else {
            ord
        }
    });
    games.into_iter().cloned().collect()
}
