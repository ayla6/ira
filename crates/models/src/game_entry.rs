use super::asset_type::LogoPosition;
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
    /// Path to the game install directory (for Wine/Linux games).
    pub game_folder: String,
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
    pub fn for_reload(
        db_id: i64,
        kind: GameKind,
        trophy_source: TrophySource,
        steam_id: &str,
        game_id: &str,
        platform_id: &str,
    ) -> Self {
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
            logo_position: LogoPosition::BottomLeft.to_string(),
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
            game_folder: String::new(),
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
            steam_id: if g.app_id.is_empty() {
                String::new()
            } else {
                g.app_id.clone()
            },
            game_id: g.app_id.clone(),
            platform_id: g.platform_id.clone(),
            title: g.name.clone(),
            hidden: g.hidden,
            sgdb_id: if g.sgdb_id.is_empty() {
                None
            } else {
                Some(g.sgdb_id.clone())
            },
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
            game_folder: g.game_folder.clone(),
            playtime: g.playtime,
            cached_earned_count: g.earned_count as i64,
            cached_total_count: g.total_count as i64,
            cached_achievement_mtime: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_type::LogoPosition;
    use crate::game::Game;
    use crate::kind::{GameKind, TrophySource};

    #[test]
    fn test_for_reload_sets_defaults() {
        let entry =
            GameEntry::for_reload(42, GameKind::Steam, TrophySource::Gse, "sid", "gid", "pid");

        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, GameKind::Steam);
        assert_eq!(entry.trophy_source, TrophySource::Gse);
        assert_eq!(entry.steam_id, "sid");
        assert_eq!(entry.game_id, "gid");
        assert_eq!(entry.platform_id, "pid");
        assert_eq!(entry.logo_position, LogoPosition::BottomLeft.to_string());
        assert_eq!(entry.title, "");
        assert!(!entry.hidden);
        assert!(entry.sgdb_id.is_none());
        assert_eq!(entry.logo_size, 0);
        assert!(!entry.manual_unmatch);
        assert_eq!(entry.sort_title, "");
        assert_eq!(entry.shadps4_version, "");
        assert_eq!(entry.last_played, 0);
        assert_eq!(entry.release_date, "");
        assert_eq!(entry.release_timestamp, 0);
        assert_eq!(entry.metacritic_score, -1);
        assert_eq!(entry.steam_review_score, -1);
        assert_eq!(entry.steam_review_count, 0);
        assert_eq!(entry.ra_core, "");
        assert_eq!(entry.emulator_override, "");
        assert_eq!(entry.rom_path, "");
        assert_eq!(entry.game_folder, "");
        assert_eq!(entry.playtime, 0.0);
        assert_eq!(entry.cached_earned_count, 0);
        assert_eq!(entry.cached_total_count, 0);
        assert_eq!(entry.cached_achievement_mtime, 0);
    }

    #[test]
    fn test_from_game_copies_all_fields() {
        let g = Game {
            app_id: "app123".to_string(),
            kind: GameKind::Steam,
            trophy_source: TrophySource::Gse,
            platform_id: "plat1".to_string(),
            db_id: 99,
            name: "Test Game".to_string(),
            hidden: true,
            sgdb_id: "sgdb_1".to_string(),
            logo_position: "center".to_string(),
            logo_size: 80,
            manual_unmatch: true,
            sort_title: "SortKey".to_string(),
            shadps4_version: "v1.0".to_string(),
            last_played: 1000,
            release_date: "15 Sep, 2014".to_string(),
            release_timestamp: 1410739200,
            metacritic_score: 85,
            steam_review_score: 8,
            steam_review_count: 5000,
            ra_core: "nestopia".to_string(),
            emulator_override: "retroarch".to_string(),
            rom_path: "/path/to/rom".to_string(),
            game_folder: "/path/to/game".to_string(),
            playtime: 12.5,
            earned_count: 10,
            total_count: 20,
            ..Default::default()
        };
        let entry = GameEntry::from_game(&g);

        assert_eq!(entry.id, g.db_id);
        assert_eq!(entry.kind, g.kind);
        assert_eq!(entry.trophy_source, g.trophy_source);
        assert_eq!(entry.steam_id, g.app_id);
        assert_eq!(entry.game_id, g.app_id);
        assert_eq!(entry.platform_id, g.platform_id);
        assert_eq!(entry.title, g.name);
        assert_eq!(entry.hidden, g.hidden);
        assert_eq!(entry.sgdb_id, Some(g.sgdb_id.clone()));
        assert_eq!(entry.logo_position, g.logo_position);
        assert_eq!(entry.logo_size, g.logo_size);
        assert_eq!(entry.manual_unmatch, g.manual_unmatch);
        assert_eq!(entry.sort_title, g.sort_title);
        assert_eq!(entry.shadps4_version, g.shadps4_version);
        assert_eq!(entry.last_played, g.last_played);
        assert_eq!(entry.release_date, g.release_date);
        assert_eq!(entry.release_timestamp, g.release_timestamp);
        assert_eq!(entry.metacritic_score, g.metacritic_score);
        assert_eq!(entry.steam_review_score, g.steam_review_score);
        assert_eq!(entry.steam_review_count, g.steam_review_count);
        assert_eq!(entry.ra_core, g.ra_core);
        assert_eq!(entry.emulator_override, g.emulator_override);
        assert_eq!(entry.rom_path, g.rom_path);
        assert_eq!(entry.game_folder, g.game_folder);
        assert_eq!(entry.playtime, g.playtime);
        assert_eq!(entry.cached_earned_count, g.earned_count as i64);
        assert_eq!(entry.cached_total_count, g.total_count as i64);
        assert_eq!(entry.cached_achievement_mtime, 0);
    }

    #[test]
    fn test_for_reload_has_consistent_defaults() {
        let entry = GameEntry::for_reload(1, GameKind::Other, TrophySource::Empty, "", "", "");
        let _ = format!("{entry:?}");
        assert_eq!(entry.id, 1);
        assert!(entry.steam_id.is_empty());
        assert!(entry.game_id.is_empty());
        assert!(entry.platform_id.is_empty());
        assert_eq!(entry.logo_position, LogoPosition::BottomLeft.to_string());
    }
}
