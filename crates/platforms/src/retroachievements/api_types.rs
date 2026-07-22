use std::collections::HashSet;

use serde::Deserialize;
use tracing::info_span;

use ira_models::{Game, MergedAchievement};
use crate::retroachievements::api::RaClient;
use crate::retroachievements::paths;

pub fn read_console_games_cache(save_dir: &str, console_id: u32) -> Option<Vec<RaGameEntry>> {
    let cache = paths::console_games_path(save_dir, console_id);
    let data = std::fs::read(&cache).ok()?;
    let resp: ConsoleGamesResponse = serde_json::from_slice(&data).ok()?;
    Some(resp.response)
}

#[derive(Debug, Deserialize)]
pub(crate) struct LoginResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "Token")]
    pub(crate) token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConsoleGamesResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "Response")]
    pub(crate) response: Vec<RaGameEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaGameEntry {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(default, rename = "ImageIcon")]
    pub image_icon: String,
    #[serde(default, rename = "ImageUrl")]
    pub image_url: String,
    #[serde(default, rename = "NumAchievements")]
    pub num_achievements: u32,
    #[serde(default, rename = "Points")]
    pub points: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GameDataResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(rename = "PatchData")]
    pub(crate) patch_data: RaGameData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaGameData {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(default, rename = "ImageIcon")]
    pub image_icon: String,
    #[serde(default, rename = "Achievements")]
    pub achievements: Vec<RaAchievementDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaAchievementDef {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Description")]
    pub description: String,
    #[serde(default, rename = "Points")]
    pub points: u32,
    #[serde(default, rename = "BadgeName")]
    pub badge_name: String,
    #[serde(default, rename = "Rarity")]
    pub rarity: f64,
    #[serde(default, rename = "RarityHardcore")]
    pub rarity_hardcore: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UnlocksResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "UserUnlocks")]
    pub(crate) user_unlocks: Vec<u32>,
}

