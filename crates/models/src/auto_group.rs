//! Auto groups: saved rules that name a set of games out of the
//! library's metadata — a genre here, a console there, a release window,
//! a playtime floor — updated by themselves, because membership is
//! derived from the live games on every evaluation instead of stored.

use crate::find_console;
use crate::game::Game;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The dimensions a rule can look at. Value dimensions match any-of a
/// list of names; the range dimensions read the bounds instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoDimension {
    #[default]
    Genre,
    Family,
    Developer,
    Publisher,
    Console,
    Released,
    Playtime,
}

impl AutoDimension {
    /// Every dimension, in picker order: content first, then dates, then
    /// the player's own history.
    pub const ALL: &'static [AutoDimension] = &[
        AutoDimension::Genre,
        AutoDimension::Family,
        AutoDimension::Developer,
        AutoDimension::Publisher,
        AutoDimension::Console,
        AutoDimension::Released,
        AutoDimension::Playtime,
    ];

    pub fn display_label(self) -> &'static str {
        match self {
            AutoDimension::Genre => "Genre",
            AutoDimension::Family => "Series",
            AutoDimension::Developer => "Developer",
            AutoDimension::Publisher => "Publisher",
            AutoDimension::Console => "Console",
            AutoDimension::Released => "Release date",
            AutoDimension::Playtime => "Playtime",
        }
    }

    /// The value dimensions collect names; the rest read bounds.
    pub fn takes_values(self) -> bool {
        !matches!(self, AutoDimension::Released | AutoDimension::Playtime)
    }
}

/// One rule line: `dimension` decides which fields carry meaning — the
/// name lists for value dimensions, the date bounds for release ranges,
/// the hour bounds for playtime. Every field defaults so the JSON stays
/// round-trippable as dimensions evolve.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoCriterion {
    pub dimension: AutoDimension,
    #[serde(default)]
    pub values: Vec<String>,
    /// Release range, inclusive ISO dates or bare years; empty is open.
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    /// Playtime bounds in hours; `None` is open on that side.
    #[serde(default)]
    pub min_hours: Option<f64>,
    #[serde(default)]
    pub max_hours: Option<f64>,
}

impl AutoCriterion {
    /// AND across rule lines, OR across the values of one line.
    pub fn matches(&self, game: &Game, ctx: &AutoGroupContext) -> bool {
        match self.dimension {
            AutoDimension::Genre => {
                values_overlap(&self.values, ctx.genres.get(&game.db_id))
            }
            AutoDimension::Family => {
                values_overlap(&self.values, ctx.families.get(&game.db_id))
            }
            AutoDimension::Developer => {
                values_overlap(&self.values, ctx.developers.get(&game.db_id))
            }
            AutoDimension::Publisher => {
                values_overlap(&self.values, ctx.publishers.get(&game.db_id))
            }
            AutoDimension::Console => {
                let console = find_console(&game.platform_id)
                    .map(|c| c.display_name.to_string())
                    .unwrap_or_else(|| game.platform_id.clone());
                self.values
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(console.trim()))
            }
            // ISO dates and bare years compare correctly as strings —
            // `1998` sits inside `1996-01-01`..`1998-03-30` either way.
            AutoDimension::Released => {
                let date = game.release_date.trim();
                if date.is_empty() {
                    return false;
                }
                let from = self.from.trim();
                let to = self.to.trim();
                let after_from = from.is_empty() || date >= from;
                let before_to = to.is_empty() || date <= to;
                after_from && before_to
            }
            AutoDimension::Playtime => {
                let after_min = match self.min_hours {
                    Some(min) => game.playtime >= min,
                    None => true,
                };
                let before_max = match self.max_hours {
                    Some(max) => game.playtime <= max,
                    None => true,
                };
                after_min && before_max
            }
        }
    }
}

