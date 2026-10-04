use std::collections::HashMap;

use crate::ps4::play_time_path;

/// Read play_time.txt, returning CUSA_ID → playtime string ("H:MM:SS").
pub fn read_play_times() -> HashMap<String, String> {
    read_play_times_from_path(&play_time_path())
}

pub fn read_play_times_from_path(path: &std::path::Path) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return result,
    };
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((id, time)) = line.split_once(' ') {
            result.insert(id.to_string(), time.to_string());
        }
    }
    result
}

/// Parse "H:MM:SS" (or "M:SS") → hours as f64
pub fn parse_playtime(time_str: &str) -> f64 {
    let mut parts = time_str.split(':');
    let (Some(first), Some(second)) = (parts.next(), parts.next()) else {
        return 0.0;
    };
    match (parts.next(), parts.next()) {
        (Some(secs), None) => {
            let h: f64 = first.parse().unwrap_or(0.0);
            let m: f64 = second.parse().unwrap_or(0.0);
            let s: f64 = secs.parse().unwrap_or(0.0);
            h + m / 60.0 + s / 3600.0
        }
        (None, None) => {
            let m: f64 = first.parse().unwrap_or(0.0);
            let s: f64 = second.parse().unwrap_or(0.0);
            m / 60.0 + s / 3600.0
        }
        _ => 0.0,
    }
}
