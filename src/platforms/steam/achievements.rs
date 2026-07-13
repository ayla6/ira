use std::collections::HashMap;
use std::path::Path;

use crate::models::AchievementStatus;
use super::paths;

/// Full achievement data from the Steam librarycache.
pub struct SteamAchievement {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub earned: bool,
    pub earned_time: i64,
    pub icon_url: String,
    pub global_percent: f64,
}

/// Result of reading Steam achievements: the list plus authoritative counts.
pub struct SteamAchievementData {
    pub achievements: Vec<SteamAchievement>,
    pub n_total: usize,
    pub n_achieved: usize,
}

/// Read achievements from the Steam librarycache for a given app ID.
/// Returns the visible achievements (~15 recent ones) plus nTotal/nAchieved
/// from the librarycache for correct sidebar counts.
pub fn read_steam_achievements_full(app_id: &str, _save_dir: &str) -> SteamAchievementData {
    let user_ids = paths::get_steam_user_ids();

    for steam_id in &user_ids {
        let Some(path) = paths::librarycache_path(steam_id, app_id) else {
            continue;
        };
        if !path.is_file() { continue; }
        match parse_librarycache_full(&path) {
            Ok((achs, n_total, n_achieved)) => {
                return SteamAchievementData { achievements: achs, n_total, n_achieved };
            }
            Err(e) => {
                eprintln!("[steam] parse_librarycache_full error for {}: {}", app_id, e);
            }
        }
    }
    SteamAchievementData { achievements: Vec::new(), n_total: 0, n_achieved: 0 }
}

fn parse_librarycache_full(path: &Path) -> Result<(Vec<SteamAchievement>, usize, usize), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let arr: Vec<serde_json::Value> = serde_json::from_slice(&data).map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut n_total = 0usize;
    let mut n_achieved = 0usize;

    for entry in &arr {
        let Some(pair) = entry.as_array() else { continue };
        if pair.len() < 2 { continue; }
        if pair[0].as_str() != Some("achievements") { continue; }

        let data_obj = pair[1].get("data").ok_or("no data")?;

        n_total = data_obj.get("nTotal").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        n_achieved = data_obj.get("nAchieved").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        for vec_name in &["vecHighlight", "vecUnachieved", "vecAchievedHidden"] {
            if let Some(vec) = data_obj.get(vec_name).and_then(|v| v.as_array()) {
                for ach in vec {
                    let Some(id) = ach.get("strID").and_then(|s| s.as_str()) else { continue };
                    if !seen.insert(id.to_string()) { continue }

                    result.push(SteamAchievement {
                        id: id.to_string(),
                        display_name: ach.get("strName").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                        description: ach.get("strDescription").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                        hidden: ach.get("bHidden").and_then(|b| b.as_bool()).unwrap_or(false),
                        earned: ach.get("bAchieved").and_then(|b| b.as_bool()).unwrap_or(false),
                        earned_time: ach.get("rtUnlocked").and_then(|t| t.as_i64()).unwrap_or(0),
                        icon_url: ach.get("strImage").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                        global_percent: ach.get("flAchieved").and_then(|f| f.as_f64()).unwrap_or(0.0),
                    });
                }
            }
        }
        break;
    }

    Ok((result, n_total, n_achieved))
}

/// Simple unlock-state-only read (for compatibility with load_game's status_map path).
pub fn read_steam_achievements(app_id: &str, save_dir: &str) -> HashMap<String, AchievementStatus> {
    read_steam_achievements_full(app_id, save_dir)
        .achievements
        .into_iter()
        .map(|a| (
            a.id,
            AchievementStatus { earned: a.earned, earned_time: a.earned_time },
        ))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_librarycache_full() {
        let json = r#"[
  ["achievements", {"data": {
    "vecHighlight": [
      {"strID": "ACH_1", "strName": "First", "strDescription": "Do thing 1", "bAchieved": true, "rtUnlocked": 100, "strImage": "https://example.com/1.jpg", "bHidden": false, "flAchieved": 50.0}
    ],
    "vecUnachieved": [
      {"strID": "ACH_2", "strName": "Second", "strDescription": "Do thing 2", "bAchieved": false, "rtUnlocked": 0, "strImage": "https://example.com/2.jpg", "bHidden": false, "flAchieved": 30.0}
    ],
    "vecAchievedHidden": [],
    "nTotal": 2,
    "nAchieved": 1
  }}]
]"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), json).unwrap();
        let (achs, n_total, n_achieved) = parse_librarycache_full(tmp.path()).unwrap();
        assert_eq!(achs.len(), 2);
        assert_eq!(n_total, 2);
        assert_eq!(n_achieved, 1);
        assert!(achs[0].earned);
        assert_eq!(achs[0].display_name, "First");
        assert!(!achs[1].earned);
        assert_eq!(achs[1].icon_url, "https://example.com/2.jpg");
    }
}
