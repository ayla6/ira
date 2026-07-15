use super::achievement::MergedAchievement;

#[derive(Debug, Clone)]
pub struct Game {
    pub app_id: String,
    pub kind: String,
    pub trophy_source: String,
    pub platform_id: String,
    pub db_id: i64,
    pub name: String,
    pub icon_path: String,
    pub hero_image_path: String,
    pub grid_path: String,
    pub header_path: String,
    pub logo_path: String,
    pub achievements: Vec<MergedAchievement>,
    pub earned_count: usize,
    pub total_count: usize,
    pub hidden: bool,
    /// Lutris internal game id (0 = not linked / unmatched).
    pub lutris_id: i64,
    pub slug: String,
    /// Playtime in hours (from Lutris).
    pub playtime: f64,
    /// Unix timestamp of last play (from Lutris).
    pub last_played: i64,
    /// Logo overlay position (e.g. "bottom-left", "center", etc.).
    pub logo_position: String,
    /// Logo overlay pixel size.
    pub logo_size: i32,
    /// Original Lutris name (for restoring on unmatch).
    pub lutris_name: String,
    /// True if user manually unmatched — don't auto-rematch.
    pub manual_unmatch: bool,
    /// Sort key (empty = use name for sorting).
    pub sort_title: String,
    /// Path to game directory (for PS4 games — used to find eboot.bin).
    pub game_path: String,
    /// SteamGridDB game ID (if matched) for image downloads.
    pub sgdb_id: String,
    /// Per-game shadPS4 version path (empty = use global default).
    pub shadps4_version: String,
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
}

impl Default for Game {
    fn default() -> Self {
        Game {
            app_id: String::new(),
            kind: String::new(),
            trophy_source: String::new(),
            platform_id: String::new(),
            db_id: 0,
            name: String::new(),
            icon_path: String::new(),
            hero_image_path: String::new(),
            grid_path: String::new(),
            header_path: String::new(),
            logo_path: String::new(),
            achievements: Vec::new(),
            earned_count: 0,
            total_count: 0,
            hidden: false,
            lutris_id: 0,
            slug: String::new(),
            playtime: 0.0,
            last_played: 0,
            logo_position: "bottom-left".to_string(),
            logo_size: 50,
            lutris_name: String::new(),
            manual_unmatch: false,
            sort_title: String::new(),
            game_path: String::new(),
            sgdb_id: String::new(),
            shadps4_version: String::new(),
            release_date: String::new(),
            release_timestamp: 0,
            metacritic_score: -1,
            steam_review_score: -1,
            steam_review_count: 0,
            ra_core: String::new(),
            emulator_override: String::new(),
            rom_path: String::new(),
        }
    }
}

impl Game {
    pub fn sort_key(&self) -> &str {
        if self.sort_title.is_empty() { &self.name } else { &self.sort_title }
    }

    pub fn completion_pct(&self) -> f64 {
        if self.total_count == 0 {
            0.0
        } else {
            self.earned_count as f64 / self.total_count as f64 * 100.0
        }
    }
}

/// A Lutris game with no matched achievement source yet — shown in the sidebar
/// with no achievements until the user matches it to a Steam/GOG app id.
pub fn unmatched_game(lutris_id: i64, name: &str, slug: &str, playtime: f64, last_played: i64) -> Game {
    Game {
        lutris_id,
        name: name.to_string(),
        slug: slug.to_string(),
        playtime,
        last_played,
        lutris_name: name.to_string(),
        ..Default::default()
    }
}
