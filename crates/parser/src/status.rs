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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_status_map_goldberg_format() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"ACH_NAME":{"earned":true,"earned_time":12345},"ACH_NAME2":{"earned":false,"earned_time":0}}"#,
        )
        .unwrap();
        let map = load_status_map(tmp.path());
        assert_eq!(map.len(), 2);
        assert!(map["ACH_NAME"].earned);
        assert_eq!(map["ACH_NAME"].earned_time, 12345);
        assert!(!map["ACH_NAME2"].earned);
        assert_eq!(map["ACH_NAME2"].earned_time, 0);
    }

    #[test]
    fn test_load_status_map_gog_format() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"ACH_ONE":{"unlock_time":67890},"ACH_TWO":{"unlock_time":0}}"#,
        )
        .unwrap();
        let map = load_status_map(tmp.path());
        assert_eq!(map.len(), 2);
        assert!(map["ACH_ONE"].earned);
        assert_eq!(map["ACH_ONE"].earned_time, 67890);
        assert!(!map["ACH_TWO"].earned);
        assert_eq!(map["ACH_TWO"].earned_time, 0);
    }

    #[test]
    fn test_load_status_map_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"").unwrap();
        let map = load_status_map(tmp.path());
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_status_map_nonexistent_file() {
        let path = Path::new("/tmp/__nonexistent_achievements_file__.json");
        let map = load_status_map(path);
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_status_map_invalid_json() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is not json").unwrap();
        let map = load_status_map(tmp.path());
        assert!(map.is_empty());
    }
}
