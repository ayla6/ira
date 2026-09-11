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
    front_load_article(result)
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

/// A trailing English article moves to the front, so a file named
/// "World Ends With You, The" and a store title "The World Ends With You"
/// normalize to the same key.
fn front_load_article(norm: String) -> String {
    for article in [" the", " an", " a"] {
        if let Some(base) = norm.strip_suffix(article) {
            if !base.is_empty() {
                return format!("{} {}", &article[1..], base);
            }
        }
    }
    norm
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
