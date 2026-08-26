//! Human-readable naming for 3DS dumps: cleaned filenames and stable
//! keys for homebrew without title IDs.

/// 3DSX homebrew carries no title ID; derive a stable key from the cleaned
/// file name so DB entries survive re-scans.
pub(super) fn three_dsx_key(title: &str) -> String {
    let mut key = String::from("3dsx-");
    let mut last_dash = true;
    for c in title.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            key.push(c);
            last_dash = false;
        } else if !last_dash {
            key.push('-');
            last_dash = true;
        }
    }
    while key.ends_with('-') {
        key.pop();
    }
    key
}

fn is_dump_tag(tag: &str) -> bool {
    let tag = tag.trim();
    if tag.is_empty() {
        return true;
    }
    let upper = tag.to_ascii_uppercase();
    if upper.starts_with("CTR-P-") || upper.starts_with("REV ") || upper.starts_with("REV-") {
        return true;
    }
    if let Some(rest) = upper.strip_prefix('V') {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return true;
        }
    }
    matches!(
        upper.as_str(),
        "U" | "USA"
            | "US"
            | "E"
            | "EU"
            | "EUR"
            | "EUROPE"
            | "J"
            | "JP"
            | "JPN"
            | "JAPAN"
            | "K"
            | "KR"
            | "KOR"
            | "A"
            | "AU"
            | "AUS"
            | "AUSTRALIA"
            | "W"
            | "WORLD"
            | "CN"
            | "CHN"
            | "CHINA"
            | "TW"
            | "TWN"
            | "HK"
            | "NL"
            | "FR"
            | "DE"
            | "IT"
            | "ES"
            | "PT"
            | "RU"
            | "MX"
            | "CA"
            | "NORDIC"
            | "SCANDINAVIA"
            | "DECRYPTED"
            | "ENCRYPTED"
            | "DIGITAL"
            | "RETAIL"
            | "DEMO"
            | "ONLINE"
    )
}

/// Derives a display title from a dump filename. Dump tools usually name
/// files like `00040000000E5C00 Shin Megami Tensei IV (CTR-P-AMXE) (v0.1.0)
/// (U).cci`; strip the leading title ID and release tags while keeping any
/// bracketed text that is part of the game's own name.
pub(super) fn title_from_filename(stem: &str) -> String {
    let stem = stem.trim();
    let stem = match stem.get(..16) {
        Some(prefix)
            if prefix.chars().all(|c| c.is_ascii_hexdigit())
                && stem.as_bytes().get(16) == Some(&b' ') =>
        {
            &stem[17..]
        }
        _ => stem,
    };

    let mut name = String::new();
    let mut tag = String::new();
    let mut depth = 0i32;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth <= 0 {
                    depth = 0;
                    if is_dump_tag(&tag) {
                        tag.clear();
                    } else {
                        let bracket = if c == ')' { '(' } else { '[' };
                        let close = c;
                        tag.insert(0, bracket);
                        tag.push(close);
                        if !name.is_empty() {
                            name.push(' ');
                        }
                        name.push_str(&tag);
                        tag.clear();
                    }
                }
            }
            _ if depth > 0 => tag.push(c),
            _ => name.push(if c == '_' { ' ' } else { c }),
        }
    }
    let cleaned = collapse_spaces(&name);
    if cleaned.is_empty() {
        collapse_spaces(&stem.replace('_', " "))
    } else {
        cleaned
    }
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_from_filename_strips_dump_tags() {
        assert_eq!(
            title_from_filename("00040000000E5C00 Shin Megami Tensei IV (CTR-P-AMXE) (v0.1.0) (U)"),
            "Shin Megami Tensei IV"
        );
        assert_eq!(
            title_from_filename("Professor_Layton [USA] (v2)"),
            "Professor Layton"
        );
        assert_eq!(title_from_filename("Cubic Ninja (EUR)"), "Cubic Ninja");
        // Bracketed words that are part of the name survive.
        assert_eq!(
            title_from_filename("Game & Wario (Something Real) (J)"),
            "Game & Wario (Something Real)"
        );
        // Nothing but tags: fall back to the raw stem instead of an empty name.
        assert_eq!(title_from_filename("(USA)"), "(USA)");
    }

    #[test]
    fn test_three_dsx_key_sanitizes_cleaned_title() {
        assert_eq!(three_dsx_key("Game Homebrew"), "3dsx-game-homebrew");
        assert_eq!(three_dsx_key("Fruit Punch"), "3dsx-fruit-punch");
    }
}
