//! The age-rating boards the sources emit — Steam's `ratings` keys and
//! ScreenScraper's classification kinds, which together cover the boards
//! actually in use on game store data — with their known value sets.
//! The closed sets drive the rating picker, and the board id + value
//! pair names the bundled mark in `ira-images`.

/// One board: its canonical stored kind, display bits, known values in
/// ascending age order, and the other spellings the sources emit for
/// it. Boards the sources emit without a fixed public value set stay
/// out of this table entirely — unknown kinds simply keep their raw
/// text and get no mark.
pub struct RatingBoard {
    /// The canonical kind id stored in the database.
    pub id: &'static str,
    /// The board's display name.
    pub name: &'static str,
    /// The region the board rates for.
    pub region: &'static str,
    /// The board's known values, youngest first.
    pub values: &'static [&'static str],
    /// Other kind spellings stored for this board (Steam key variants,
    /// predecessor boards).
    pub aliases: &'static [&'static str],
}

pub const BOARDS: &[RatingBoard] = &[
    RatingBoard {
        id: "PEGI",
        name: "PEGI",
        region: "Europe",
        values: &["3", "7", "12", "16", "18"],
        aliases: &[],
    },
    RatingBoard {
        id: "ESRB",
        name: "ESRB",
        region: "North America",
        values: &["E", "E10+", "T", "M", "AO", "RP"],
        aliases: &[],
    },
    RatingBoard {
        id: "USK",
        name: "USK",
        region: "Germany",
        values: &["0", "6", "12", "16", "18"],
        aliases: &["STEAM_GERMANY"],
    },
    RatingBoard {
        id: "CERO",
        name: "CERO",
        region: "Japan",
        values: &["A", "B", "C", "D", "Z"],
        aliases: &[],
    },
    RatingBoard {
        id: "GRB",
        name: "GRB",
        region: "South Korea",
        values: &["ALL", "12", "15", "19"],
        aliases: &["KGRB"],
    },
    RatingBoard {
        id: "CLASSIND",
        name: "ClassInd",
        region: "Brazil",
        values: &["L", "6", "10", "12", "14", "16", "18"],
        aliases: &["CLASS_IND", "DEJUS"],
    },
    // The self-classification marks digital Brazilian games carry —
    // the same ages with an A prefix (AL = livre).
    RatingBoard {
        id: "CLASSIND_A",
        name: "ClassInd Autoclassificada",
        region: "Brazil (self-rated)",
        values: &["AL", "A6", "A10", "A12", "A14", "A16", "A18"],
        aliases: &[],
    },
    RatingBoard {
        id: "ACB",
        name: "ACB",
        region: "Australia",
        values: &["G", "PG", "M", "MA 15+", "R 18+"],
        aliases: &["OFLC", "STEAM_AUSTRALIA"],
    },
    RatingBoard {
        id: "NZOFLC",
        name: "OFLC",
        region: "New Zealand",
        values: &["G", "PG", "M", "R13", "R15", "R16", "R18"],
        aliases: &["NZ"],
    },
    RatingBoard {
        id: "CSRR",
        name: "CSRR",
        region: "Taiwan",
        values: &["0+", "6+", "12+", "15+", "18+"],
        aliases: &["GSRR"],
    },
    RatingBoard {
        id: "IGRS",
        name: "IGRS",
        region: "Indonesia",
        values: &["SU", "3+", "7+", "13+", "15+", "18+", "RC"],
        aliases: &[],
    },
    RatingBoard {
        id: "BBFC",
        name: "BBFC",
        region: "United Kingdom (retired)",
        values: &["U", "PG", "12", "15", "18"],
        aliases: &[],
    },
    RatingBoard {
        id: "ELSPA",
        name: "ELSPA",
        region: "United Kingdom (retired)",
        values: &["3+", "11+", "15+", "18+"],
        aliases: &[],
    },
    RatingBoard {
        id: "SELL",
        name: "SELL",
        region: "France (retired)",
        values: &["3+", "7+", "12+", "16+", "18+"],
        // ScreenScraper stores the French marks as "JV" (Jeux Vidéo —
        // what the old SELL ratings printed on the box).
        aliases: &["JV"],
    },
];

/// The board a stored kind belongs to: case, underscores and spaces
/// fold away, so Steam's `class_ind` meets the stored `CLASSIND` and
/// `steam_germany` meets `USK`. `None` — the kind stays raw text — for
/// boards with no fixed value set (CRL, MDA, …).
pub fn find_board(kind: &str) -> Option<&'static RatingBoard> {
    let normalized = normalize(kind);
    BOARDS.iter().find(|board| {
        normalize(board.id) == normalized
            || board.aliases.iter().any(|alias| normalize(alias) == normalized)
    })
}