pub fn build_ra_achievements(
    game_data: &RaGameData,
    unlocks: &[u32],
    client: &RaClient,
    save_dir: &str,
    game_id: &str,
) -> (Vec<MergedAchievement>, String, String) {
    let _s = info_span!("build_ra_achievements", game_id, count = game_data.achievements.len()).entered();
    let mut achievements = Vec::new();
    let mut icon_path = String::new();
    let mut icon_gray_path = String::new();

    for def in &game_data.achievements {
        let earned = unlocks.contains(&def.id);
        let badge = if def.badge_name.is_empty() {
            String::new()
        } else if earned {
            client.download_badge(save_dir, game_id, &def.badge_name, false)
        } else {
            client.download_badge(save_dir, game_id, &def.badge_name, true)
        };

        let (icon, icon_gray) = if earned {
            let locked_badge = if def.badge_name.is_empty() {
                String::new()
            } else {
                client.download_badge(save_dir, game_id, &def.badge_name, true)
            };
            (badge.clone(), locked_badge)
        } else {
            let unlocked_badge = if def.badge_name.is_empty() {
                String::new()
            } else {
                client.download_badge(save_dir, game_id, &def.badge_name, false)
            };
            (unlocked_badge, badge.clone())
        };

        if icon_path.is_empty() && !icon.is_empty() {
            icon_path = icon.clone();
        }
        if icon_gray_path.is_empty() && !icon_gray.is_empty() {
            icon_gray_path = icon_gray.clone();
        }

        achievements.push(MergedAchievement {
            name: format!("{}", def.id),
            display_name: def.title.clone(),
            description: def.description.clone(),
            hidden: false,
            earned,
            earned_time: 0,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }

    (achievements, icon_path, icon_gray_path)
}

pub fn enrich_ra_game(game: &mut Game, save_dir: &str, username: &str, token: &str, password: &str) {
    let _s = info_span!("enrich_ra_game", game_id = &game.app_id[..]).entered();
    if RaClient::auth_is_broken() {
        return;
    }

    let client = RaClient::new(username, token, password);

    let game_data = match client.fetch_game_data(save_dir, &game.app_id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("RA game data fetch failed for {}: {}", game.app_id, e);
            return;
        }
    };

    let unlocks = client.fetch_user_unlocks(save_dir, &game.app_id).unwrap_or_default();

    let (achievements, icon_path, _icon_gray) = build_ra_achievements(&game_data, &unlocks, &client, save_dir, &game.app_id);

    game.total_count = achievements.len();
    game.earned_count = achievements.iter().filter(|a| a.earned).count();
    game.achievements = achievements;

    if game.icon_path.is_empty() && !game_data.image_icon.is_empty() {
        let icon = client.download_game_icon(save_dir, &game.app_id, &game_data.image_icon);
        if !icon.is_empty() {
            game.icon_path = icon;
        }
    }
    if game.icon_path.is_empty() && !icon_path.is_empty() {
        game.icon_path = icon_path;
    }
}

pub fn load_ra_achievements_from_cache(save_dir: &str, game_id: &str) -> Vec<MergedAchievement> {
    let game_data_path = paths::game_data_path(save_dir, game_id);
    let game_data: RaGameData = match std::fs::read(&game_data_path) {
        Ok(data) => match serde_json::from_slice::<GameDataResponse>(&data) {
            Ok(resp) => resp.patch_data,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };

    let unlocks: HashSet<u32> = match std::fs::read(paths::unlocks_path(save_dir, game_id)) {
        Ok(data) => serde_json::from_slice::<UnlocksResponse>(&data)
            .map(|r| r.user_unlocks.into_iter().collect())
            .unwrap_or_default(),
        Err(_) => HashSet::new(),
    };

    let ach_dir = paths::achievements_dir(save_dir, game_id);
    let badge_files: HashSet<String> = std::fs::read_dir(&ach_dir)
        .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut achievements = Vec::new();
    for def in &game_data.achievements {
        let earned = unlocks.contains(&def.id);
        let (icon, icon_gray) = if def.badge_name.is_empty() {
            (String::new(), String::new())
        } else {
            let earned_name = format!("{}.webp", def.badge_name);
            let locked_name = format!("{}_lock.webp", def.badge_name);
            let earned_path = if earned && badge_files.contains(&earned_name) {
                ach_dir.join(&earned_name).to_string_lossy().into_owned()
            } else {
                String::new()
            };
            let locked_path = if badge_files.contains(&locked_name) {
                ach_dir.join(&locked_name).to_string_lossy().into_owned()
            } else {
                String::new()
            };
            if earned {
                (earned_path, locked_path)
            } else {
                (locked_path.clone(), locked_path)
            }
        };

        achievements.push(MergedAchievement {
            name: format!("{}", def.id),
            display_name: def.title.clone(),
            description: def.description.clone(),
            hidden: false,
            earned,
            earned_time: 0,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }
    achievements
}

pub fn redownload_missing_ra_badges(save_dir: &str, game_id: &str) -> bool {
    let _s = info_span!("redownload_missing_ra_badges", game_id).entered();
    let game_data_path = paths::game_data_path(save_dir, game_id);
    let game_data: RaGameData = match std::fs::read(&game_data_path) {
        Ok(data) => match serde_json::from_slice::<GameDataResponse>(&data) {
            Ok(resp) => resp.patch_data,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    let client = RaClient::new("", "", "");
    let mut downloaded_any = false;
    for def in &game_data.achievements {
        if def.badge_name.is_empty() {
            continue;
        }
        let unlocked = client.download_badge(save_dir, game_id, &def.badge_name, false);
        let locked = client.download_badge(save_dir, game_id, &def.badge_name, true);
        if !unlocked.is_empty() || !locked.is_empty() {
            downloaded_any = true;
        }
    }
    downloaded_any
}
