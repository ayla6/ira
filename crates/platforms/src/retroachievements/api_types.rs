use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use tracing::info_span;

use crate::retroachievements::api::RaClient;
use crate::retroachievements::paths;
use ira_models::{Game, MergedAchievement};

/// Fake achievement RA injects via the legacy dorequest `patch` endpoint as a
/// client-support warning. Web API responses never contain it; it is filtered
/// out here so nothing downstream has to special-case it.
const RA_WARNING_ACHIEVEMENT_ID: u32 = 101000001;

pub fn read_console_games_cache(save_dir: &str, console_id: u32) -> Option<Vec<RaGameEntry>> {
    let cache = paths::console_games_path(save_dir, console_id);
    let data = std::fs::read(&cache).ok()?;
    serde_json::from_slice(&data).ok()
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RaGameData {
    #[serde(default, rename = "ID")]
    pub id: u32,
    #[serde(default, rename = "Title")]
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

/// User unlock state for a single achievement, from the Web API
/// (`API_GetGameInfoAndUserProgress`).
#[derive(Debug, Clone, Copy, Default)]
pub struct RaUnlockInfo {
    pub earned: bool,
    pub earned_time: i64,
}

impl RaUnlockInfo {
    pub(crate) fn earned(time: i64) -> Self {
        Self {
            earned: true,
            earned_time: time,
        }
    }
}

/// One game-level info+progress fetch from the RA Web API.
/// `Achievements` is a map keyed by achievement id; entries are only present
/// for achievements returned by the endpoint, and `DateEarned*` fields only
/// when the user has earned them.
#[derive(Debug, Deserialize)]
pub(crate) struct WebGameProgress {
    #[serde(default, rename = "ID")]
    pub(crate) id: u32,
    #[serde(default, rename = "Title")]
    pub(crate) title: String,
    #[serde(default, rename = "ImageIcon")]
    pub(crate) image_icon: String,
    #[serde(default, rename = "NumDistinctPlayers")]
    pub(crate) num_distinct_players: u32,
    #[serde(default, rename = "Achievements")]
    pub(crate) achievements: HashMap<String, WebAchievement>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WebAchievement {
    #[serde(default, rename = "ID")]
    pub(crate) id: u32,
    #[serde(default, rename = "Title")]
    pub(crate) title: String,
    #[serde(default, rename = "Description")]
    pub(crate) description: String,
    #[serde(default, rename = "Points")]
    pub(crate) points: u32,
    #[serde(default, rename = "BadgeName")]
    pub(crate) badge_name: String,
    #[serde(default, rename = "NumAwarded")]
    pub(crate) num_awarded: u32,
    #[serde(default, rename = "NumAwardedHardcore")]
    pub(crate) num_awarded_hardcore: u32,
    #[serde(default, rename = "DisplayOrder")]
    pub(crate) display_order: u32,
    #[serde(default, rename = "DateEarned")]
    pub(crate) date_earned: Option<String>,
    #[serde(default, rename = "DateEarnedHardcore")]
    pub(crate) date_earned_hardcore: Option<String>,
}

/// RA "Y-m-d H:i:s" UTC timestamps (as returned by the Web API) parsed into
/// unix seconds. Returns 0 for empty/whitespace input.
fn parse_ra_datetime(s: &str) -> i64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let (date, time) = match s.split_once(' ') {
        Some(p) => p,
        None => return 0,
    };
    let mut dparts = date.split('-');
    let year = dparts.next().and_then(|d| d.parse::<i64>().ok());
    let month = dparts.next().and_then(|d| d.parse::<i64>().ok());
    let day = dparts.next().and_then(|d| d.parse::<i64>().ok());
    let (year, month, day) = match (year, month, day) {
        (Some(y), Some(m), Some(d)) => (y, m, d),
        _ => return 0,
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return 0;
    }

    let mut tparts = time.split(':');
    let hour = tparts.next().and_then(|d| d.parse::<i64>().ok());
    let minute = tparts.next().and_then(|d| d.parse::<i64>().ok());
    let second = tparts.next().and_then(|d| d.parse::<i64>().ok());
    let (hour, minute, second) = match (hour, minute, second) {
        (Some(h), Some(mi), Some(s)) if h < 24 && mi < 60 && s < 60 => (h, mi, s),
        _ => return 0,
    };

    days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
}

/// Howard Hinnant's days-from-civil algorithm (proleptic Gregorian, epoch
/// 1970-01-01). Valid for the date range RA serves.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Convert a single Web API progress response into the game data shape used by
/// the rest of the app, plus per-achievement unlock info. The warning fake
/// achievement is dropped here so nothing downstream has to special-case it.
pub(crate) fn web_progress_to_data(
    progress: &WebGameProgress,
) -> (RaGameData, HashMap<u32, RaUnlockInfo>) {
    let players = progress.num_distinct_players.max(1) as f64;

    let mut defs: Vec<(u32, RaAchievementDef)> = progress
        .achievements
        .values()
        .filter(|a| a.id != RA_WARNING_ACHIEVEMENT_ID)
        .map(|a| {
            let def = RaAchievementDef {
                id: a.id,
                title: a.title.clone(),
                description: a.description.clone(),
                points: a.points,
                badge_name: a.badge_name.clone(),
                rarity: a.num_awarded as f64 * 100.0 / players,
                rarity_hardcore: a.num_awarded_hardcore as f64 * 100.0 / players,
            };
            (a.display_order, def)
        })
        .collect();
    defs.sort_by_key(|(order, _)| *order);

    let mut unlocks = HashMap::new();
    for a in progress.achievements.values() {
        if a.id == RA_WARNING_ACHIEVEMENT_ID {
            continue;
        }
        let hardcore_time = a
            .date_earned_hardcore
            .as_deref()
            .map(parse_ra_datetime)
            .unwrap_or(0);
        let softcore_time = a.date_earned.as_deref().map(parse_ra_datetime).unwrap_or(0);
        let earned_time = hardcore_time.max(softcore_time);
        if earned_time > 0 {
            unlocks.insert(a.id, RaUnlockInfo::earned(earned_time));
        }
    }

    (
        RaGameData {
            id: progress.id,
            title: progress.title.clone(),
            image_icon: progress.image_icon.clone(),
            achievements: defs.into_iter().map(|(_, d)| d).collect(),
        },
        unlocks,
    )
}

pub fn build_ra_achievements(
    game_data: &RaGameData,
    unlocks: &HashMap<u32, RaUnlockInfo>,
    client: &RaClient,
    save_dir: &str,
    game_id: &str,
) -> (Vec<MergedAchievement>, String, String) {
    let _s = info_span!(
        "build_ra_achievements",
        game_id,
        count = game_data.achievements.len()
    )
    .entered();
    let mut achievements = Vec::new();
    let mut icon_path = String::new();
    let mut icon_gray_path = String::new();

    for def in &game_data.achievements {
        let unlock = unlocks.get(&def.id).copied().unwrap_or_default();
        let earned = unlock.earned;
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
            earned_time: unlock.earned_time,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }

    (achievements, icon_path, icon_gray_path)
}

pub fn enrich_ra_game(game: &mut Game, save_dir: &str, username: &str, web_api_key: &str) {
    let _s = info_span!("enrich_ra_game", game_id = &game.app_id[..]).entered();
    let client = RaClient::new(username, web_api_key);

    let (game_data, unlocks) = match client.fetch_web_game_progress(save_dir, &game.app_id) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("RA web progress fetch failed for {}: {}", game.app_id, e);
            return;
        }
    };

    let (achievements, icon_path, _icon_gray) =
        build_ra_achievements(&game_data, &unlocks, &client, save_dir, &game.app_id);

    game.total_count = achievements.len();
    game.earned_count = achievements.iter().filter(|a| a.earned).count();
    game.achievements = achievements;

    if game.icon_path.is_empty() && !game_data.image_icon.is_empty() {
        let icon = client.download_game_icon(save_dir, game.db_id, &game_data.image_icon);
        if !icon.is_empty() {
            game.icon_path = icon;
        }
    }
    if game.icon_path.is_empty() && !icon_path.is_empty() {
        game.icon_path = icon_path;
    }
}

