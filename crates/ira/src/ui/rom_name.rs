//! ROM names cleaned into ScreenScraper search terms. The source's word
//! search needs the punctuation ("Phoenix Wright: Ace Attorney Trilogy"
//! hits only with the colon) but drowns in dump tags and refuses terms
//! that are too short — the rules here are ES-DE's
//! (`references/emulationstation-de`, ScreenScraper.cpp + StringUtil.cpp).

/// Strip the dump tags a ROM name carries — every `(...)` and `[...]`
/// group, including the bracketed switch/3DS title ids — and the
/// underscores scene names use for spaces, so a title search sees a
/// title. Punctuation stays: the source's search needs it, and the
/// acceptance comparison is what ignores it.
pub(crate) fn clean_rom_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
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

#[cfg(test)]
mod tests {
    use super::{clean_rom_name, looks_like_title_id, too_short_for_recherche};

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
}
