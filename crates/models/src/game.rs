use super::achievement::MergedAchievement;
use super::asset_type::LogoPosition;
use super::kind::{GameKind, TrophySource};

#[derive(Debug, Clone)]
pub struct Game {
    pub app_id: String,
    pub kind: GameKind,
    pub trophy_source: TrophySource,
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
    pub slug: String,
    /// Playtime in hours.
    pub playtime: f64,
    /// Unix timestamp of last play.
    pub last_played: i64,
    /// Logo overlay position (e.g. "bottom-left", "center", etc.).
    pub logo_position: String,
    /// Logo overlay pixel size.
    pub logo_size: i32,
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
    /// If Some(variant_id), this is a pseudo-game entry for a variant
    /// shown as a separate grid entry. None for real games.
    pub variant_id: Option<i64>,
}

impl Default for Game {
    fn default() -> Self {
        Game {
            app_id: String::new(),
            kind: GameKind::default(),
            trophy_source: TrophySource::default(),
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
            slug: String::new(),
            playtime: 0.0,
            last_played: 0,
            logo_position: LogoPosition::BottomLeft.to_string(),
            logo_size: 50,
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
            variant_id: None,
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

    /// Unique identifier for grid/sidebar selection.
    /// Real games use db_id; variant pseudo-entries use "{db_id}-v{variant_id}".
    pub fn grid_id(&self) -> String {
        match self.variant_id {
            Some(vid) => format!("{}-v{}", self.db_id, vid),
            None => self.db_id.to_string(),
        }
    }

    /// Whether this is a variant pseudo-game entry (not a real game).
    pub fn is_variant_entry(&self) -> bool {
        self.variant_id.is_some()
    }
}

/// Extract the db_id from a grid_id string ("123" or "123-v456").
pub fn parse_db_id(grid_id: &str) -> i64 {
    grid_id.split("-v").next().and_then(|s| s.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_key_uses_sort_title_when_non_empty() {
        let g = Game {
            sort_title: "Alpha".to_string(),
            name: "Zeta".to_string(),
            ..Default::default()
        };
        assert_eq!(g.sort_key(), "Alpha");
    }

    #[test]
    fn test_sort_key_falls_back_to_name() {
        let g = Game {
            sort_title: String::new(),
            name: "Zeta".to_string(),
            ..Default::default()
        };
        assert_eq!(g.sort_key(), "Zeta");
    }

    #[test]
    fn test_completion_pct_total_zero() {
        let g = Game::default();
        assert_eq!(g.completion_pct(), 0.0);
    }

    #[test]
    fn test_completion_pct_some_earned() {
        let g = Game {
            earned_count: 5,
            total_count: 10,
            ..Default::default()
        };
        assert!((g.completion_pct() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_completion_pct_none_earned() {
        let g = Game {
            earned_count: 0,
            total_count: 10,
            ..Default::default()
        };
        assert_eq!(g.completion_pct(), 0.0);
    }

}