/// Read achievements for a game purely from the `web_progress.json` cache.
/// Caches written by the pre-Web-API (dorequest) format are not migrated:
/// games fetched before the upgrade return empty here until a network enrich
/// rewrites the cache in the current format.
pub fn load_ra_achievements_from_cache(save_dir: &str, game_id: &str) -> Vec<MergedAchievement> {
    let (game_data, unlocks) = load_ra_progress_from_cache(save_dir, game_id);

    let ach_dir = paths::achievements_dir(save_dir, game_id);
    let badge_files: HashSet<String> = std::fs::read_dir(&ach_dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut achievements = Vec::new();
    for def in &game_data.achievements {
        let unlock = unlocks.get(&def.id).copied().unwrap_or_default();
        let earned = unlock.earned;
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
            earned_time: unlock.earned_time,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }
    achievements
}

fn load_ra_progress_from_cache(
    save_dir: &str,
    game_id: &str,
) -> (RaGameData, HashMap<u32, RaUnlockInfo>) {
    let web_path = paths::web_progress_path(save_dir, game_id);
    if let Ok(data) = std::fs::read(&web_path) {
        if let Ok(progress) = serde_json::from_slice::<WebGameProgress>(&data) {
            return web_progress_to_data(&progress);
        }
    }
    (RaGameData::default(), HashMap::new())
}

pub fn redownload_missing_ra_badges(save_dir: &str, game_id: &str) -> bool {
    let _s = info_span!("redownload_missing_ra_badges", game_id).entered();
    let game_data: RaGameData = match std::fs::read(paths::web_progress_path(save_dir, game_id)) {
        Ok(data) => match serde_json::from_slice::<WebGameProgress>(&data) {
            Ok(resp) => web_progress_to_data(&resp).0,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    let client = RaClient::new("", "");
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

#[cfg(test)]
mod web_progress_tests {
    use super::{web_progress_to_data, RaUnlockInfo, WebAchievement, RA_WARNING_ACHIEVEMENT_ID};
    use crate::retroachievements::api_types::WebGameProgress;

    fn progress(achievements: Vec<WebAchievement>) -> WebGameProgress {
        WebGameProgress {
            id: 5,
            title: "Test Game".to_string(),
            image_icon: "/Images/Test.png".to_string(),
            num_distinct_players: 200,
            achievements: achievements
                .into_iter()
                .map(|a| (a.id.to_string(), a))
                .collect(),
        }
    }

    fn ach(id: u32, order: u32, awarded: u32, hardcore: u32) -> WebAchievement {
        WebAchievement {
            id,
            title: format!("Ach {id}"),
            description: String::new(),
            points: 5,
            badge_name: String::new(),
            num_awarded: awarded,
            num_awarded_hardcore: hardcore,
            display_order: order,
            date_earned: None,
            date_earned_hardcore: None,
        }
    }

    #[test]
    fn test_web_progress_sorts_by_display_order() {
        let p = progress(vec![ach(2, 2, 0, 0), ach(1, 1, 0, 0)]);
        let (data, _) = web_progress_to_data(&p);
        let ids: Vec<u32> = data.achievements.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn test_web_progress_computes_rarity_from_players() {
        let p = progress(vec![ach(1, 1, 50, 20)]);
        let (data, _) = web_progress_to_data(&p);
        let def = &data.achievements[0];
        assert!((def.rarity - 25.0).abs() < 1e-9);
        assert!((def.rarity_hardcore - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_web_progress_avoids_divide_by_zero_players() {
        let mut p = progress(vec![ach(1, 1, 1, 0)]);
        p.num_distinct_players = 0;
        let (data, _) = web_progress_to_data(&p);
        assert!((data.achievements[0].rarity - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_web_progress_drops_warning_achievement() {
        let p = progress(vec![
            ach(RA_WARNING_ACHIEVEMENT_ID, 0, 0, 0),
            ach(1, 1, 0, 0),
        ]);
        let (data, _) = web_progress_to_data(&p);
        assert_eq!(data.achievements.len(), 1);
    }

    #[test]
    fn test_web_progress_maps_hardcore_unlock() {
        let mut a = ach(1, 1, 100, 50);
        a.date_earned_hardcore = Some("2024-03-01 10:00:00".to_string());
        let (_, unlocks) = web_progress_to_data(&progress(vec![a]));
        let info: &RaUnlockInfo = unlocks.get(&1).unwrap();
        assert!(info.earned);
        assert_eq!(info.earned_time, 1709287200);
    }

    #[test]
    fn test_web_progress_maps_softcore_only_unlock() {
        let mut a = ach(1, 1, 100, 50);
        a.date_earned = Some("2024-03-01 10:00:00".to_string());
        let (_, unlocks) = web_progress_to_data(&progress(vec![a]));
        let info: &RaUnlockInfo = unlocks.get(&1).unwrap();
        assert!(info.earned);
        assert_eq!(info.earned_time, 1709287200);
    }

    #[test]
    fn test_web_progress_ignores_zero_dates() {
        let mut a = ach(1, 1, 100, 50);
        a.date_earned = Some("0000-00-00 00:00:00".to_string());
        let (_, unlocks) = web_progress_to_data(&progress(vec![a]));
        assert!(unlocks.is_empty());
    }

    #[test]
    fn test_web_progress_empty_achievements() {
        let (data, unlocks) = web_progress_to_data(&progress(vec![]));
        assert!(data.achievements.is_empty());
        assert!(unlocks.is_empty());
    }

    #[test]
    fn test_web_progress_uses_latest_of_softcore_and_hardcore() {
        let mut a = ach(1, 1, 100, 50);
        a.date_earned = Some("2024-03-01 10:00:00".to_string());
        a.date_earned_hardcore = Some("2024-03-02 10:00:00".to_string());
        let (_, unlocks) = web_progress_to_data(&progress(vec![a]));
        let info: &RaUnlockInfo = unlocks.get(&1).unwrap();
        assert!(info.earned);
        assert_eq!(info.earned_time, 1709373600);
    }
}

#[cfg(test)]
mod datetime_tests {
    use super::parse_ra_datetime;

    #[test]
    fn test_parse_ra_datetime_epoch() {
        assert_eq!(parse_ra_datetime("1970-01-01 00:00:00"), 0);
    }

    #[test]
    fn test_parse_ra_datetime_known_value() {
        assert_eq!(parse_ra_datetime("2024-01-15 04:23:10"), 1705292590);
    }

    #[test]
    fn test_parse_ra_datetime_leap_day() {
        assert_eq!(parse_ra_datetime("2024-02-29 12:00:00"), 1709208000);
    }

    #[test]
    fn test_parse_ra_datetime_empty() {
        assert_eq!(parse_ra_datetime(""), 0);
        assert_eq!(parse_ra_datetime("  "), 0);
    }

    #[test]
    fn test_parse_ra_datetime_garbage() {
        assert_eq!(parse_ra_datetime("not a date"), 0);
        assert_eq!(parse_ra_datetime("2024-13-99 25:61:61"), 0);
    }

    #[test]
    fn test_parse_ra_datetime_out_of_range_month_or_day() {
        assert_eq!(parse_ra_datetime("2024-00-15 04:23:10"), 0);
        assert_eq!(parse_ra_datetime("2024-13-15 04:23:10"), 0);
        assert_eq!(parse_ra_datetime("2024-01-00 04:23:10"), 0);
        assert_eq!(parse_ra_datetime("2024-01-32 04:23:10"), 0);
    }
}
