//! Game-title normalization for matching: one canonical key so a ROM file
//! name, a RetroAchievements title and a SteamGridDB store title compare
//! equal despite punctuation, articles, ampersands and accents.

/// Lowercases, strips bracketed tag groups (`(USA)`, `[!]`) and punctuation,
/// folds accented Latin letters onto their base letter, turns "&" into
/// "and", and moves a trailing English article to the front — so
/// "Castlevania - Dawn of Sorrow", "Castlevania: Dawn of Sorrow",
/// "Pokémon" and "Pokemon", "World Ends With You, The" and "The World Ends
/// With You" land on the same key.
pub fn normalize_name(s: &str) -> String {
    let s = front_load_article(s);
    // "&" and "and" must land on the same key: titles use either.
    let s = s.replace('&', " and ");
    let mut result = String::with_capacity(s.len());
    let mut in_brackets = 0i32;
    let mut prev_space = true;

    for c in s.chars() {
        match c {
            '(' | '[' => in_brackets += 1,
            ')' | ']' => {
                if in_brackets > 0 {
                    in_brackets -= 1;
                }
            }
            _ if in_brackets > 0 => {}
            _ => {
                let c = if c == '_' || c == '.' { ' ' } else { c };
                for lc in c.to_lowercase() {
                    let lc = fold_accent(lc);
                    if lc.is_alphanumeric() {
                        result.push(lc);
                        prev_space = false;
                    } else if !prev_space {
                        result.push(' ');
                        prev_space = true;
                    }
                }
            }
        }
    }

    while result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Accented Latin letters fold onto their base letter so "Pokémon" and
/// "Pokemon" normalize the same; other scripts are left alone.
fn fold_accent(c: char) -> char {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

/// Moves a file-name-style article back to the front: No-Intro names write
/// "World Ends With You, The" and "Legend of Zelda, The - Phantom
/// Hourglass", where store titles say "The World Ends With You" and "The
/// Legend of Zelda: Phantom Hourglass". The ", The" must sit at the end of
/// the main title — before a subtitle, tag group or the end of the name —
/// so a comma inside prose ("Me, the Robot") is left alone.
fn front_load_article(s: &str) -> String {
    let bytes = s.as_bytes();
    for article in ["the", "an", "a"] {
        let mut from = 0;
        while let Some(rel) = bytes[from..].iter().position(|b| *b == b',') {
            let start = from + rel;
            let end = start + 2 + article.len();
            if end <= bytes.len()
                && bytes[start + 1] == b' '
                && bytes[start + 2..end].eq_ignore_ascii_case(article.as_bytes())
                && ends_main_title(&bytes[end..])
            {
                return format!("{} {}{}", &s[start + 2..end], &s[..start], &s[end..]);
            }
            from = start + 1;
        }
    }
    s.to_string()
}

/// Whether the bytes after a ", The" are a subtitle separator, a tag
/// group, or nothing — the places a file-name article can legally end.
fn ends_main_title(rest: &[u8]) -> bool {
    rest.is_empty()
        || rest.starts_with(b" -")
        || rest.starts_with(b":")
        || rest.starts_with(b" (")
        || rest.starts_with(b" [")
}

#[cfg(test)]
mod tests {
    use super::normalize_name;

    #[test]
    fn test_normalize_name_basic() {
        assert_eq!(normalize_name("Final Fantasy VII (USA)"), "final fantasy vii");
    }

    #[test]
    fn test_normalize_name_underscores() {
        assert_eq!(normalize_name("Final_Fantasy.VII"), "final fantasy vii");
    }

    #[test]
    fn test_normalize_name_version_tags() {
        assert_eq!(normalize_name("Chrono Trigger [!]"), "chrono trigger");
    }

    #[test]
    fn test_normalize_name_empty() {
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn test_normalize_name_trailing_article_moves_to_front() {
        assert_eq!(
            normalize_name("World Ends With You, The"),
            "the world ends with you"
        );
        assert_eq!(
            normalize_name("The World Ends With You"),
            "the world ends with you"
        );
        assert_eq!(normalize_name("Hoshi no Kirby, The"), "the hoshi no kirby");
        assert_eq!(normalize_name("Odyssey, An"), "an odyssey");
        assert_eq!(normalize_name("Boy and His Blob, A"), "a boy and his blob");
    }

    #[test]
    fn test_normalize_name_article_before_subtitle_or_tags() {
        // No-Intro puts the article after the main title, before the subtitle.
        assert_eq!(
            normalize_name("Legend of Zelda, The - Phantom Hourglass (USA)"),
            normalize_name("The Legend of Zelda: Phantom Hourglass")
        );
        assert_eq!(
            normalize_name("Wizard of Oz, The - Beyond the Yellow Brick Road"),
            "the wizard of oz beyond the yellow brick road"
        );
        assert_eq!(
            normalize_name("World Ends with You, The (USA)"),
            "the world ends with you"
        );
        // A comma inside prose is not an article marker.
        assert_eq!(normalize_name("Me, the Robot"), "me the robot");
        assert_eq!(normalize_name("Game, Anthology Edition"), "game anthology edition");
    }

    #[test]
    fn test_normalize_name_ampersand_becomes_and() {
        assert_eq!(
            normalize_name("Mario & Luigi: Superstar Saga"),
            "mario and luigi superstar saga"
        );
        assert_eq!(
            normalize_name("Mario and Luigi: Superstar Saga"),
            "mario and luigi superstar saga"
        );
    }

    #[test]
    fn test_normalize_name_folds_accents() {
        assert_eq!(normalize_name("Pokémon Emerald"), "pokemon emerald");
        assert_eq!(
            normalize_name("Pocket Monsters: Élite"),
            "pocket monsters elite"
        );
    }

    #[test]
    fn test_normalize_name_dash_and_colon_agree() {
        assert_eq!(
            normalize_name("Castlevania - Dawn of Sorrow"),
            normalize_name("Castlevania: Dawn of Sorrow")
        );
    }
}
