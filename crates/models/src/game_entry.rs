use super::kind::{GameKind, TrophySource};

#[derive(Debug, Clone)]
pub struct GameEntry {
    pub id: i64,
    pub kind: GameKind,
    pub trophy_source: TrophySource,
    pub steam_id: String,
    pub game_id: String,
    pub platform_id: String,
    pub title: String,
    pub hidden: bool,
    /// SteamGridDB id for games with no achievement source but need images.
    pub sgdb_id: Option<String>,
    /// Per-game logo overlay position (e.g. "bottom-left").
    pub logo_position: String,
    /// Per-game logo height constraint in pixels.
    pub logo_size: i32,
    /// Set when user manually unmatches — prevents auto-rematching.
    pub manual_unmatch: bool,
    /// Sort title (empty = use title for sorting).
    pub sort_title: String,
    /// Per-game shadPS4 version path (empty = use global default).
    pub shadps4_version: String,
    /// Unix timestamp of last time the game was launched via our play button.
    pub last_played: i64,
    /// Raw release date string from Steam API (e.g. "15 Sep, 2014").
    pub release_date: String,
    /// Parsed release date as Unix timestamp (0 = unknown).
    pub release_timestamp: i64,
    /// Metacritic score 0-100 (-1 = no data).
    pub metacritic_score: i64,
    /// Steam review score 0-10 (-1 = no data).
    pub steam_review_score: i64,
    /// Total Steam review count.
    pub steam_review_count: i64,
    /// Per-game RetroArch core override (empty = use global default).
    pub ra_core: String,
    /// Per-game emulator override (empty = use global default).
    pub emulator_override: String,
    /// Path to the ROM file (for retro games).
    pub rom_path: String,
    /// Playtime in hours.
    pub playtime: f64,
    /// Cached achievement count (earned) from last full load.
    pub cached_earned_count: i64,
    /// Cached achievement count (total) from last full load.
    pub cached_total_count: i64,
    /// Mtime of achievement files at last cache write (0 = never cached).
    pub cached_achievement_mtime: i64,
}

impl GameEntry {
    /// Build a minimal GameEntry for reloading a game from disk.
    /// Callers can override specific fields (e.g. `entry.title = ...`) as needed.
    pub fn for_reload(db_id: i64, kind: GameKind, trophy_source: TrophySource, steam_id: &str, game_id: &str, platform_id: &str) -> Self {
        GameEntry {
            id: db_id,
            kind,
            trophy_source,
            steam_id: steam_id.to_string(),
            game_id: game_id.to_string(),
            platform_id: platform_id.to_string(),
            title: String::new(),
            hidden: false,
            sgdb_id: None,
            logo_position: String::new(),
            logo_size: 0,
            manual_unmatch: false,
            sort_title: String::new(),
            shadps4_version: String::new(),
            last_played: 0,
            release_date: String::new(),
            release_timestamp: 0,
            metacritic_score: -1,
            steam_review_score: -1,
            steam_review_count: 0,
            ra_core: String::new(),
            emulator_override: String::new(),
            rom_path: String::new(),
            playtime: 0.0,
            cached_earned_count: 0,
            cached_total_count: 0,
            cached_achievement_mtime: 0,
        }
    }

    pub fn from_game(g: &super::game::Game) -> Self {
        GameEntry {
            id: g.db_id,
            kind: g.kind,
            trophy_source: g.trophy_source,
            steam_id: if g.app_id.is_empty() { String::new() } else { g.app_id.clone() },
            game_id: g.app_id.clone(),
            platform_id: g.platform_id.clone(),
            title: g.name.clone(),
            hidden: g.hidden,
            sgdb_id: if g.sgdb_id.is_empty() { None } else { Some(g.sgdb_id.clone()) },
            logo_position: g.logo_position.clone(),
            logo_size: g.logo_size,
            manual_unmatch: g.manual_unmatch,
            sort_title: g.sort_title.clone(),
            shadps4_version: g.shadps4_version.clone(),
            last_played: g.last_played,
            release_date: g.release_date.clone(),
            release_timestamp: g.release_timestamp,
            metacritic_score: g.metacritic_score,
            steam_review_score: g.steam_review_score,
            steam_review_count: g.steam_review_count,
            ra_core: g.ra_core.clone(),
            emulator_override: g.emulator_override.clone(),
            rom_path: g.rom_path.clone(),
            playtime: g.playtime,
            cached_earned_count: g.earned_count as i64,
            cached_total_count: g.total_count as i64,
            cached_achievement_mtime: 0,
        }
    }
}
