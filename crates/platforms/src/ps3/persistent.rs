use std::collections::HashMap;
use std::path::Path;

/// Playtime and last-played data for a game, keyed by TITLE_ID (e.g. "NPUB30698").
#[derive(Debug, Clone, Default)]
pub struct PersistentData {
    /// TITLE_ID → playtime in milliseconds.
    pub playtime_ms: HashMap<String, u64>,
    /// TITLE_ID → last-played Unix timestamp in seconds.
    pub last_played: HashMap<String, i64>,
}

/// Parse persistent_settings.dat (INI format) into playtime and last-played maps.
///
/// Format:
///   [LastPlayed]
///   NPUB30698=2026-07-21T16:05:18
///
///   [Playtime]
///   NPUB30698=639249
///
/// Playtime values are in milliseconds (accumulated QElapsedTimer::restart() calls).
pub fn parse_persistent_settings(path: &Path) -> PersistentData {
    let mut data = PersistentData::default();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return data,
    };

    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim();
        match section.as_str() {
            "Playtime" => {
                if let Ok(ms) = value.parse::<u64>() {
                    data.playtime_ms.insert(key, ms);
                }
            }
            "LastPlayed" => {
                if let Some(ts) = parse_iso8601(value) {
                    data.last_played.insert(key, ts);
                }
            }
            _ => {}
        }
    }

    data
}

/// Parse an ISO 8601 timestamp like "2026-07-21T16:05:18" into a Unix timestamp (seconds).
/// Returns None on parse failure.
fn parse_iso8601(s: &str) -> Option<i64> {
    // Format: YYYY-MM-DDTHH:MM:SS (optionally with fractional seconds or timezone)
    let s = s.trim();
    let date_time = s.split('T').collect::<Vec<&str>>();
    if date_time.len() != 2 {
        return None;
    }
    let date_parts: Vec<&str> = date_time[0].split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year: i64 = date_parts[0].parse().ok()?;
    let month: u32 = date_parts[1].parse().ok()?;
    let day: u32 = date_parts[2].parse().ok()?;

    // Time part may have fractional seconds or timezone suffix; take HH:MM:SS only.
    let time_str = date_time[1].split(['.', '+', '-']).next().unwrap_or("");
    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 2 {
        return None;
    }
    let hour: i64 = time_parts[0].parse().ok()?;
    let minute: i64 = time_parts[1].parse().ok()?;
    let second: i64 = if time_parts.len() >= 3 {
        time_parts[2].parse().ok()?
    } else {
        0
    };

    let days = days_from_civil(year, month, day);
    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's days-from-civil algorithm — converts a proleptic Gregorian
/// date to days since 1970-01-01. Negative results represent dates before the epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // [0, 399]
    let m_adj = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * m_adj + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe as i64 - 719468
}

/// Convert milliseconds to hours (f64).
pub fn ms_to_hours(ms: u64) -> f64 {
    ms as f64 / 3_600_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ms_to_hours() {
        assert_eq!(ms_to_hours(0), 0.0);
        assert!((ms_to_hours(3_600_000) - 1.0).abs() < f64::EPSILON);
        assert!((ms_to_hours(639_249) - 0.1775691).abs() < 0.001);
    }

    #[test]
    fn test_parse_iso8601_basic() {
        let ts = parse_iso8601("2026-07-21T16:05:18").unwrap();
        // 2026-07-21 is a known date; verify the timestamp is reasonable.
        assert!(ts > 1_700_000_000); // after 2023
        assert!(ts < 2_000_000_000); // before 2033
    }

    #[test]
    fn test_parse_iso8601_epoch() {
        // 1970-01-01T00:00:00 should be 0.
        let ts = parse_iso8601("1970-01-01T00:00:00").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_parse_iso8601_with_fractional() {
        let ts1 = parse_iso8601("2026-07-21T16:05:18").unwrap();
        let ts2 = parse_iso8601("2026-07-21T16:05:18.123").unwrap();
        assert_eq!(ts1, ts2);
    }

    #[test]
    fn test_parse_iso8601_invalid() {
        assert!(parse_iso8601("garbage").is_none());
        assert!(parse_iso8601("2026-07-21").is_none());
        assert!(parse_iso8601("").is_none());
    }

    #[test]
    fn test_days_from_civil_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn test_days_from_civil_leap_year() {
        // 2024-03-01 is day 60 of 2024 (leap year: Feb has 29 days).
        // Days since epoch: 2024-01-01 is 19723 days after 1970-01-01.
        // 2024-03-01 = 19723 + 31 (Jan) + 29 (Feb) = 19783
        assert_eq!(days_from_civil(2024, 3, 1), 19783);
    }

    #[test]
    fn test_parse_persistent_settings_full() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "[LastPlayed]\nNPUB30698=2026-07-21T16:05:18\n\n[Playtime]\nNPUB30698=639249\nBLJS10000=3600000\n").unwrap();

        let data = parse_persistent_settings(tmp.path());
        assert_eq!(data.playtime_ms.get("NPUB30698"), Some(&639249));
        assert_eq!(data.playtime_ms.get("BLJS10000"), Some(&3_600_000));
        assert!(data.last_played.contains_key("NPUB30698"));
        assert!(!data.last_played.contains_key("BLJS10000"));
    }

    #[test]
    fn test_parse_persistent_settings_missing_file() {
        let data = parse_persistent_settings(std::path::Path::new("/nonexistent/path"));
        assert!(data.playtime_ms.is_empty());
        assert!(data.last_played.is_empty());
    }

    #[test]
    fn test_parse_persistent_settings_ignores_comments() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "; comment\n# another\n[Playtime]\nNPUB30698=100\n",
        )
        .unwrap();

        let data = parse_persistent_settings(tmp.path());
        assert_eq!(data.playtime_ms.get("NPUB30698"), Some(&100));
    }
}
