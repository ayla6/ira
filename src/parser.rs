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
    /// Lutris internal game id (0 = not linked / unmatched).
    pub lutris_id: i64,
    pub slug: String,
    /// Playtime in hours (from Lutris).
    pub playtime: f64,
    /// Unix timestamp of last play (from Lutris).
    pub lastplayed: i64,
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
}

impl Game {
    pub fn sort_key(&self) -> &str {
        if self.sort_title.is_empty() { &self.name } else { &self.sort_title }
    }
}

/// A Lutris game with no matched achievement source yet — shown in the sidebar
/// with no achievements until the user matches it to a Steam/GOG app id.
pub fn unmatched_game(lutris_id: i64, name: &str, slug: &str, playtime: f64, lastplayed: i64) -> Game {
    Game {
        app_id: String::new(),
        kind: String::new(),
        platform_id: String::new(),
        db_id: 0,
        name: name.to_string(),
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden: false,
        lutris_id,
        slug: slug.to_string(),
        playtime,
        lastplayed,
        logo_position: String::new(),
        logo_size: 0,
        lutris_name: name.to_string(),
        manual_unmatch: false,
        sort_title: String::new(),
    }
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
    Path::new(save_dir).join("data").join("steam").join(app_id)
}

pub fn ps4_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ps4").join(app_id)
}

pub fn sgdb_data_dir(save_dir: &str, sgdb_id: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("steamgriddb").join(sgdb_id)
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

/// Read the full `appdetails.json` (name, languages, DLCs) from disk.
pub fn read_app_details(save_dir: &str, app_id: &str) -> Option<crate::steam::AppDetails> {
    let path = data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    serde_json::from_slice(&data).ok()
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
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
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
        lutris_id: entry.lutris_db_id.unwrap_or(0),
        slug: String::new(),
        playtime: 0.0,
        lastplayed: 0,
        logo_position: entry.logo_position.clone(),
        logo_size: entry.logo_size,
        lutris_name: String::new(),
        manual_unmatch: entry.manual_unmatch.unwrap_or(0) == 1,
        sort_title: entry.sort_title.clone(),
    };

    let ach_dir = achievements_dir(save_dir, app_id);

    // Title fallback from appdetails.json if DB title is empty
    if entry.title.is_empty() {
        if let Some(name) = read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }

    // Use the correct data directory based on kind / sgdb_id
    let image_dir = match entry.sgdb_id.as_ref().filter(|s| !s.is_empty()) {
        Some(sgdb_id) => sgdb_data_dir(save_dir, sgdb_id),
        None => {
            if kind == "sgdb" {
                sgdb_data_dir(save_dir, app_id)
            } else if kind == "ps4" {
                ps4_data_dir(save_dir, app_id)
            } else {
                data_dir(save_dir, app_id)
            }
        }
    };

    // Icon — try .png first, then .ico (with conversion), then other formats
    let icon_png = image_dir.join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else {
        let icon_ico = image_dir.join("icon.ico");
        if icon_ico.is_file() {
            match convert_ico_to_png(&icon_ico) {
                Ok(png) => game.icon_path = png.to_string_lossy().into_owned(),
                Err(_) => {
                    // Conversion failed — try renaming (might be PNG with .ico extension)
                    let renamed = image_dir.join("icon.png");
                    if std::fs::rename(&icon_ico, &renamed).is_ok() {
                        game.icon_path = renamed.to_string_lossy().into_owned();
                    }
                }
            }
        } else {
            // Try .jpg, .webp
            for ext in ["jpg", "webp"] {
                let p = image_dir.join(format!("icon.{}", ext));
                if p.is_file() {
                    game.icon_path = p.to_string_lossy().into_owned();
                    break;
                }
            }
        }
    }

    // Grid assets
    let grid = image_dir.join("library_600x900.jpg");
    if grid.is_file() {
        game.grid_path = grid.to_string_lossy().into_owned();
    }
    let header = image_dir.join("header.jpg");
    if header.is_file() {
        game.header_path = header.to_string_lossy().into_owned();
    }
    let hero = image_dir.join("library_hero.jpg");
    if hero.is_file() {
        game.hero_image_path = hero.to_string_lossy().into_owned();
    }
    let logo = image_dir.join("logo.png");
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