fn values_overlap(wanted: &[String], names: Option<&Vec<String>>) -> bool {
    let Some(names) = names else {
        return false;
    };
    wanted
        .iter()
        .any(|want| names.iter().any(|name| name.eq_ignore_ascii_case(want)))
}

/// The per-game names the value dimensions are matched against, straight
/// from the entity cache. A game absent from a map simply has none.
#[derive(Default, Clone)]
pub struct AutoGroupContext {
    pub genres: HashMap<i64, Vec<String>>,
    pub families: HashMap<i64, Vec<String>>,
    pub developers: HashMap<i64, Vec<String>>,
    pub publishers: HashMap<i64, Vec<String>>,
}

/// How a set of nodes combines: all of them (AND), any of them (OR),
/// or exactly one of them (XOR).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoLogic {
    #[default]
    All,
    Any,
    One,
}

impl AutoLogic {
    pub fn display_label(self) -> &'static str {
        match self {
            AutoLogic::All => "all of",
            AutoLogic::Any => "any of",
            AutoLogic::One => "exactly one of",
        }
    }

    fn combine(&self, mut results: impl Iterator<Item = bool>) -> bool {
        match self {
            // The rules are pure lookups, so short-circuiting is safe.
            AutoLogic::All => results.all(|r| r),
            AutoLogic::Any => results.any(|r| r),
            AutoLogic::One => results.filter(|r| *r).count() == 1,
        }
    }
}

/// One node of a rule tree: either a leaf criterion or a nested group of
/// nodes under its own gate. `(genre: visual novel AND console: snes)
/// OR (genre: action AND console: psx)` is `Any([All([leaf, leaf]),
/// All([leaf, leaf])])`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum AutoNode {
    #[default]
    None,
    Logic {
        logic: AutoLogic,
        nodes: Vec<AutoNode>,
    },
    Rule(AutoCriterion),
}

impl AutoNode {
    /// Empty trees match nothing, so a half-built group never swallows
    /// the whole library: an AND with no children has nothing to agree
    /// with, and an OR with no children has nothing to fire.
    pub fn matches(&self, game: &Game, ctx: &AutoGroupContext) -> bool {
        match self {
            AutoNode::None => false,
            AutoNode::Rule(criterion) => criterion.matches(game, ctx),
            AutoNode::Logic { logic, nodes } => {
                if nodes.is_empty() {
                    return false;
                }
                logic.combine(nodes.iter().map(|node| node.matches(game, ctx)))
            }
        }
    }
}

/// A named rule tree. The id is what the rest of the app files the group
/// under; the app allocates stable negative ids so they can share the
/// collection machinery without colliding with the database's own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoGroup {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub root: AutoNode,
}

