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

/// The ordering every game list shares — the grid's, so the sidebar can
/// never disagree with it: the sort mode (Publisher and Developer order
/// by the credited names from the cache, uncredited games after), ties
/// broken by insertion id, the whole thing reversed when descending.
pub(crate) fn game_compare(
    a: &Game,
    b: &Game,
    sort_mode: SortMode,
    descending: bool,
    entity_names: &HashMap<i64, String>,
) -> std::cmp::Ordering {
    let ord = match sort_mode {
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
    if descending {
        ord.reverse()
    } else {
        ord
    }
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

    matched.sort_by(|a, b| game_compare(a, b, *sort_mode, *sort_descending, entity_names));
    matched.into_iter().cloned().collect()
}

/// The category name a game lands under for a group-by dimension: the
/// console's display name, the release year, or the credited entity.
pub(crate) fn group_key(
    game: &Game,
    group_by: ira_models::GroupBy,
    entity_names: &HashMap<i64, String>,
) -> String {
    match group_by {
        ira_models::GroupBy::Console => ira_models::platform_display_name(&game.platform_id),
        ira_models::GroupBy::Year => {
            if game.release_timestamp > 0 {
                use chrono::Datelike;
                chrono::DateTime::from_timestamp(game.release_timestamp, 0)
                    .map(|date| date.year().to_string())
                    .unwrap_or_default()
            } else {
                crate::tr!("Unknown year").to_string()
            }
        }
        ira_models::GroupBy::Developer
        | ira_models::GroupBy::Publisher
        | ira_models::GroupBy::Genre
        | ira_models::GroupBy::Family => entity_names
            .get(&game.db_id)
            .cloned()
            .unwrap_or_else(|| crate::tr!("Uncategorized").to_string()),
        ira_models::GroupBy::Off => game.name.clone(),
    }
}

/// The group-by categories over `games`, in section order: years
/// newest-first with the unknowns last, every other dimension by name.
/// Members keep the input order, so callers that sorted `games` by the
/// grid's sort get section contents already in grid order.
pub(crate) fn group_categories<'a>(
    group_by: ira_models::GroupBy,
    games: &[&'a Game],
    entity_names: &HashMap<i64, String>,
) -> Vec<(String, Vec<&'a Game>)> {
    let unknown = crate::tr!("Unknown year");
    let mut categories: Vec<(String, Vec<&Game>)> = Vec::new();
    for game in games {
        let key = group_key(game, group_by, entity_names);
        match categories
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(&key))
        {
            Some((_, members)) => members.push(game),
            None => categories.push((key, vec![game])),
        }
    }
    match group_by {
        ira_models::GroupBy::Year => {
            categories.sort_by(|(a, _), (b, _)| {
                let known_a = a.as_str() != unknown;
                let known_b = b.as_str() != unknown;
                known_b.cmp(&known_a).then_with(|| b.cmp(a))
            });
        }
        _ => categories.sort_by_key(|(name, _)| name.to_lowercase()),
    }
    categories
}


#[cfg(test)]
mod tests {
    use super::game_compare;
    use crate::Game;
    use ira_models::SortMode;

    #[test]
    fn test_game_compare_matches_the_grid_and_flips() {
        let a = Game {
            db_id: 1,
            sort_title: "Alpha".into(),
            ..Game::default()
        };
        let b = Game {
            db_id: 2,
            sort_title: "Beta".into(),
            ..Game::default()
        };

        // Alphabetical ascending puts Alpha first; descending flips it.
        assert_eq!(
            game_compare(&a, &b, SortMode::Alphabetical, false, &Default::default()),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            game_compare(&a, &b, SortMode::Alphabetical, true, &Default::default()),
            std::cmp::Ordering::Greater
        );
        // Equal keys tiebreak on the insertion id, in both directions.
        let twin = Game {
            db_id: 2,
            sort_title: "Alpha".into(),
            ..Game::default()
        };
        assert_eq!(
            game_compare(&a, &twin, SortMode::Alphabetical, false, &Default::default()),
            std::cmp::Ordering::Less
        );
    }
}
