use std::path::Path;

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

/// Read the FULL achievement unlock state from Steam's binary stats files.
///
/// Steam stores achievement data in two binary files under `appcache/stats/`:
/// - `UserGameStatsSchema_<appid>.bin` — maps bit positions to achievement API names
/// - `UserGameStats_<steam_id>_<appid>.bin` — contains unlock timestamps for achieved achievements
///
/// Unlike the librarycache JSON (which only has ~15 visible achievements), these files
/// contain the complete set. No API key or network access required.
///
/// Returns a map of achievement API name → (earned, unlock_time).
pub fn read_user_stats(app_id: &str) -> std::collections::HashMap<String, (bool, i64)> {
    let mut result = std::collections::HashMap::new();

    let Some(schema_path) = paths::stats_schema_path(app_id) else {
        eprintln!("[steam] read_user_stats: cannot find stats dir");
        return result;
    };
    let Ok(schema_data) = std::fs::read(&schema_path) else {
        eprintln!("[steam] read_user_stats: cannot read schema {}", schema_path.display());
        return result;
    };

    let bit_to_name = match parse_schema(&schema_data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[steam] read_user_stats: schema parse error for {}: {}", app_id, e);
            return result;
        }
    };

    if bit_to_name.is_empty() {
        eprintln!("[steam] read_user_stats: no achievements found in schema for {}", app_id);
        return result;
    }

    let user_ids = paths::get_steam_user_ids();
    for steam_id in &user_ids {
        let Some(stats_path) = paths::stats_user_path(steam_id, app_id) else { continue };
        let Ok(stats_data) = std::fs::read(&stats_path) else { continue };

        let achieved = match parse_user_stats(&stats_data) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[steam] read_user_stats: user stats parse error for {}: {}", app_id, e);
                continue;
            }
        };

        for (bit_pos, name) in &bit_to_name {
            let earned = achieved.contains_key(bit_pos);
            let unlock_time = achieved.get(bit_pos).copied().unwrap_or(0);
            result.insert(name.clone(), (earned, unlock_time));
        }
        break;
    }

    if result.is_empty() {
        eprintln!("[steam] read_user_stats: no user stats file found for {} (tried {} users)", app_id, user_ids.len());
        for (_, name) in &bit_to_name {
            result.insert(name.clone(), (false, 0));
        }
    }

    result
}

/// Parse a UserGameStatsSchema binary file.
/// Returns a map of bit position → achievement API name.
///
/// The schema contains one or more sections, each starting with a `bits` field
/// that gives the starting bit offset. Achievement names follow as `\x01name\x00<API_NAME>\x00`.
fn parse_schema(data: &[u8]) -> Result<std::collections::HashMap<usize, String>, String> {
    let mut bit_to_name = std::collections::HashMap::new();

    let mut pos = 0;
    while pos < data.len() {
        let bits_idx = find_bytes(data, b"bits\x00", pos).ok_or("no bits field")?;

        let val_start = skip_nulls(data, bits_idx + 5);
        let val_end = data[val_start..].iter()
            .position(|&b| b == 0)
            .ok_or("bits value not null-terminated")?;
        let bits_val: usize = std::str::from_utf8(&data[val_start..val_start + val_end])
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;

        let next_bits = find_bytes(data, b"bits\x00", val_start)
            .unwrap_or(data.len());

        let mut idx_in_section = 0;
        let mut search_pos = bits_idx;
        while search_pos < next_bits {
            match find_bytes(data, b"\x01name\x00", search_pos) {
                Some(name_marker_pos) if name_marker_pos < next_bits => {
                    let name_start = name_marker_pos + 6;
                    let name_end = data[name_start..next_bits].iter()
                        .position(|&b| b == 0)
                        .ok_or("name not null-terminated")?;
                    let name = std::str::from_utf8(&data[name_start..name_start + name_end])
                        .map_err(|e| e.to_string())?
                        .to_string();

                    if !name.starts_with("display") {
                        bit_to_name.insert(bits_val + idx_in_section, name);
                        idx_in_section += 1;
                    }
                    search_pos = name_start + name_end + 1;
                }
                _ => break,
            }
        }

        pos = next_bits;
    }

    Ok(bit_to_name)
}

/// Parse a UserGameStats binary file.
/// Returns a map of bit position → unlock timestamp (only achieved achievements).
///
/// Entries are: `\x02<index_str>\x00<4-byte LE timestamp>`.
/// Non-numeric entries (like "crc", "data", "PendingChanges") are skipped.
fn parse_user_stats(data: &[u8]) -> Result<std::collections::HashMap<usize, i64>, String> {
    let mut achieved = std::collections::HashMap::new();

    let mut pos = 0;
    while pos < data.len() {
        if data[pos] != 0x02 {
            pos += 1;
            continue;
        }
        pos += 1;

        let end = data[pos..].iter()
            .position(|&b| b == 0)
            .ok_or("index string not null-terminated")?;
        let idx_str = std::str::from_utf8(&data[pos..pos + end])
            .map_err(|e| e.to_string())?;
        pos += end + 1;

        let Ok(bit_pos) = idx_str.parse::<usize>() else {
            continue;
        };

        if pos + 4 > data.len() {
            break;
        }
        let ts = i64::from(u32::from_le_bytes([
            data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
        ]));
        achieved.insert(bit_pos, ts);
        pos += 4;
    }

    Ok(achieved)
}

