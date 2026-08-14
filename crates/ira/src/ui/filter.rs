use super::state::SharedState;
use crate::Game;
use ira_models::GroupSelection;
use std::collections::HashSet;

pub fn matches_search(game: &Game, query: &str) -> bool {
    query.is_empty()
        || game.name_lower.contains(query)
        || (!game.sort_title.is_empty() && game.sort_title.to_lowercase().contains(query))
        || game.platform_id.to_lowercase().contains(query)
}

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let s = state.borrow();
    let search = s.search_query.to_lowercase();
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
                matches_search(g, &search)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_search_matches_name_sort_title_and_platform() {
        let game = Game {
            name_lower: "the elder scrolls v skyrim".to_string(),
            sort_title: "Skyrim".to_string(),
            platform_id: "ps3".to_string(),
            ..Default::default()
        };

        assert!(matches_search(&game, "elder"));
        assert!(matches_search(&game, "skyrim"));
        assert!(matches_search(&game, "ps3"));
    }
}
