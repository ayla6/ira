//! Auto groups in the running app. The stored rules are re-evaluated
//! against the live games on every sidebar or grid rebuild — that is the
//! whole "updates by itself" contract: a fresh match, a play session, or
//! an edited title reshapes membership at the next look, with no
//! bookkeeping anywhere.
//!
//! Identity: the database hands out positive ids, the user groups' id
//! space. In memory every auto group runs under its *negated* id, so the
//! two share the collection machinery (selection, filtering, collapse)
//! without ever colliding.

use super::state::SharedState;
use ira_models::{AutoDimension, AutoGroup, AutoGroupContext, AutoNode};
use std::collections::{HashMap, HashSet};

/// The stored groups, ids negated for in-memory use. Loading is
/// best-effort: a read failure leaves the app without auto groups rather
/// than without a library.
pub(crate) fn load_auto_groups(db: &ira_db::DbConn) -> Vec<AutoGroup> {
    match ira_db::get_all_auto_groups(db) {
        Ok(groups) => groups
            .into_iter()
            .map(|mut group| {
                group.id = -group.id;
                group
            })
            .collect(),
        Err(e) => {
            eprintln!("Failed to load auto groups: {e}");
            Vec::new()
        }
    }
}

/// The negated id an auto group runs under in memory.
pub(crate) fn to_memory_id(db_id: i64) -> i64 {
    -db_id
}

/// The database id behind a memory id, for writes.
pub(crate) fn to_db_id(memory_id: i64) -> i64 {
    -memory_id
}

/// Re-evaluate every rule over every game and fold the memberships into
/// `group_members` under the negated ids. Cheap: per game per rule it is
/// a handful of hash lookups, and the entity maps are only queried when
/// some rule actually reads that dimension.
pub(crate) fn refresh_auto_members(state: &SharedState) {
    let (auto_groups, games) = {
        let s = state.borrow();
        (s.auto_groups.clone(), s.games.clone())
    };
    if auto_groups.is_empty() {
        state.borrow_mut().group_members.retain(|k, _| *k >= 0);
        return;
    }
    // A dimension is queried only when some leaf rule reads it.
    let wants = |dimension: AutoDimension| {
        fn tree_wants(node: &AutoNode, dimension: AutoDimension) -> bool {
            match node {
                AutoNode::None => false,
                AutoNode::Rule(criterion) => criterion.dimension == dimension,
                AutoNode::Logic { nodes, .. } => {
                    nodes.iter().any(|node| tree_wants(node, dimension))
                }
            }
        }
        auto_groups
            .iter()
            .any(|group| tree_wants(&group.root, dimension))
    };
    let db = state.borrow().db.clone();
    let genres = if wants(AutoDimension::Genre) {
        ira_db::game_entity_names_all(&db, ira_db::KIND_GENRE, None).unwrap_or_default()
    } else {
        Default::default()
    };
    let families = if wants(AutoDimension::Family) {
        ira_db::game_entity_names_all(&db, ira_db::KIND_FAMILY, None).unwrap_or_default()
    } else {
        Default::default()
    };
    let developers = if wants(AutoDimension::Developer) {
        ira_db::game_entity_names_all(&db, ira_db::KIND_COMPANY, Some("is_developer"))
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let publishers = if wants(AutoDimension::Publisher) {
        ira_db::game_entity_names_all(&db, ira_db::KIND_COMPANY, Some("is_publisher"))
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let ctx = AutoGroupContext {
        genres,
        families,
        developers,
        publishers,
    };
    let mut members: HashMap<i64, HashSet<i64>> = HashMap::new();
    for game in &games {
        for group in &auto_groups {
            if group.matches(game, &ctx) {
                members.entry(group.id).or_default().insert(game.db_id);
            }
        }
    }
    let mut s = state.borrow_mut();
    s.group_members.retain(|k, _| *k >= 0);
    for (id, ids) in members {
        s.group_members.insert(id, ids);
    }
}

/// The auto group behind a memory id, for names and rules.
pub(crate) fn find_auto_group(state: &SharedState, memory_id: i64) -> Option<AutoGroup> {
    state
        .borrow()
        .auto_groups
        .iter()
        .find(|group| group.id == memory_id)
        .cloned()
}