fn normalize(kind: &str) -> String {
    kind.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// The asset slug naming the board+value's bundled mark: board id and
/// the canonical value spelling, lowercased with punctuation dropped
/// ("CLASSIND" + "MA 15+" becomes `classind-ma15`). Values the sources
/// spell oddly ("+7 ans", "R 13") still match by their digits. `None`
/// when the board has no known values or the value is not one of them —
/// those get text, not a mark.
pub fn rating_slug(kind: &str, value: &str) -> Option<String> {
    let board = find_board(kind)?;
    let canonical = canonical_value(board, value)?;
    Some(format!(
        "{}-{}",
        normalize(board.id),
        canonical
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    ))
}

/// The display text for a stored pair: the board's name with its
/// canonical value spelling — "STEAM_GERMANY" + "12" reads as
/// "USK 12", "CERO" + "a" as "CERO A". `None` for unknown boards;
/// callers keep the raw text.
pub fn display(kind: &str, value: &str) -> Option<String> {
    let board = find_board(kind)?;
    let canonical = canonical_value(board, value)?;
    Some(format!("{} {}", board.name, canonical))
}

fn canonical_value(board: &RatingBoard, value: &str) -> Option<&'static str> {
    board
        .values
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(value))
        .copied()
        .or_else(|| fuzzy_by_digits(board, value))
}

/// The value whose number matches ("+7 ans" and "7+" are both 7).
/// Lettered values (M, AL, RP…) never match this way.
fn fuzzy_by_digits(board: &RatingBoard, value: &str) -> Option<&'static str> {
    let digits = digits_of(value);
    if digits.is_empty() {
        return None;
    }
    board
        .values
        .iter()
        .find(|candidate| digits_of(candidate) == digits)
        .copied()
}

fn digits_of(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_digit()).collect()
}

#[cfg(test)]
mod tests {
    use super::{find_board, rating_slug, BOARDS};

    #[test]
    fn test_find_board_folds_spelling_across_sources() {
        // Steam's snake_case keys and SS's spellings meet one board.
        assert_eq!(find_board("class_ind").map(|b| b.id), Some("CLASSIND"));
        assert_eq!(find_board("DEJUS").map(|b| b.id), Some("CLASSIND"));
        assert_eq!(find_board("KGRB").map(|b| b.id), Some("GRB"));
        assert_eq!(find_board("steam_germany").map(|b| b.id), Some("USK"));
        assert_eq!(find_board("STEAM_AUSTRALIA").map(|b| b.id), Some("ACB"));
        // Old Australia's board is its successor's alias; New Zealand's
        // is its own board.
        assert_eq!(find_board("OFLC").map(|b| b.id), Some("ACB"));
        assert_eq!(find_board("NZOFLC").map(|b| b.id), Some("NZOFLC"));
        // Plain ids find themselves, unknown kinds find nothing.
        assert_eq!(find_board("pegi").map(|b| b.id), Some("PEGI"));
        assert!(find_board("CRL").is_none());
        assert!(find_board("").is_none());
    }

    #[test]
    fn test_rating_slug_canonicalizes_board_and_value() {
        assert_eq!(rating_slug("PEGI", "18"), Some("pegi-18".to_string()));
        assert_eq!(rating_slug("ESRB", "e10+"), Some("esrb-e10".to_string()));
        // The alias resolves to the board, but the value must be one
        // that board actually issues.
        assert_eq!(rating_slug("CLASS_IND", "MA 15+"), None);
        assert_eq!(rating_slug("ACB", "MA 15+"), Some("acb-ma15".to_string()));
        assert_eq!(rating_slug("KGRB", "all"), Some("grb-all".to_string()));
        // A value the board never issues gets no mark.
        assert_eq!(rating_slug("PEGI", "14"), None);
        // Boards without fixed values get none either.
        assert_eq!(rating_slug("CRL", "whatever"), None);
        // ScreenScraper's French kind is the SELL board, and its "+7
        // ans" spelling finds the 7+ mark by digits.
        assert_eq!(find_board("JV").map(|b| b.id), Some("SELL"));
        assert_eq!(rating_slug("JV", "+7 ans"), Some("sell-7".to_string()));
        // Oddly spelled numbers elsewhere resolve the same way.
        assert_eq!(rating_slug("NZOFLC", "R 13"), Some("nzoflc-r13".to_string()));
        // Display names canonicalize the board and the value casing.
        assert_eq!(super::display("STEAM_GERMANY", "12"), Some("USK 12".to_string()));
        assert_eq!(super::display("CERO", "a"), Some("CERO A".to_string()));
        assert_eq!(super::display("DEJUS", "12"), Some("ClassInd 12".to_string()));
        assert_eq!(super::display("NOTABOARD", "12"), None);
        // A lettered value never matches through digits.
        assert_eq!(rating_slug("ESRB", "E10+"), Some("esrb-e10".to_string()));
        assert_eq!(rating_slug("CERO", "12"), None);
    }

    #[test]
    fn test_boards_have_unique_ids_and_sluggable_values() {
        for (index, board) in BOARDS.iter().enumerate() {
            assert!(!BOARDS[..index].iter().any(|b| b.id == board.id));
            for value in board.values {
                assert!(
                    rating_slug(board.id, value).is_some(),
                    "{} {} has no slug",
                    board.id,
                    value
                );
            }
        }
    }
}
