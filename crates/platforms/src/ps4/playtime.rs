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
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            result.insert(parts[0].to_string(), parts[1].to_string());
        }
    }
    result
}

/// Parse "H:MM:SS" → hours as f64
pub fn parse_playtime(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().unwrap_or(0.0);
            let m: f64 = parts[1].parse().unwrap_or(0.0);
            let s: f64 = parts[2].parse().unwrap_or(0.0);
            h + m / 60.0 + s / 3600.0
        }
        2 => {
            let m: f64 = parts[0].parse().unwrap_or(0.0);
            let s: f64 = parts[1].parse().unwrap_or(0.0);
            m / 60.0 + s / 3600.0
        }
        _ => 0.0,
    }
}