/// Find a byte pattern in data starting from `from`. Returns the byte offset.
fn find_bytes(data: &[u8], pattern: &[u8], from: usize) -> Option<usize> {
    if from >= data.len() || pattern.is_empty() {
        return None;
    }
    data[from..].windows(pattern.len())
        .position(|w| w == pattern)
        .map(|pos| from + pos)
}

fn skip_nulls(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && data[pos] == 0 {
        pos += 1;
    }
    pos
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

        if let Some(obj) = data_obj.as_object() {
            let vec_keys: Vec<String> = obj.keys()
                .filter(|k| k.starts_with("vec"))
                .map(|k| k.to_string())
                .collect();

            for vec_name in &vec_keys {
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
        }
        break;
    }

    Ok((result, n_total, n_achieved))
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

    #[test]
    fn test_parse_schema_balatro() {
        let schema = b"\x002379780\x00\x00stats\x00\x001\x00\x00bits\x00\x0030\x00\x01name\x00BAL_01\x00\x00display\x00\x00name\x00\x01english\x00Ante Up!\x00\x00stats\x00\x002\x00\x00bits\x00\x000\x00\x01name\x00BAL_03\x00\x00display\x00\x00name\x00\x01english\x00Heads Up\x00";
        let bit_to_name = parse_schema(schema).unwrap();
        assert_eq!(bit_to_name.len(), 2);
        assert_eq!(bit_to_name[&30], "BAL_01");
        assert_eq!(bit_to_name[&0], "BAL_03");
    }

    #[test]
    fn test_parse_user_stats_balatro() {
        let mut data = Vec::new();
        data.extend_from_slice(b"\x00cache\x00\x02crc\x00\x2e\x7f\x28\xab");
        data.extend_from_slice(b"\x02PendingChanges\x00\x00\x00\x00\x00");
        data.extend_from_slice(b"\x002\x00\x02data\x00\x00\x00\x00\xc0");
        data.extend_from_slice(b"\x02AchievementTimes\x00\x0230\x00\xa3\x0f\x77\x67");
        data.extend_from_slice(b"\x0231\x00\xc6\x12\x77\x67");
        data.extend_from_slice(b"\x0808\x00\x002\x00\x02data\x00\xff\xff\xef\x07");
        data.extend_from_slice(b"\x00AchievementTimes\x00\x020\x00\xc2\x13\x77\x67");
        data.extend_from_slice(b"\x021\x00\x0d\x84\x77\x67");
        data.extend_from_slice(b"\x022\x00\x0d\x84\x77\x67");

        let achieved = parse_user_stats(&data).unwrap();
        assert_eq!(achieved.len(), 5);
        let ts30 = achieved[&30];
        let ts0 = achieved[&0];
        let ts1 = achieved[&1];
        let ts2 = achieved[&2];
        let ts31 = achieved[&31];

        assert!(ts30 > 0);
        assert!(ts0 > 0);
        assert!(ts1 > 0);
        assert!(ts2 > 0);
        assert!(ts31 > 0);
        assert_ne!(ts0, ts1);
        assert_eq!(ts1, ts2);
        assert_ne!(ts30, ts31);
    }

    #[test]
    fn test_parse_user_stats_skips_non_numeric() {
        let data = b"\x02crc\x00\x2e\x7f\x28\xab\x02data\x00\x00\x00\x00\x00\x025\x00\x01\x00\x00\x00";
        let achieved = parse_user_stats(data).unwrap();
        assert_eq!(achieved.len(), 1);
        assert!(achieved.contains_key(&5));
    }

    #[test]
    fn test_read_user_stats_combines_schema_and_stats() {
        let schema = b"\x00app\x00\x00stats\x00\x001\x00\x00bits\x00\x0030\x00\x01name\x00ACH_A\x00\x00display\x00\x00name\x00\x01english\x00Test A\x00\x00stats\x00\x002\x00\x00bits\x00\x000\x00\x01name\x00ACH_B\x00\x00display\x00\x00name\x00\x01english\x00Test B\x00";
        let user_stats = b"\x02crc\x00\x00\x00\x00\x00\x020\x00\x01\x02\x03\x04\x0230\x00\x05\x06\x07\x08";

        let bit_to_name = parse_schema(schema).unwrap();
        assert_eq!(bit_to_name.len(), 2);
        assert_eq!(bit_to_name[&0], "ACH_B");
        assert_eq!(bit_to_name[&30], "ACH_A");

        let achieved = parse_user_stats(user_stats).unwrap();
        assert_eq!(achieved.len(), 2);
        assert!(achieved.contains_key(&0));
        assert!(achieved.contains_key(&30));

        let earned_a = achieved.contains_key(&30);
        let earned_b = achieved.contains_key(&0);
        assert!(earned_a);
        assert!(earned_b);
    }
}
