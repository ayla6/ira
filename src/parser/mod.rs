mod paths;
mod icons;
mod status;
mod date;

pub use paths::*;
pub use icons::*;
pub use status::*;
pub use date::*;

use crate::db::DbConn;
pub use crate::models::{AchievementStatus, Game, GameEntry, MergedAchievement};
use std::collections::HashMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct AppDetailsName {
    #[serde(default)]
    name: String,
}

pub fn read_app_name(save_dir: &str, app_id: &str) -> Option<String> {
    let path = paths::data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    let details: AppDetailsName = serde_json::from_slice(&data).ok()?;
    let name = details.name.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

pub fn read_app_details(save_dir: &str, app_id: &str) -> Option<crate::api::types::AppDetails> {
    let path = paths::data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn populate_image_paths(image_dir: &std::path::Path, game: &mut Game) {
    if game.icon_path.is_empty() {
        if let Some(p) = paths::find_image_file(image_dir, "icon") {
            game.icon_path = p.to_string_lossy().into_owned();
        }
    }
    if let Some(p) = paths::find_image_file(image_dir, "library_600x900") {
        game.grid_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = paths::find_image_file(image_dir, "header") {
        game.header_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = paths::find_image_file(image_dir, "library_hero") {
        game.hero_image_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = paths::find_image_file(image_dir, "logo") {
        game.logo_path = p.to_string_lossy().into_owned();
    }
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
        trophy_source: entry.trophy_source.clone(),
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
        lastplayed: entry.last_played,
        logo_position: entry.logo_position.clone(),
        logo_size: entry.logo_size,
        lutris_name: String::new(),
        manual_unmatch: entry.manual_unmatch == 1,
        sort_title: entry.sort_title.clone(),
        game_path: String::new(),
        sgdb_id: entry.sgdb_id.clone().unwrap_or_default(),
        shadps4_version: entry.shadps4_version.clone().unwrap_or_default(),
        release_date: entry.release_date.clone(),
        release_timestamp: entry.release_timestamp,
        metacritic_score: entry.metacritic_score,
        steam_review_score: entry.steam_review_score,
        steam_review_count: entry.steam_review_count,
        ra_core: entry.ra_core.clone(),
        emulator_override: entry.emulator_override.clone(),
        rom_path: entry.rom_path.clone(),
    };

    let ach_dir = paths::achievements_dir(save_dir, app_id);

    if entry.title.is_empty() {
        if let Some(name) = read_app_name(save_dir, app_id) {
            game.name = name;
        }
    }

    let image_dir = if kind == "ps4" {
        paths::ps4_data_dir(save_dir, app_id)
    } else if crate::models::has_steam_enrichment(&entry.trophy_source) {
        paths::data_dir(save_dir, app_id)
    } else if entry.sgdb_id.as_deref().filter(|s| !s.is_empty()).is_some() {
        paths::sgdb_data_dir(save_dir, entry.sgdb_id.as_deref().unwrap())
    } else {
        paths::data_dir(save_dir, app_id)
    };

    let icon_png = image_dir.join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else {
        let ra_icon = paths::ra_icon_path(save_dir, app_id);
        if ra_icon.is_file() {
            game.icon_path = ra_icon.to_string_lossy().into_owned();
        }
        let icon_ico = image_dir.join("icon.ico");
        if icon_ico.is_file() {
            match icons::convert_ico_to_png(&icon_ico) {
                Ok(png) => game.icon_path = png.to_string_lossy().into_owned(),
                Err(_) => {
                    let renamed = image_dir.join("icon.png");
                    if std::fs::rename(&icon_ico, &renamed).is_ok() {
                        game.icon_path = renamed.to_string_lossy().into_owned();
                    }
                }
            }
        } else {
            for ext in ["jpg", "webp"] {
                let p = image_dir.join(format!("icon.{}", ext));
                if p.is_file() {
                    game.icon_path = p.to_string_lossy().into_owned();
                    break;
                }
            }
        }
    }

    populate_image_paths(&image_dir, &mut game);

    let is_steam_native = entry.trophy_source == crate::models::STEAM_NATIVE;
    let steam_native_data = if is_steam_native {
        crate::platforms::steam::read_steam_achievements_full(app_id, save_dir)
    } else {
        crate::platforms::steam::SteamAchievementData { achievements: Vec::new(), n_total: 0, n_achieved: 0 }
    };

    let meta_path = ach_dir.join("achievements.json");
    let has_meta = meta_path.is_file();

    let status_map = if is_steam_native {
        let mut map: HashMap<String, AchievementStatus> = crate::platforms::steam::read_user_stats(app_id)
            .into_iter()
            .map(|(name, (earned, earned_time))| (name, AchievementStatus { earned, earned_time }))
            .collect();
        for ach in &steam_native_data.achievements {
            map.entry(ach.id.clone())
                .and_modify(|s| { if s.earned_time == 0 { s.earned_time = ach.earned_time; } })
                .or_insert(AchievementStatus { earned: ach.earned, earned_time: ach.earned_time });
        }
        map
    } else {
        let status_path = paths::unlock_status_path(save_dir, &entry.trophy_source, app_id, platform_id);
        status::load_status_map(&status_path)
    };

    if !has_meta && !steam_native_data.achievements.is_empty() {
        let icons_dir = ach_dir.join("achievement_images");
        for ach in &steam_native_data.achievements {
            let icon_path = if !ach.icon_url.is_empty() {
                let icon_file = icons_dir.join(format!("{}.jpg", ach.id));
                if icon_file.is_file() {
                    icon_file.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            game.achievements.push(MergedAchievement {
                name: ach.id.clone(),
                display_name: ach.display_name.clone(),
                description: ach.description.clone(),
                hidden: ach.hidden,
                earned: ach.earned,
                earned_time: ach.earned_time,
                icon_path,
                icon_gray_path: String::new(),
                global_percent: ach.global_percent,
                trophy_type: '\0',
            });
        }
    } else if has_meta {
        let meta_data = std::fs::read(&meta_path).map_err(|e| format!("read achievements.json: {}", e))?;
        let meta_list: Vec<crate::models::achievement::AchievementMeta> =
            serde_json::from_slice(&meta_data).map_err(|e| {
                eprintln!("Meta load error for {}", app_id);
                format!("parse achievements.json: {}", e)
            })?;

        for meta in meta_list {
            let status = status_map.get(&meta.name).cloned().unwrap_or_default();
            let hidden = crate::models::achievement::parse_hidden(&meta.hidden);
            let icon_gray = if meta.icon_gray.is_empty() {
                meta.icon_gray_alt.clone()
            } else {
                meta.icon_gray.clone()
            };

            game.achievements.push(MergedAchievement {
                name: meta.name.clone(),
                display_name: meta.display_name.val.clone(),
                description: meta.description.val.clone(),
                hidden,
                earned: status.earned,
                earned_time: status.earned_time,
                icon_path: icons::find_icon_path(&ach_dir, &meta.icon),
                icon_gray_path: icons::find_icon_path(&ach_dir, &icon_gray),
                global_percent: 0.0,
                trophy_type: '\0',
            });
        }
    } else {
        let mut keys: Vec<_> = status_map.keys().cloned().collect();
        keys.sort();
        for name in keys {
            let status = &status_map[&name];
            game.achievements.push(MergedAchievement {
                name: name.clone(),
                display_name: name.clone(),
                description: "No description available.".into(),
                hidden: false,
                earned: status.earned,
                earned_time: status.earned_time,
                icon_path: String::new(),
                icon_gray_path: String::new(),
                global_percent: 0.0,
                trophy_type: '\0',
            });
        }
    }

    game.total_count = game.achievements.len();
    game.earned_count = game.achievements.iter().filter(|a| a.earned).count();

    if is_steam_native && steam_native_data.n_total > 0 {
        game.total_count = steam_native_data.n_total;
        game.earned_count = steam_native_data.n_achieved;
    }

    Ok(game)
}

pub fn set_achievement_earned(save_dir: &str, trophy_source: &str, app_id: &str, platform_id: &str, ach_name: &str, earned: bool) -> Result<(), String> {
    let status_path = paths::unlock_status_path(save_dir, trophy_source, app_id, platform_id);
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
