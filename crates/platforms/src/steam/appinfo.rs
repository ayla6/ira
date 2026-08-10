use std::collections::HashMap;

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
        return result;
    }

    let st_offset = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if st_offset + 4 > data.len() {
        return result;
    }

    let Some(type_idx) = find_string_index(&data, st_offset, "type") else {
        return result;
    };
    let idx_bytes = (type_idx as u32).to_le_bytes();
    let type_pattern = [
        0x01u8,
        idx_bytes[0],
        idx_bytes[1],
        idx_bytes[2],
        idx_bytes[3],
    ];

    let scan_end = st_offset.min(data.len());
    let requested: std::collections::HashSet<u32> = app_ids.iter().copied().collect();

    let mut pos = 24;
    while pos + 5 <= scan_end {
        if data[pos] == 0x08 {
            let app_id =
                u32::from_le_bytes([data[pos + 1], data[pos + 2], data[pos + 3], data[pos + 4]]);
            if requested.contains(&app_id) && !result.contains_key(&app_id) {
                let search_start = pos + 5;
                let search_end = (search_start + 3000).min(scan_end);
                let search_region = &data[search_start..search_end];
                if let Some(rel) = search_region.windows(5).position(|w| w == type_pattern) {
                    let value_start = search_start + rel + 5;
                    if let Some(null_pos) = data[value_start..].iter().position(|&b| b == 0) {
                        let value =
                            String::from_utf8_lossy(&data[value_start..value_start + null_pos])
                                .to_string();
                        result.insert(app_id, value);
                    }
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
    let data = std::fs::read(appinfo_path()?).ok()?;
    if data.len() < 24 {
        return None;
    }

    let st_offset = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if st_offset + 4 > data.len() {
        return None;
    }

    let ci_idx = find_string_index(&data, st_offset, "clienticon")?;
    let ci_bytes = (ci_idx as u32).to_le_bytes();
    let ci_pattern = [0x01u8, ci_bytes[0], ci_bytes[1], ci_bytes[2], ci_bytes[3]];

    let id_bytes = app_id.to_le_bytes();
    let search_pattern = [0x08u8, id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]];

    let scan_end = st_offset.min(data.len());
    let mut pos = 24;
    while pos + 5 <= scan_end {
        if data[pos..pos + 5] == search_pattern {
            let search_start = pos + 5;
            let search_end = (search_start + 5000).min(scan_end);
            let search_region = &data[search_start..search_end];
            if let Some(rel) = search_region.windows(5).position(|w| w == ci_pattern) {
                let value_start = search_start + rel + 5;
                if let Some(null_pos) = data[value_start..].iter().position(|&b| b == 0) {
                    return Some(
                        String::from_utf8_lossy(&data[value_start..value_start + null_pos])
                            .to_string(),
                    );
                }
            }
            break;
        }
        pos += 1;
    }
    None
}

fn appinfo_path() -> Option<std::path::PathBuf> {
    super::paths::steam_install_dir().map(|d| d.join("appcache").join("appinfo.vdf"))
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
