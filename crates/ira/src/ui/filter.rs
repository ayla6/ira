use super::search::SearchQuery;
use super::state::SharedState;
use crate::Game;
use ira_models::{GroupSelection, SortMode};
use std::collections::{HashMap, HashSet};

pub(crate) type PlaytimeKey = (i64, Option<i64>);
pub(crate) type PlaytimeValues = HashMap<PlaytimeKey, f64>;

pub(crate) fn entity_names_for_sort(
    db: &ira_db::DbConn,
    sort_mode: SortMode,
) -> HashMap<i64, String> {
    match sort_mode {
        SortMode::Publisher => {
            ira_db::game_entity_names(db, ira_db::KIND_COMPANY, Some("is_publisher"))
                .unwrap_or_default()
        }
        SortMode::Developer => {
            ira_db::game_entity_names(db, ira_db::KIND_COMPANY, Some("is_developer"))
                .unwrap_or_default()
        }
        _ => HashMap::new(),
    }
}

pub fn filtered_games(state: &SharedState) -> Vec<Game> {
    let _span = tracing::info_span!("filtered_games").entered();
    let s = state.borrow();
    let search = SearchQuery::parse(&s.search_query);
    let sort_mode = s.cfg.sort_mode;
    // Publisher and developer ordering needs the credited names from
    // the cache; every other mode ignores the map.
    let entity_names = entity_names_for_sort(&s.db, sort_mode);
    let playtimes = if sort_mode == SortMode::HoursPlayed {
        playtimes_for_sort(&s.db, &s.playtime_links, &s.games)
    } else {
        PlaytimeValues::new()
    };
    filter_and_sort(
        &s.games,
        &GameFilter {
            show_hidden: s.cfg.show_hidden_games,
            search: &search,
            group: &s.selected_group,
            group_members: &s.group_members,
            derived_members: &s.derived_members,
            sort_mode,
            sort_descending: s.cfg.sort_descending,
            entity_names: &entity_names,
            playtimes: &playtimes,
        },
    )
}

/// Resolve the values used by an hours-played sort once per refresh. Linked
/// groups share one database total, while variants keep their own playtime
/// when they are not linked.
pub(crate) fn playtimes_for_sort(
    db: &ira_db::DbConn,
    links: &HashMap<i64, Vec<i64>>,
    games: &[Game],
) -> PlaytimeValues {
    let mut group_totals: HashMap<Vec<i64>, f64> = HashMap::new();
    games
        .iter()
        .map(|game| {
            let playtime = match links.get(&game.db_id) {
                Some(members) => *group_totals.entry(members.clone()).or_insert_with(|| {
                    members
                        .iter()
                        .filter_map(|id| {
                            ira_db::find_by_db_id(db, *id)
                                .unwrap_or_else(|e| {
                                    eprintln!("Failed to read linked game {id}: {e}");
                                    None
                                })
                        })
                        .map(|entry| entry.playtime)
                        .sum()
                }),
                None => game.playtime,
            };
            ((game.db_id, game.variant_id), playtime)
        })
        .collect()
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
    /// Display playtimes keyed by the concrete game or variant entry.
    pub playtimes: &'a PlaytimeValues,
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
    playtimes: &PlaytimeValues,
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
        SortMode::HoursPlayed => playtime_for_sort(a, playtimes)
            .partial_cmp(&playtime_for_sort(b, playtimes))
            .map(std::cmp::Ordering::reverse)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.sort_key().cmp(b.sort_key()))
            .then_with(|| a.db_id.cmp(&b.db_id)),
        _ => sort_mode.compare(a, b).then_with(|| a.db_id.cmp(&b.db_id)),
    };
    if descending {
        ord.reverse()
    } else {
        ord
    }
}

fn playtime_for_sort(game: &Game, playtimes: &PlaytimeValues) -> f64 {
    playtimes
        .get(&(game.db_id, game.variant_id))
        .copied()
        .unwrap_or(game.playtime)
}

/// The filter+sort core shared by the desktop sidebar and big-picture,
/// so the two modes can't drift apart: hidden handling, search,
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
        playtimes,
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
        game_compare(
            a,
            b,
            *sort_mode,
            *sort_descending,
            entity_names,
            playtimes,
        )
    });
    matched.into_iter().cloned().collect()
}

pub(crate) fn sort_detail(
    game: &Game,
    sort_mode: SortMode,
    playtime: f64,
    entity_names: &HashMap<i64, String>,
) -> Option<String> {
    match sort_mode {
        SortMode::Alphabetical => None,
        SortMode::Completion => (game.total_count > 0).then(|| {
            format!(
                "{}/{} ({}%)",
                game.earned_count,
                game.total_count,
                game.completion_pct() as u8
            )
        }),
        SortMode::HoursPlayed => (playtime > 0.0)
            .then(|| super::game_display::format_playtime(playtime)),
        SortMode::LastPlayed => {
            if game.last_played == 0 {
                None
            } else {
                super::helpers::local_datetime(game.last_played)
                    .map(|date| date.format("%b %-d, %Y").to_string())
            }
        }
        SortMode::ReleaseDate => {
            if !game.release_date.is_empty() {
                Some(game.release_date.clone())
            } else if game.release_timestamp > 0 {
                chrono::DateTime::from_timestamp(game.release_timestamp, 0)
                    .map(|date| date.format("%Y-%m-%d").to_string())
            } else {
                None
            }
        }
        SortMode::MetacriticScore => (game.metacritic_score >= 0)
            .then(|| game.metacritic_score.to_string()),
        SortMode::SteamReview => (game.steam_review_score >= 0)
            .then(|| format!("{}%", game.steam_review_score)),
        SortMode::Publisher | SortMode::Developer => entity_names.get(&game.db_id).cloned(),
    }
}

