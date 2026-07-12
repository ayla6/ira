use chrono::NaiveDate;

pub fn parse_steam_release_date(date: &str) -> i64 {
    let date = date.trim();
    if date.is_empty() {
        return 0;
    }

    if let Ok(d) = NaiveDate::parse_from_str(date, "%d %b, %Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    }

    if let Ok(d) = NaiveDate::parse_from_str(date, "%b %d, %Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    }

    if let Ok(d) = NaiveDate::parse_from_str(date, "%b %Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    }

    if let Ok(d) = NaiveDate::parse_from_str(date, "%Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    }

    if let Some(year) = extract_year(date) {
        return year;
    }

    0
}

fn extract_year(s: &str) -> Option<i64> {
    for part in s.split_whitespace() {
        if let Ok(y) = part.parse::<i32>() {
            if y >= 1900 && y <= 2100 {
                return NaiveDate::from_ymd_opt(y, 1, 1)
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_date() {
        let ts = parse_steam_release_date("15 Sep, 2014");
        assert!(ts > 0, "expected non-zero timestamp, got {}", ts);
    }

    #[test]
    fn test_parse_year_only() {
        let ts = parse_steam_release_date("2014");
        let jan_1_2014 = NaiveDate::from_ymd_opt(2014, 1, 1).unwrap()
            .and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        assert_eq!(ts, jan_1_2014);
    }

    #[test]
    fn test_parse_extract_year() {
        let ts = parse_steam_release_date("Q1 2014");
        let jan_1_2014 = NaiveDate::from_ymd_opt(2014, 1, 1).unwrap()
            .and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        assert_eq!(ts, jan_1_2014);
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(parse_steam_release_date("Coming Soon"), 0);
    }

    #[test]
    fn test_parse_empty() {
        assert_eq!(parse_steam_release_date(""), 0);
    }

    #[test]
    fn test_parse_alt_format() {
        let ts = parse_steam_release_date("Sep 15, 2014");
        assert!(ts > 0, "expected non-zero timestamp, got {}", ts);
    }
}
