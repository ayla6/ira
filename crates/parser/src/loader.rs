use ira_models::{AchievementStatus, AssetType, Game};
use serde::Deserialize;
use std::collections::HashMap;

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
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

pub fn populate_image_paths(image_dir: &std::path::Path, game: &mut Game) {
    let db_id = game.db_id;
    let _s = tracing::info_span!("populate_image_paths", db_id).entered();

    let mut files: std::collections::HashSet<String> = std::fs::read_dir(image_dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for &at in AssetType::all() {
        let base = at.file_base();
        let (max_w, max_h) = at.thumb_dims();
        let small_webp = format!("{}_small.webp", base);
        let small_jpg = format!("{}_small.jpg", base);
        if !files.contains(&small_webp) && !files.contains(&small_jpg) {
            super::paths::ensure_small_image(image_dir, base, max_w, max_h);
            if image_dir.join(&small_webp).is_file() {
                files.insert(small_webp);
            }
            if image_dir.join(&small_jpg).is_file() {
                files.insert(small_jpg);
            }
        }
    }

    let find = |base: &str| -> Option<std::path::PathBuf> {
        for ext in &["webp", "jpg"] {
            let name = format!("{}.{}", base, ext);
            if files.contains(&name) {
                return Some(image_dir.join(name));
            }
        }
        None
    };

    let found: Vec<(AssetType, String)> = AssetType::all()
        .iter()
        .filter_map(|&at| {
            find(&format!("{}_small", at.file_base()))
                .or_else(|| find(at.file_base()))
                .map(|p| (at, p.to_string_lossy().into_owned()))
        })
        .collect();
    for (at, p) in found {
        match at {
            AssetType::Icon => game.icon_path = p,
            AssetType::Hero => game.hero_image_path = p,
            AssetType::Grid => game.grid_path = p,
            AssetType::Header => game.header_path = p,
            AssetType::Logo => game.logo_path = p,
        }
    }
}

pub fn set_achievement_earned(
    save_dir: &str,
    trophy_source: ira_models::TrophySource,
    app_id: &str,
    platform_id: &str,
    ach_name: &str,
    earned: bool,
) -> Result<(), String> {
    let status_path =
        super::paths::unlock_status_path(save_dir, trophy_source, app_id, platform_id);
    let mut status_map: HashMap<String, AchievementStatus> = HashMap::new();
    if let Ok(data) = std::fs::read(&status_path) {
        let _ = serde_json::from_slice::<HashMap<String, AchievementStatus>>(&data)
            .map(|m| status_map = m);
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
