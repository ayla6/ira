use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AchievementStatus {
    pub earned: bool,
    #[serde(rename = "earned_time")]
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
    pub game_dir: String,
    pub name: String,
    pub icon_path: String,
    pub hero_image_path: String,
    pub achievements: Vec<MergedAchievement>,
    pub earned_count: usize,
    pub total_count: usize,
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
        return Ok(png_path);
    }
    let img = image::open(ico_path).map_err(|e| e.to_string())?;
    img.save(&png_path).map_err(|e| e.to_string())?;
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

fn find_icon_path(game_dir: &Path, icon_field: &str) -> String {
    if icon_field.is_empty() {
        return String::new();
    }
    if Path::new(icon_field).extension().is_none() {
        return String::new();
    }
    let path = game_dir.join("steam_settings").join(icon_field);
    if path.is_file() {
        let converted = try_convert_ico(&path);
        return converted.to_string_lossy().into_owned();
    }

    let base = Path::new(icon_field).file_name().unwrap_or_default();
    let candidates = [
        game_dir.join("steam_settings").join(base),
        game_dir.join("steam_settings").join("achievement_images").join(base),
        game_dir.join("steam_settings").join("img").join(base),
    ];
    for cand in &candidates {
        if cand.is_file() {
            let converted = try_convert_ico(cand);
            return converted.to_string_lossy().into_owned();
        }
    }
    String::new()
}

/// Scans both steam/ and gog/ subdirectories under base_path for games.
pub fn load_games(base_path: &str) -> Vec<Game> {
    let mut games: Vec<Game> = Vec::new();

    // Steam games: base_path/steam/<app_id>/
    let steam_dir = format!("{}/steam", base_path);
    if let Ok(entries) = std::fs::read_dir(&steam_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let app_id = entry.file_name().to_string_lossy().into_owned();
            if app_id.parse::<i64>().is_err() {
                continue;
            }
            let game_dir = entry.path();
            match load_game(&app_id, &game_dir) {
                Ok(mut game) => {
                    game.game_dir = game_dir.to_string_lossy().into_owned();
                    games.push(game);
                }
                Err(e) => eprintln!("Skipping steam game {}: {}", app_id, e),
            }
        }
    }

    // GOG games: base_path/gog/<galaxyid>/<productid>/
    let gog_dir = format!("{}/gog", base_path);
    if let Ok(galaxy_entries) = std::fs::read_dir(&gog_dir) {
        for galaxy_entry in galaxy_entries.flatten() {
            if !galaxy_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let galaxy_path = galaxy_entry.path();
            if let Ok(product_entries) = std::fs::read_dir(&galaxy_path) {
                for product_entry in product_entries.flatten() {
                    if !product_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let game_dir = product_entry.path();
                    // Read Steam App ID from steam_appid.txt
                    let appid_path = game_dir.join("steam_appid.txt");
                    let app_id = match std::fs::read_to_string(&appid_path) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue, // Not a registered GOG game
                    };
                    if app_id.parse::<i64>().is_err() {
                        continue;
                    }
                    match load_game(&app_id, &game_dir) {
                        Ok(mut game) => {
                            game.game_dir = game_dir.to_string_lossy().into_owned();
                            games.push(game);
                        }
                        Err(e) => eprintln!("Skipping gog game {}: {}", app_id, e),
                    }
                }
            }
        }
    }

    games.sort_by(|a, b| a.name.cmp(&b.name));
    games
}

pub fn set_achievement_earned(game_dir: &str, ach_name: &str, earned: bool) -> Result<(), String> {
    let status_path = Path::new(game_dir).join("achievements.json");
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

/// Reads achievement status from achievements.json. Handles both Goldberg
/// format ({ "name": { "earned": bool, "earned_time": N } }) and GOG emulator
/// format ({ "name": { "unlock_time": N } }, only earned listed, or null).
fn load_status_map(game_dir: &Path) -> HashMap<String, AchievementStatus> {
    let status_path = game_dir.join("achievements.json");
    let Ok(data) = std::fs::read(&status_path) else {
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

pub fn load_game(app_id: &str, game_dir: &Path) -> Result<Game, String> {
    let mut game = Game {
        app_id: app_id.to_string(),
        game_dir: game_dir.to_string_lossy().into_owned(),
        name: format!("App ID: {}", app_id),
        icon_path: String::new(),
        hero_image_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
    };

    let title_path = game_dir.join("steam_settings").join("title.txt");
    if let Ok(data) = std::fs::read_to_string(&title_path) {
        let trimmed = data.trim();
        if !trimmed.is_empty() {
            game.name = trimmed.to_string();
        }
    }

    let icon_png = game_dir.join("steam_settings").join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else {
        let icon_ico = game_dir.join("steam_settings").join("icon.ico");
        if icon_ico.is_file() {
            if let Ok(png) = convert_ico_to_png(&icon_ico) {
                game.icon_path = png.to_string_lossy().into_owned();
            }
        }
    }

    let status_map = load_status_map(game_dir);

    let meta_path = game_dir.join("steam_settings").join("achievements.json");
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
                    icon_path: find_icon_path(game_dir, &meta.icon),
                    icon_gray_path: find_icon_path(game_dir, &icon_gray),
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
