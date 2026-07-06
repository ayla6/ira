use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::{DbConn, GameEntry};

pub const GALAXY_ID: &str = "100000000000000000";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AchievementStatus {
    pub earned: bool,
    pub earned_time: i64,
}

/// GOG emulator status entry: { "unlock_time": 1234567890 }
/// Only earned achievements appear; absent = not earned.
#[derive(Debug, Clone, Deserialize)]
pub struct GogAchievementStatus {
    #[serde(default)]
    pub unlock_time: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StringOrMap {
    pub val: String,
}

impl<'de> Deserialize<'de> for StringOrMap {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Map(HashMap<String, String>),
        }
        match Raw::deserialize(de)? {
            Raw::Str(s) => Ok(StringOrMap { val: s }),
            Raw::Map(m) => {
                let val = m
                    .get("english")
                    .cloned()
                    .or_else(|| m.into_iter().next().map(|(_, v)| v))
                    .unwrap_or_default();
                Ok(StringOrMap { val })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AchievementMeta {
    #[serde(default)]
    description: StringOrMap,
    #[serde(default, rename = "displayName")]
    display_name: StringOrMap,
    #[serde(default)]
    hidden: serde_json::Value,
    #[serde(default)]
    icon: String,
    #[serde(default, rename = "icongray")]
    icon_gray: String,
    #[serde(default, rename = "icon_gray")]
    icon_gray_alt: String,
    name: String,
}

#[derive(Debug, Clone)]
pub struct MergedAchievement {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub earned: bool,
    pub earned_time: i64,
    pub icon_path: String,
    pub icon_gray_path: String,
    pub global_percent: f64,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub app_id: String,
    pub kind: String,
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
}

fn parse_hidden(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        serde_json::Value::String(s) => s == "1" || s == "true",
        _ => false,
    }
}

pub fn convert_ico_to_png(ico_path: &Path) -> Result<PathBuf, String> {
    let png_path = ico_path.with_extension("png");
    if png_path.exists() {
        let _ = std::fs::remove_file(ico_path);
        return Ok(png_path);
    }
    let img = image::open(ico_path).map_err(|e| e.to_string())?;
    img.save(&png_path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(ico_path);
    Ok(png_path)
}

fn try_convert_ico(path: &Path) -> PathBuf {
    if path.extension().and_then(|e| e.to_str()) == Some("ico") {
        if let Ok(png) = convert_ico_to_png(path) {
            return png;
        }
    }
    path.to_path_buf()
}

// ---- Path helpers ----

pub fn data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join(app_id)
}

pub fn achievements_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir(save_dir, app_id).join("achievements")
}

pub fn unlock_status_path(save_dir: &str, kind: &str, app_id: &str, platform_id: &str) -> PathBuf {
    match kind {
        "gog" => Path::new(save_dir).join("gog").join(GALAXY_ID).join(platform_id).join("achievements.json"),
        _ => Path::new(save_dir).join("steam").join(app_id).join("achievements.json"),
    }
}

/// Read the game name from a cached `appdetails.json`, if present and non-empty.
#[derive(Deserialize)]
struct AppDetailsName {
    #[serde(default)]
    name: String,
}
pub fn read_app_name(save_dir: &str, app_id: &str) -> Option<String> {
    let path = data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    let details: AppDetailsName = serde_json::from_slice(&data).ok()?;
    let name = details.name.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn find_icon_path(ach_dir: &Path, icon_field: &str) -> String {
    if icon_field.is_empty() {
        return String::new();
    }
    if Path::new(icon_field).extension().is_none() {
        return String::new();
    }
    let path = ach_dir.join(icon_field);
    if path.is_file() {
        let converted = try_convert_ico(&path);
        return converted.to_string_lossy().into_owned();
    }

    let base = Path::new(icon_field).file_name().unwrap_or_default();
    let candidates = [
        ach_dir.join(base),
        ach_dir.join("achievement_images").join(base),
        ach_dir.join("img").join(base),
    ];
    for cand in &candidates {
        if cand.is_file() {
            let converted = try_convert_ico(cand);
            return converted.to_string_lossy().into_owned();
        }
    }
    String::new()
}

pub fn load_games(conn: &DbConn, save_dir: &str) -> Vec<Game> {
    let entries = match crate::db::load_all_games(conn) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to load games from DB: {}", e);
            return Vec::new();
        }
    };

    let mut games = Vec::new();
    for entry in entries {
        match load_game(&entry, save_dir) {
            Ok(game) => games.push(game),
            Err(e) => eprintln!("Skipping game {} ({}): {}", entry.steam_id, entry.kind, e),
        }
    }
    games.sort_by(|a, b| a.name.cmp(&b.name));
    games
}