impl AutoGroup {
    /// The tree decides; an empty tree matches nothing.
    pub fn matches(&self, game: &Game, ctx: &AutoGroupContext) -> bool {
        self.root.matches(game, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find_console;

    fn game() -> Game {
        Game {
            db_id: 7,
            platform_id: "snes".to_string(),
            name: "Chrono Trigger".to_string(),
            release_date: "1995-03-11".to_string(),
            release_timestamp: 794_870_400,
            playtime: 12.5,
            ..Game::default()
        }
    }

    fn leaf_criterion(dimension: AutoDimension, value: &str) -> AutoCriterion {
        AutoCriterion {
            dimension,
            values: if value.is_empty() {
                vec![]
            } else {
                vec![value.to_string()]
            },
            ..Default::default()
        }
    }

    fn ctx() -> AutoGroupContext {
        AutoGroupContext {
            genres: HashMap::from([(7, vec!["JRPG".to_string(), "Turn-based".to_string()])]),
            families: HashMap::from([(
                7,
                vec!["Chrono (Series)".to_string()],
            )]),
            ..Default::default()
        }
    }

    #[test]
    fn test_auto_values_match_any_of_the_games_names() {
        let g = game();
        let ctx = ctx();
        let rule = AutoCriterion {
            dimension: AutoDimension::Genre,
            values: vec!["Shooter".into(), "jrpg".into()],
            ..Default::default()
        };
        assert!(rule.matches(&g, &ctx), "case folds, any value matches");
        let rule = AutoCriterion {
            dimension: AutoDimension::Genre,
            values: vec!["Shooter".into()],
            ..Default::default()
        };
        assert!(!rule.matches(&g, &ctx));
        // A game the map says nothing about matches no value rule.
        let mut stranger = game();
        stranger.db_id = 99;
        assert!(!rule.matches(&stranger, &ctx));
    }

    #[test]
    fn test_auto_console_matches_the_display_name() {
        let g = game();
        let ctx = AutoGroupContext::default();
        let rule = AutoCriterion {
            dimension: AutoDimension::Console,
            values: vec!["SNES".into()],
            ..Default::default()
        };
        assert!(rule.matches(&g, &ctx));
        // The console name really is what the platform resolves to.
        assert_eq!(
            find_console("snes").map(|c| c.display_name.to_string()),
            Some("SNES".to_string())
        );
    }

    #[test]
    fn test_auto_release_range_bounds_are_inclusive() {
        let g = game();
        let ctx = AutoGroupContext::default();
        let rule = AutoCriterion {
            dimension: AutoDimension::Released,
            from: "1996-01-01".into(),
            to: "1998-03-30".into(),
            ..Default::default()
        };
        assert!(!rule.matches(&g, &ctx), "1995 predates the window");
        let rule = AutoCriterion {
            dimension: AutoDimension::Released,
            from: "1995-03-11".into(),
            to: "1998".into(),
            ..Default::default()
        };
        assert!(rule.matches(&g, &ctx), "both bounds are inclusive");
        // An open-ended side lets everything through on that side.
        let rule = AutoCriterion {
            dimension: AutoDimension::Released,
            to: "1995-12-31".into(),
            ..Default::default()
        };
        assert!(rule.matches(&g, &ctx));
        // A game with no date at all never sits in a dated window.
        let mut undated = game();
        undated.release_date.clear();
        undated.release_timestamp = 0;
        assert!(!rule.matches(&undated, &ctx));
    }

    #[test]
    fn test_auto_playtime_bounds_and_open_sides() {
        let g = game();
        let ctx = AutoGroupContext::default();
        let rule = AutoCriterion {
            dimension: AutoDimension::Playtime,
            min_hours: Some(2.0),
            max_hours: None,
            ..Default::default()
        };
        assert!(rule.matches(&g, &ctx), "12.5h is past the 2h floor");
        let rule = AutoCriterion {
            dimension: AutoDimension::Playtime,
            min_hours: Some(2.0),
            max_hours: Some(10.0),
            ..Default::default()
        };
        assert!(!rule.matches(&g, &ctx));
        // Every side open matches everything, games that never ran too.
        let open = AutoCriterion {
            dimension: AutoDimension::Playtime,
            ..Default::default()
        };
        let mut fresh = game();
        fresh.playtime = 0.0;
        assert!(open.matches(&fresh, &ctx));
    }

    #[test]
    fn test_auto_group_requires_every_rule_to_agree() {
        let g = game();
        let ctx = ctx();
        let group = AutoGroup {
            id: -1,
            name: "SNES JRPGs".into(),
            root: AutoNode::Logic {
                logic: AutoLogic::All,
                nodes: vec![
                    AutoNode::Rule(AutoCriterion {
                        dimension: AutoDimension::Console,
                        values: vec!["SNES".into()],
                        ..Default::default()
                    }),
                    AutoNode::Rule(AutoCriterion {
                        dimension: AutoDimension::Genre,
                        values: vec!["JRPG".into()],
                        ..Default::default()
                    }),
                ],
            },
        };
        assert!(group.matches(&g, &ctx));
        let refused = AutoGroup {
            root: AutoNode::Logic {
                logic: AutoLogic::All,
                nodes: vec![
                    group.root.clone(),
                    AutoNode::Rule(AutoCriterion {
                        dimension: AutoDimension::Playtime,
                        min_hours: Some(100.0),
                        ..Default::default()
                    }),
                ],
            },
            ..group.clone()
        };
        assert!(!refused.matches(&g, &ctx));
    }

    #[test]
    fn test_auto_nested_groups_cover_the_or_of_ands_shape() {
        // (genre: visual novel AND console: snes) OR (playtime: 2h+) —
        // the user's example shape, one level of nesting each side.
        let g = game();
        let ctx = ctx();
        let leaf = |dimension: AutoDimension, values: &[&str]| {
            AutoNode::Rule(AutoCriterion {
                dimension,
                values: values.iter().map(|v| v.to_string()).collect(),
                ..Default::default()
            })
        };
        let group = AutoGroup {
            id: -4,
            name: "Nested".into(),
            root: AutoNode::Logic {
                logic: AutoLogic::Any,
                nodes: vec![
                    AutoNode::Logic {
                        logic: AutoLogic::All,
                        nodes: vec![leaf(AutoDimension::Genre, &["JRPG"]), leaf(AutoDimension::Console, &["SNES"])],
                    },
                    AutoNode::Logic {
                        logic: AutoLogic::All,
                        nodes: vec![AutoNode::Rule(AutoCriterion {
                            dimension: AutoDimension::Playtime,
                            min_hours: Some(100.0),
                            ..Default::default()
                        })],
                    },
                ],
            },
        };
        assert!(group.matches(&g, &ctx), "the first OR side agrees");
        // XOR over one agreeing and one refusing side fires; the
        // empty-values leaf never matches anything.
        let xor = AutoGroup {
            id: -5,
            name: "Nested XOR".into(),
            root: AutoNode::Logic {
                logic: AutoLogic::One,
                nodes: vec![
                    AutoNode::Logic {
                        logic: AutoLogic::All,
                        nodes: vec![
                            leaf(AutoDimension::Genre, &["JRPG"]),
                            leaf(AutoDimension::Console, &["SNES"]),
                        ],
                    },
                    AutoNode::Rule(leaf_criterion(AutoDimension::Genre, "Shooter")),
                ],
            },
        };
        assert!(xor.matches(&g, &ctx), "exactly one side agrees");
        // Two agreeing sides break the "exactly one" contract.
        let two_agree = AutoGroup {
            root: AutoNode::Logic {
                logic: AutoLogic::One,
                nodes: vec![
                    AutoNode::Rule(leaf_criterion(AutoDimension::Genre, "JRPG")),
                    AutoNode::Rule(leaf_criterion(AutoDimension::Genre, "Turn-based")),
                ],
            },
            ..AutoGroup::default()
        };
        assert!(!two_agree.matches(&g, &ctx));
    }

    #[test]
    fn test_auto_empty_trees_match_nothing() {
        let g = game();
        let ctx = ctx();
        assert!(!AutoGroup::default().matches(&g, &ctx));
        let empty_and = AutoGroup {
            root: AutoNode::Logic {
                logic: AutoLogic::All,
                nodes: vec![],
            },
            ..AutoGroup::default()
        };
        assert!(!empty_and.matches(&g, &ctx));
    }

    #[test]
    fn test_auto_criteria_json_round_trips() {
        let group = AutoGroup {
            id: -3,
            name: "Retro before 2000".into(),
            root: AutoNode::Logic {
                logic: AutoLogic::All,
                nodes: vec![
                    AutoNode::Rule(AutoCriterion {
                        dimension: AutoDimension::Released,
                        to: "2000-01-01".into(),
                        ..Default::default()
                    }),
                    AutoNode::Rule(AutoCriterion {
                        dimension: AutoDimension::Playtime,
                        min_hours: Some(0.5),
                        ..Default::default()
                    }),
                ],
            },
        };
        let json = serde_json::to_string(&group).unwrap();
        let back: AutoGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, group);
    }

}
