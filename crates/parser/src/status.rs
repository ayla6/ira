use ira_models::achievement::{AchievementStatus, GogAchievementStatus};
use std::collections::HashMap;
use std::path::Path;

pub fn load_status_map(status_path: &Path) -> HashMap<String, AchievementStatus> {
    let Ok(data) = std::fs::read(status_path) else {
        return HashMap::new();
    };

    let trimmed = std::str::from_utf8(&data).unwrap_or("").trim();
    if trimmed == "null" || trimmed.is_empty() {
        return HashMap::new();
    }

    if let Ok(m) = serde_json::from_slice::<HashMap<String, AchievementStatus>>(&data) {
        return m;
    }

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
