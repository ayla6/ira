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
}
