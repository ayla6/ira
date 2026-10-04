use std::collections::{HashMap, HashSet};

/// Look up app types from Steam's appinfo.vdf binary cache.
/// Scans the file once for all requested app IDs instead of per-app.
pub fn get_app_types(app_ids: &[u32]) -> HashMap<u32, String> {
    let mut result = HashMap::new();
    if app_ids.is_empty() {
        return result;
    }

    let Some(path) = appinfo_path() else {
        return result;
    };
    let Ok(data) = std::fs::read(&path) else {
        return result;
    };
    if data.len() < 24 {
        eprintln!(
            "[steam] appinfo.vdf too short ({} bytes): {}",
            data.len(),
            path.display()
        );
        return result;
    }

    let Some(st_offset) = string_table_offset(&data) else {
        return result;
    };

    let Some(type_idx) = find_string_index(&data, st_offset, "type") else {
        return result;
    };
    let type_pattern = key_pattern(type_idx);

    let scan_end = st_offset.min(data.len());
    let requested: HashSet<u32> = app_ids.iter().copied().collect();

    let mut pos = 24;
    while pos + 5 <= scan_end {
        if data[pos] == 0x08 {
            let app_id =
                u32::from_le_bytes([data[pos + 1], data[pos + 2], data[pos + 3], data[pos + 4]]);
            if requested.contains(&app_id) && !result.contains_key(&app_id) {
                let search_start = pos + 5;
                let search_end = (search_start + 3000).min(scan_end);
                if let Some(value) =
                    value_after_pattern(&data, search_start, search_end, &type_pattern)
                {
                    result.insert(app_id, value);
                }
            }
        }
        pos += 1;
    }

    result
}

/// Get the clienticon hash for a specific app from appinfo.vdf.
/// This hash maps to a .ico file at steam/games/<hash>.ico
pub fn get_clienticon(app_id: u32) -> Option<String> {
    let data = read_appinfo_data()?;
    let st_offset = string_table_offset(&data)?;

    let ci_idx = find_string_index(&data, st_offset, "clienticon")?;
    let ci_pattern = key_pattern(ci_idx);

    let id_bytes = app_id.to_le_bytes();
    let search_pattern = [0x08u8, id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]];

    let scan_end = st_offset.min(data.len());
    let mut pos = 24;
    while pos + 5 <= scan_end {
        if data[pos..pos + 5] == search_pattern {
            let search_start = pos + 5;
            let search_end = (search_start + 5000).min(scan_end);
            return value_after_pattern(&data, search_start, search_end, &ci_pattern);
        }
        pos += 1;
    }
    None
}

fn appinfo_path() -> Option<std::path::PathBuf> {
    super::paths::steam_install_dir().map(|d| d.join("appcache").join("appinfo.vdf"))
}

/// Read the raw appinfo.vdf bytes. Missing files stay silent (Steam may not
/// be installed); corrupt headers are reported since they indicate a real
/// problem with an existing install.
fn read_appinfo_data() -> Option<Vec<u8>> {
    let path = appinfo_path()?;
    let data = std::fs::read(&path).ok()?;
    if data.len() < 24 {
        eprintln!(
            "[steam] appinfo.vdf too short ({} bytes): {}",
            data.len(),
            path.display()
        );
        return None;
    }
    Some(data)
}

/// Offset of the string table within appinfo.vdf, with bounds checking.
fn string_table_offset(data: &[u8]) -> Option<usize> {
    let st_offset = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if st_offset + 4 > data.len() {
        eprintln!("[steam] appinfo.vdf has a bad string-table offset");
        return None;
    }
    Some(st_offset)
}

/// VDF key-reference pattern: `0x01` followed by the string-table index.
fn key_pattern(index: usize) -> [u8; 5] {
    let bytes = (index as u32).to_le_bytes();
    [0x01u8, bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// Find `pattern` inside `data[search_start..search_end]` and return the
/// null-terminated string that follows it.
fn value_after_pattern(
    data: &[u8],
    search_start: usize,
    search_end: usize,
    pattern: &[u8; 5],
) -> Option<String> {
    let search_region = data.get(search_start..search_end)?;
    let rel = search_region.windows(5).position(|w| w == pattern)?;
    let value_start = search_start + rel + 5;
    let null_pos = data[value_start..].iter().position(|&b| b == 0)?;
    Some(
        String::from_utf8_lossy(&data[value_start..value_start + null_pos]).to_string(),
    )
}

fn find_string_index(data: &[u8], table_offset: usize, target: &str) -> Option<usize> {
    if table_offset + 4 > data.len() {
        return None;
    }
    let size = u32::from_le_bytes([
        data[table_offset],
        data[table_offset + 1],
        data[table_offset + 2],
        data[table_offset + 3],
    ]) as usize;
    let strings_start = table_offset + 4;
    let strings_end = (strings_start + size).min(data.len());

    let mut index = 0;
    let mut pos = strings_start;
    while pos < strings_end {
        let end = data[pos..strings_end].iter().position(|&b| b == 0)?;
        let s = std::str::from_utf8(&data[pos..pos + end]).ok()?;
        if s == target {
            return Some(index);
        }
        pos = pos + end + 1;
        index += 1;
    }
    None
}

/// Check if an app type string represents a game or demo (not a tool/utility).
pub fn is_game_or_demo(app_type: &str) -> bool {
    let lower = app_type.to_lowercase();
    lower == "game" || lower == "demo"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_game_or_demo_accepts_games_and_demos() {
        assert!(is_game_or_demo("game"));
        assert!(is_game_or_demo("Game"));
        assert!(is_game_or_demo("demo"));
        assert!(is_game_or_demo("DEMO"));
        assert!(!is_game_or_demo("tool"));
        assert!(!is_game_or_demo(""));
        assert!(!is_game_or_demo("game demo"));
    }

    #[test]
    fn test_key_pattern_prefixes_index() {
        assert_eq!(key_pattern(0), [0x01, 0, 0, 0, 0]);
        assert_eq!(key_pattern(0x01020304), [0x01, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_value_after_pattern_reads_null_terminated() {
        let pattern = key_pattern(7);
        let mut data = vec![0u8; 10];
        data.extend_from_slice(&pattern);
        data.extend_from_slice(b"game\x00trailing");
        assert_eq!(
            value_after_pattern(&data, 10, data.len(), &pattern).as_deref(),
            Some("game")
        );
        assert!(value_after_pattern(&data, 10, 12, &pattern).is_none());
    }

    #[test]
    fn test_string_table_offset_rejects_bad_offset() {
        let mut data = vec![0u8; 24];
        data[8..12].copy_from_slice(&9999u32.to_le_bytes());
        assert!(string_table_offset(&data).is_none());
    }
}
