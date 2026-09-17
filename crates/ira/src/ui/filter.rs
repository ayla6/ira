use super::search::SearchQuery;
use super::state::SharedState;
use crate::Game;
use ira_models::{GroupSelection, SortMode};
use std::collections::{HashMap, HashSet};

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let s = state.borrow();
    let search = SearchQuery::parse(&s.search_query);
    filter_and_sort(
        &s.games,
        s.cfg.show_hidden_games,
        &search,
        &s.selected_group,
        &s.group_members,
        s.cfg.sort_mode,
        s.cfg.sort_descending,
    )
}

/// The filter+sort core shared by the desktop sidebar and big-picture,
/// so the two modes can't drift apart again: hidden handling, search,
/// group membership, and the stable sort (equal keys tiebreak on the
/// insertion id).
pub fn filter_and_sort(
    games: &[Game],
    show_hidden: bool,
    search: &SearchQuery,
    group: &GroupSelection,
    group_members: &HashMap<i64, HashSet<i64>>,
    sort_mode: SortMode,
    sort_descending: bool,
) -> Vec<Game> {
    let collection_game_ids: HashSet<i64> = match group {
        GroupSelection::Collection(group_id) => {
            group_members.get(group_id).cloned().unwrap_or_default()
        }
        GroupSelection::Uncategorized => group_members.values().flatten().copied().collect(),
        _ => HashSet::new(),
    };

    let mut matched: Vec<&Game> = games
        .iter()
        .filter(|g| !g.hidden || show_hidden)
        .filter(|g| {
            if !search.is_empty() {
                search.matches(g)
            } else {
                match group {
                    GroupSelection::AllGames => true,
                    GroupSelection::Collection(_) => collection_game_ids.contains(&g.db_id),
                    GroupSelection::Uncategorized => !collection_game_ids.contains(&g.db_id),
                }
            }
        })
        .collect();

    matched.sort_by(|a, b| {
        let ord = sort_mode.compare(a, b).then_with(|| a.db_id.cmp(&b.db_id));
        if sort_descending {
            ord.reverse()
        } else {
            ord
        }
    });
    matched.into_iter().cloned().collect()
}
