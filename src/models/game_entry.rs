#[derive(Debug, Clone)]
pub struct GameEntry {
    pub id: i64,
    pub kind: String,
    pub steam_id: String,
    pub platform_id: String,
    pub title: String,
    pub hidden: bool,
    /// Link to the Lutris game (its internal numeric id). None = not linked.
    pub lutris_db_id: Option<i64>,
    /// SteamGridDB id for games with no achievement source but need images.
    pub sgdb_id: Option<String>,
    /// Per-game logo overlay position (e.g. "bottom-left").
    pub logo_position: String,
    /// Per-game logo height constraint in pixels.
    pub logo_size: i32,
    /// Set when user removes a game — prevents re-adding from Lutris.
    pub ignored: i64,
    /// Set when user manually unmatches — prevents auto-rematching.
    pub manual_unmatch: i64,
    /// Sort title (empty = use title for sorting).
    pub sort_title: String,
    /// Per-game shadPS4 version path (empty = use global default).
    pub shadps4_version: Option<String>,
    /// Unix timestamp of last time the game was launched via our play button.
    pub last_played: i64,
}

impl GameEntry {
    /// Build a minimal GameEntry for reloading a game from disk.
    /// Callers can override specific fields (e.g. `entry.title = ...`) as needed.
    pub fn for_reload(db_id: i64, kind: &str, steam_id: &str, platform_id: &str, lutris_id: i64) -> Self {
        GameEntry {
            id: db_id,
            kind: kind.to_string(),
            steam_id: steam_id.to_string(),
            platform_id: platform_id.to_string(),
            title: String::new(),
            hidden: false,
            lutris_db_id: if lutris_id != 0 { Some(lutris_id) } else { None },
            sgdb_id: None,
            logo_position: String::new(),
            logo_size: 0,
            ignored: 0,
            manual_unmatch: 0,
            sort_title: String::new(),
            shadps4_version: None,
            last_played: 0,
        }
    }
}