/// The category name a game lands under for a group-by dimension: the
/// console's display name, the release year, or the credited entity.
pub(crate) fn group_key(
    game: &Game,
    group_by: ira_models::GroupBy,
    entity_names: &HashMap<i64, String>,
) -> String {
    match group_by {
        ira_models::GroupBy::Console => game.platform_id.display_name().to_string(),
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
    use super::{game_compare, playtimes_for_sort, sort_detail};
    use crate::Game;
    use ira_models::{GameKind, SortMode, TrophySource};
    use std::collections::HashMap;

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
            game_compare(
                &a,
                &b,
                SortMode::Alphabetical,
                false,
                &Default::default(),
                &Default::default(),
            ),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            game_compare(
                &a,
                &b,
                SortMode::Alphabetical,
                true,
                &Default::default(),
                &Default::default(),
            ),
            std::cmp::Ordering::Greater
        );
        // Equal keys tiebreak on the insertion id, in both directions.
        let twin = Game {
            db_id: 2,
            sort_title: "Alpha".into(),
            ..Game::default()
        };
        assert_eq!(
            game_compare(
                &a,
                &twin,
                SortMode::Alphabetical,
                false,
                &Default::default(),
                &Default::default(),
            ),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_game_compare_hours_uses_linked_playtime() {
        let a = Game {
            db_id: 1,
            playtime: 1.0,
            ..Game::default()
        };
        let b = Game {
            db_id: 2,
            playtime: 5.0,
            ..Game::default()
        };
        let playtimes = HashMap::from([((1, None), 10.0), ((2, None), 2.0)]);

        assert_eq!(
            game_compare(
                &a,
                &b,
                SortMode::HoursPlayed,
                false,
                &Default::default(),
                &playtimes,
            ),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_playtimes_for_sort_combines_linked_database_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = ira_db::init_db(&tmp.path().join("games.db").to_string_lossy());
        let add_game = |title: &str| {
            ira_db::add_game(
                &db,
                ira_db::NewGame {
                    kind: GameKind::Steam,
                    trophy_source: TrophySource::Gse,
                    steam_id: "",
                    trophy_id: "",
                    native_id: "",
                    platform_id: "",
                    title,
                },
            )
            .unwrap()
        };
        let first = add_game("First");
        let second = add_game("Second");
        ira_db::update_field(&db, first, "playtime", &3.0_f64).unwrap();
        ira_db::update_field(&db, second, "playtime", &7.0_f64).unwrap();
        ira_db::link_games(&db, &[first, second]).unwrap();

        let links = ira_db::playtime_links(&db).unwrap();
        let mut by_game = HashMap::new();
        for (game_id, group_id) in links {
            by_game.entry(group_id).or_insert_with(Vec::new).push(game_id);
        }
        let mut links = HashMap::new();
        for members in by_game.into_values() {
            for id in &members {
                links.insert(*id, members.clone());
            }
        }
        let games = vec![
            Game {
                db_id: first,
                playtime: 3.0,
                ..Game::default()
            },
            Game {
                db_id: second,
                playtime: 7.0,
                ..Game::default()
            },
        ];

        let values = playtimes_for_sort(&db, &links, &games);

        assert_eq!(values[&(first, None)], 10.0);
        assert_eq!(values[&(second, None)], 10.0);
    }

    #[test]
    fn test_sort_detail_uses_the_selected_metadata() {
        let game = Game {
            db_id: 7,
            earned_count: 3,
            total_count: 10,
            playtime: 2.5,
            release_date: "15 Sep, 2014".into(),
            metacritic_score: 88,
            steam_review_score: 94,
            ..Game::default()
        };
        let entity_names = HashMap::from([(7, "Example Studio".to_string())]);

        assert_eq!(
            sort_detail(&game, SortMode::Completion, game.playtime, &entity_names),
            Some("3/10 (30%)".into())
        );
        assert_eq!(
            sort_detail(&game, SortMode::HoursPlayed, game.playtime, &entity_names),
            Some("2h30min".into())
        );
        assert_eq!(
            sort_detail(&game, SortMode::ReleaseDate, game.playtime, &entity_names),
            Some("15 Sep, 2014".into())
        );
        assert_eq!(
            sort_detail(&game, SortMode::MetacriticScore, game.playtime, &entity_names),
            Some("88".into())
        );
        assert_eq!(
            sort_detail(&game, SortMode::SteamReview, game.playtime, &entity_names),
            Some("94%".into())
        );
        assert_eq!(
            sort_detail(&game, SortMode::Developer, game.playtime, &entity_names),
            Some("Example Studio".into())
        );
        assert_eq!(
            sort_detail(&Game::default(), SortMode::LastPlayed, 0.0, &entity_names),
            None
        );
        assert_eq!(
            sort_detail(&Game::default(), SortMode::ReleaseDate, 0.0, &entity_names),
            None
        );
    }
}