pub fn load_game(entry: &GameEntry, save_dir: &str) -> Result<Game, String> {
    let app_id = &entry.steam_id;
    let kind = &entry.kind;
    let platform_id = &entry.platform_id;

    let mut game = Game {
        app_id: app_id.to_string(),
        kind: kind.to_string(),
        platform_id: platform_id.to_string(),
        db_id: entry.id,
        name: if entry.title.is_empty() {
            format!("App ID: {}", app_id)
        } else {
            entry.title.clone()
        },
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden: entry.hidden,
    };

    let ach_dir = achievements_dir(save_dir, app_id);

    // Title fallback from appdetails.json if DB title is empty
    if entry.title.is_empty() {
        if let Some(name) = read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }

    // Icon
    let icon_png = data_dir(save_dir, app_id).join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else {
        let icon_ico = data_dir(save_dir, app_id).join("icon.ico");
        if icon_ico.is_file() {
            if let Ok(png) = convert_ico_to_png(&icon_ico) {
                game.icon_path = png.to_string_lossy().into_owned();
            }
        }
    }

    // Grid assets
    let data_game_dir = data_dir(save_dir, app_id);
    let grid = data_game_dir.join("library_600x900.jpg");
    if grid.is_file() {
        game.grid_path = grid.to_string_lossy().into_owned();
    }
    let header = data_game_dir.join("header.jpg");
    if header.is_file() {
        game.header_path = header.to_string_lossy().into_owned();
    }
    let hero = data_game_dir.join("library_hero.jpg");
    if hero.is_file() {
        game.hero_image_path = hero.to_string_lossy().into_owned();
    }
    let logo = data_game_dir.join("logo.png");
    if logo.is_file() {
        game.logo_path = logo.to_string_lossy().into_owned();
    }

    // Unlock status
    let status_path = unlock_status_path(save_dir, kind, app_id, platform_id);
    let status_map = load_status_map(&status_path);

    // Achievement definitions
    let meta_path = ach_dir.join("achievements.json");
    if let Ok(meta_data) = std::fs::read(&meta_path) {
        if let Ok(meta_list) = serde_json::from_slice::<Vec<AchievementMeta>>(&meta_data) {
            for meta in meta_list {
                let status = status_map.get(&meta.name).cloned().unwrap_or_default();
                let hidden = parse_hidden(&meta.hidden);
                let icon_gray = if meta.icon_gray.is_empty() {
                    meta.icon_gray_alt.clone()
                } else {
                    meta.icon_gray.clone()
                };

                let ach = MergedAchievement {
                    name: meta.name.clone(),
                    display_name: meta.display_name.val.clone(),
                    description: meta.description.val.clone(),
                    hidden,
                    earned: status.earned,
                    earned_time: status.earned_time,
                    icon_path: find_icon_path(&ach_dir, &meta.icon),
                    icon_gray_path: find_icon_path(&ach_dir, &icon_gray),
                    global_percent: 0.0,
                };

                game.total_count += 1;
                if ach.earned {
                    game.earned_count += 1;
                }
                game.achievements.push(ach);
            }
        } else {
            eprintln!("Meta load error for {}", app_id);
        }
    } else {
        // No meta file — use status map keys as achievement names
        let mut keys: Vec<_> = status_map.keys().cloned().collect();
        keys.sort();
        for name in keys {
            let status = &status_map[&name];
            let ach = MergedAchievement {
                name: name.clone(),
                display_name: name.clone(),
                description: "No description available.".into(),
                hidden: false,
                earned: status.earned,
                earned_time: status.earned_time,
                icon_path: String::new(),
                icon_gray_path: String::new(),
                global_percent: 0.0,
            };
            game.total_count += 1;
            if ach.earned {
                game.earned_count += 1;
            }
            game.achievements.push(ach);
        }
    }

    Ok(game)
}

pub fn set_achievement_earned(save_dir: &str, kind: &str, app_id: &str, platform_id: &str, ach_name: &str, earned: bool) -> Result<(), String> {
    let status_path = unlock_status_path(save_dir, kind, app_id, platform_id);
    let mut status_map: HashMap<String, AchievementStatus> = HashMap::new();
    if let Ok(data) = std::fs::read(&status_path) {
        let _ = serde_json::from_slice::<HashMap<String, AchievementStatus>>(&data).map(|m| status_map = m);
    }
    status_map.insert(
        ach_name.to_string(),
        AchievementStatus {
            earned,
            earned_time: 0,
        },
    );
    let b = serde_json::to_string_pretty(&status_map).map_err(|e| e.to_string())?;
    std::fs::write(&status_path, b).map_err(|e| e.to_string())?;
    Ok(())
}

/// Reads achievement status from a achievements.json file. Handles both Goldberg
/// format ({ "name": { "earned": bool, "earned_time": N } }) and GOG emulator
/// format ({ "name": { "unlock_time": N } }, only earned listed, or null).
fn load_status_map(status_path: &Path) -> HashMap<String, AchievementStatus> {
    let Ok(data) = std::fs::read(status_path) else {
        return HashMap::new();
    };

    // GOG emulator writes `null` when nothing is earned yet.
    let trimmed = std::str::from_utf8(&data).unwrap_or("").trim();
    if trimmed == "null" || trimmed.is_empty() {
        return HashMap::new();
    }

    // Try Goldberg format first
    if let Ok(m) = serde_json::from_slice::<HashMap<String, AchievementStatus>>(&data) {
        return m;
    }

    // Try GOG format: { "name": { "unlock_time": N } }
    if let Ok(gog_m) = serde_json::from_slice::<HashMap<String, GogAchievementStatus>>(&data) {
        return gog_m
            .into_iter()
            .map(|(k, v)| {
                let earned = v.unlock_time > 0;
                (
                    k,
                    AchievementStatus {
                        earned,
                        earned_time: v.unlock_time,
                    },
                )
            })
            .collect();
    }

    HashMap::new()
}
