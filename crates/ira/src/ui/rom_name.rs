//! ROM names cleaned into ScreenScraper search terms. The source's word
//! search needs the punctuation ("Phoenix Wright: Ace Attorney Trilogy"
//! hits only with the colon) but drowns in dump tags and refuses terms
//! that are too short — the rules here are ES-DE's
//! (`references/emulationstation-de`, ScreenScraper.cpp + StringUtil.cpp).

/// Strip the dump tags a ROM name carries — every `(...)` and `[...]`
/// group, including the bracketed switch/3DS title ids — and the
/// underscores scene names use for spaces, so a title search sees a
/// title. Punctuation stays: the source's search needs it, and the
/// acceptance comparison is what ignores it. Console-fed titles bring
/// their own noise, which never survives into a search: the trademark
/// glyphs Nintendo's metadata loves ("Bayonetta™") and the
/// filesystem-safe colon switch titles use instead of a real one
/// ("Catherine꞉ Full Body").
pub(crate) fn clean_rom_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '\u{2122}' | '\u{00ae}' | '\u{00a9}' | '\u{2120}' => {}
            '\u{a789}' => out.push(':'),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let cleaned: String = out.replace('_', " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// ScreenScraper refuses word searches that stay under four characters
/// once its own "the " words are gone — "The Box" searches as "Box" and
/// the API only answers with an error. ES-DE routes these to the exact
/// romnom lookup; the batch pass instead skips the request: one doomed
/// call per opening is still a wasted request.
pub(crate) fn too_short_for_recherche(term: &str) -> bool {
    let mut stripped = term.to_uppercase().replace("THE ", "");
    if stripped.ends_with(" THE") {
        stripped.truncate(stripped.len() - 4);
    }
    stripped.trim().chars().count() < 4
}

/// Switch/3DS dumps are sometimes named nothing but the console's
/// 16-hex-digit title id — as a search term that is noise, so callers
/// fall back to the library title.
pub(crate) fn looks_like_title_id(term: &str) -> bool {
    term.len() == 16 && term.chars().all(|c| c.is_ascii_hexdigit())
}

/// The name up to its first subtitle separator — the part before a
/// space-adjacent `-`, `:`, `–` or `—`. ScreenScraper's word search is a
/// substring match over its own name strings, and those disagree with the
/// scene (and with each other) about the separator: the same subtitle is
/// "Ace Combat 04 : Shattered Skies" on PS2 and "Phoenix Wright: Ace
/// Attorney Trilogy" on Switch, so no full-title spelling matches both.
/// The main title alone is a substring of every variant, and the
/// acceptance comparison — which ignores punctuation — still tells the
/// right candidate from its sequels. A dash or colon inside a word
/// ("Pac-Man", "Link's Awakening DX: no") has no adjacent space and is
/// not a separator.
pub(crate) fn main_title(name: &str) -> String {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    for (pos, &(idx, c)) in chars.iter().enumerate() {
        let separator = matches!(c, '-' | ':' | '\u{2013}' | '\u{2014}');
        let spaced = pos > 0 && chars[pos - 1].1 == ' '
            || chars.get(pos + 1).is_some_and(|&(_, n)| n == ' ');
        if separator && spaced {
            return name[..idx].trim().to_string();
        }
    }
    name.trim().to_string()
}

/// The one term the word search gets: the main title, capped at two words
/// when the name carries no separator at all — ScreenScraper's own name
/// may still hold one ("Phoenix Wright: Ace Attorney Trilogy" against a
/// dump named "Phoenix Wright Ace Attorney Trilogy"), and two words stay
/// a substring as long as the separator sits past them. The acceptance
/// comparison still judges candidates against the full name.
pub(crate) fn search_head(name: &str) -> String {
    let main = main_title(name);
    if main != name {
        return main;
    }
    let head: Vec<&str> = main.split_whitespace().take(2).collect();
    head.join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        clean_rom_name, looks_like_title_id, main_title, search_head, too_short_for_recherche,
    };

    #[test]
    fn test_clean_rom_name_strips_dump_tags() {
        assert_eq!(
            clean_rom_name("Fire Emblem - Three Houses (USA) (En,Fr,De) [b]"),
            "Fire Emblem - Three Houses"
        );
        assert_eq!(clean_rom_name("Zelda_No_Densetsu [v1.0]"), "Zelda No Densetsu");
        // Unbalanced opening brackets still keep the text.
        assert_eq!(clean_rom_name("Half Life (Source"), "Half Life");
        assert_eq!(clean_rom_name("   "), "");
    }

    #[test]
    fn test_clean_rom_name_normalizes_console_title_noise() {
        // Switch metadata titles carry trademark glyphs and the
        // filesystem-safe colon.
        assert_eq!(clean_rom_name("Bayonetta\u{2122}"), "Bayonetta");
        assert_eq!(clean_rom_name("Catherine\u{a789} Full Body"), "Catherine: Full Body");
        // In-word dashes survive (Pac-Man must keep searching as Pac-Man).
        assert_eq!(clean_rom_name("Pac-Man Collection (USA)"), "Pac-Man Collection");
    }

    #[test]
    fn test_clean_rom_name_keeps_punctuation_the_source_needs() {
        assert_eq!(clean_rom_name("13 Sentinels: Aegis Rim"), "13 Sentinels: Aegis Rim");
        assert_eq!(
            clean_rom_name("Ace Attorney - Justice for All (USA)"),
            "Ace Attorney - Justice for All"
        );
        assert_eq!(clean_rom_name("Pok\u{e9}mon: Let's Go"), "Pok\u{e9}mon: Let's Go");
    }

    #[test]
    fn test_too_short_for_recherche_ignores_the_words() {
        assert!(too_short_for_recherche("The Box"));
        assert!(too_short_for_recherche("Box the"));
        assert!(too_short_for_recherche("GTA"));
        assert!(!too_short_for_recherche("Okki"));
        assert!(!too_short_for_recherche("The Matrix"));
    }

    #[test]
    fn test_looks_like_title_id_spots_bare_console_ids() {
        assert!(looks_like_title_id("0100A9400C9C2000"));
        // Serials, RA ids and titles never read as one.
        assert!(!looks_like_title_id("SLES-52005"));
        assert!(!looks_like_title_id("0100A9400C9C200"));
        assert!(!looks_like_title_id("PhoenixWrightAce"));
        assert!(!looks_like_title_id(""));
    }

    #[test]
    fn test_main_title_cuts_at_space_adjacent_separators() {
        assert_eq!(main_title("Ace Combat 04 - Shattered Skies"), "Ace Combat 04");
        assert_eq!(main_title("13 Sentinels: Aegis Rim"), "13 Sentinels");
        assert_eq!(main_title("Phoenix Wright : Ace Attorney Trilogy"), "Phoenix Wright");
        // In-word dashes and colons are not separators.
        assert_eq!(main_title("Pac-Man World"), "Pac-Man World");
        assert_eq!(main_title("Zelda No Densetsu"), "Zelda No Densetsu");
        // A separator glued to the end still cuts.
        assert_eq!(main_title("Katamari Damacy -"), "Katamari Damacy");
    }

    #[test]
    fn test_search_head_caps_separatorless_names_at_two_words() {
        // A dump named without any separator cannot match a name that has
        // one inside; two words survive a separator past them.
        assert_eq!(
            search_head("Phoenix Wright Ace Attorney Trilogy"),
            "Phoenix Wright"
        );
        // Two-word and one-word names pass through.
        assert_eq!(search_head("Katamari Damacy"), "Katamari Damacy");
        assert_eq!(search_head("Okami"), "Okami");
        // Names with a separator keep their whole main title.
        assert_eq!(search_head("Ace Combat 04 - Shattered Skies"), "Ace Combat 04");
    }
}
