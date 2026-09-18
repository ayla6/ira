use super::search::SearchQuery;
use super::state::SharedState;
use crate::Game;
use ira_models::{GroupSelection, SortMode};
use std::collections::{HashMap, HashSet};

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let s = state.borrow();
    let search = SearchQuery::parse(&s.search_query);
    // Publisher and developer ordering needs the credited names from
    // the cache; every other mode ignores the map.
    let entity_names = match s.cfg.sort_mode {
        SortMode::Publisher => {
            ira_db::game_entity_names(&s.db, ira_db::KIND_COMPANY, Some("is_publisher"))
                .unwrap_or_default()
        }
        SortMode::Developer => {
            ira_db::game_entity_names(&s.db, ira_db::KIND_COMPANY, Some("is_developer"))
                .unwrap_or_default()
        }
        _ => Default::default(),
    };
    filter_and_sort(
        &s.games,
        &GameFilter {
            show_hidden: s.cfg.show_hidden_games,
            search: &search,
            group: &s.selected_group,
            group_members: &s.group_members,
            derived_members: &s.derived_members,
            sort_mode: s.cfg.sort_mode,
            sort_descending: s.cfg.sort_descending,
            entity_names: &entity_names,
        },
    )
}

/// Everything the filter+sort core looks at besides the games.
pub struct GameFilter<'a> {
    pub show_hidden: bool,
    pub search: &'a SearchQuery,
    pub group: &'a GroupSelection,
    pub group_members: &'a HashMap<i64, HashSet<i64>>,
    /// The metadata group-by categories' members.
    pub derived_members: &'a HashMap<String, HashSet<i64>>,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    /// The credited publisher or developer per game when the sort mode
    /// orders by one of them.
    pub entity_names: &'a HashMap<i64, String>,
}

/// The filter+sort core shared by the desktop sidebar and big-picture,
/// so the two modes can't drift apart again: hidden handling, search,
/// group membership, and the stable sort (equal keys tiebreak on the
/// insertion id).
pub fn filter_and_sort(games: &[Game], f: &GameFilter) -> Vec<Game> {
    let GameFilter {
        show_hidden,
        search,
        group,
        group_members,
        derived_members,
        sort_mode,
        sort_descending,
        entity_names,
    } = f;
    let derived_game_ids: HashSet<i64> = match group {
        GroupSelection::Derived(name) => derived_members.get(name).cloned().unwrap_or_default(),
        _ => HashSet::new(),
    };
    let collection_game_ids: HashSet<i64> = match group {
        GroupSelection::Collection(group_id) => {
            group_members.get(group_id).cloned().unwrap_or_default()
        }
        GroupSelection::Uncategorized => group_members.values().flatten().copied().collect(),
        _ => HashSet::new(),
    };

    let mut matched: Vec<&Game> = games
        .iter()
        .filter(|g| !g.hidden || *show_hidden)
        .filter(|g| {
            if !search.is_empty() {
                search.matches(g)
            } else {
                match group {
                    GroupSelection::AllGames => true,
                    GroupSelection::Collection(_) => collection_game_ids.contains(&g.db_id),
                    GroupSelection::Uncategorized => !collection_game_ids.contains(&g.db_id),
                    GroupSelection::Derived(_) => derived_game_ids.contains(&g.db_id),
                }
            }
        })
        .collect();

    matched.sort_by(|a, b| {
        let ord = match *sort_mode {
            // The credited names live in the database, so the entity
            // orderings overlay their map here; uncredited games sort
            // after the credited ones.
            SortMode::Publisher | SortMode::Developer => entity_names
                .get(&a.db_id)
                .map(String::as_str)
                .cmp(&entity_names.get(&b.db_id).map(String::as_str))
                .then_with(|| {
                    a.sort_key()
                        .to_lowercase()
                        .cmp(&b.sort_key().to_lowercase())
                }),
            _ => sort_mode.compare(a, b).then_with(|| a.db_id.cmp(&b.db_id)),
        };
        if *sort_descending {
            ord.reverse()
        } else {
            ord
        }
    });
    matched.into_iter().cloned().collect()
}
