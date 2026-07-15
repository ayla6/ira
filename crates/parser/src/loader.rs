use ira_models::{AchievementStatus, Game};
use std::collections::HashMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct AppDetailsName {
    #[serde(default)]
    name: String,
}

pub fn read_app_name(save_dir: &str, app_id: &str) -> Option<String> {
    let path = super::paths::data_dir(save_dir, app_id).join("appdetails.json");
    let data = std::fs::read(&path).ok()?;
    let details: AppDetailsName = serde_json::from_slice(&data).ok()?;
    let name = details.name.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

pub fn populate_image_paths(image_dir: &std::path::Path, game: &mut Game) {
    if game.icon_path.is_empty() {
        if let Some(p) = super::paths::find_image_file(image_dir, "icon") {
            game.icon_path = p.to_string_lossy().into_owned();
        }
    }
    if let Some(p) = super::paths::find_image_file(image_dir, "library_600x900") {
        game.grid_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = super::paths::find_image_file(image_dir, "header") {
        game.header_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = super::paths::find_image_file(image_dir, "library_hero") {
        game.hero_image_path = p.to_string_lossy().into_owned();
    }
    if let Some(p) = super::paths::find_image_file(image_dir, "logo") {
        game.logo_path = p.to_string_lossy().into_owned();
    }
}

pub fn set_achievement_earned(save_dir: &str, trophy_source: ira_models::TrophySource, app_id: &str, platform_id: &str, ach_name: &str, earned: bool) -> Result<(), String> {
    let status_path = super::paths::unlock_status_path(save_dir, trophy_source, app_id, platform_id);
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
