#[derive(Debug, Clone)]
pub struct Group {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupSelection {
    AllGames,
    Collection(i64),
    Uncategorized,
    /// A metadata-computed category (developer, publisher, ...) — the
    /// name is the category's title, and its members live in the view
    /// state's derived-members map rather than the groups tables.
    Derived(String),
}

/// A stable id for a derived category: the sidebar's collapse and
/// selection bookkeeping is keyed on i64, and user collections occupy
/// the small positive ids.
pub fn derived_group_id(name: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    (hasher.finish() & i64::MAX as u64) as i64
}
