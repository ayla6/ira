//! ROM names cleaned into ScreenScraper search terms. The source's word
//! search needs the punctuation ("Phoenix Wright: Ace Attorney Trilogy"
//! hits only with the colon) but drowns in dump tags and refuses terms
//! that are too short — the rules here are ES-DE's
//! (`references/emulationstation-de`, ScreenScraper.cpp + StringUtil.cpp).

/// Strip the dump tags a ROM name carries — every `(...)` and `[...]`
/// group, including the bracketed switch/3DS title ids — and the
/// underscores and dots, so a title search sees a title. Dots go
/// because ScreenScraper's search stumbles on them: their database
/// writes "Plants vs Zombies" without one, and the dot in "Plants vs.
/// Zombies" kills the hit even though the game is theirs. Other
/// punctuation stays: the source's search needs it, and the acceptance
/// comparison is what ignores it. Console-fed titles bring their own
/// noise, which never survives into a search: the trademark glyphs
/// Nintendo's metadata loves ("Bayonetta™") and the filesystem-safe
/// colon switch titles use instead of a real one ("Catherine꞉ Full
/// Body").
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
    let cleaned: String = out.replace(['_', '.'], " ");
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    // A token that is nothing but a console id ("0004000000054000", or
    // the same with a region letter glued on) rides in front of hShop
    // and dump names; it names nothing.
    words.retain(|word| !(word.len() >= 10 && word.chars().filter(|c| c.is_ascii_digit()).count() >= 8));
    // hShop file endings survive file_stem as whole words when the
    // stack is deeper than one extension ("... (U).legit.decrypted").
    const ENDINGS: &[&str] = &["legit", "pirate", "piratelegit", "decrypted", "3ds", "cci"];
    while words.len() > 1
        && words
            .last()
            .is_some_and(|last| ENDINGS.contains(&last.to_lowercase().as_str()))
    {
        words.pop();
    }
    words.join(" ")
}

/// The widening retry when the search term answers nothing: the name's
/// other side. "Bowser's Fury" arriving empty falls back to "Super Mario
/// 3D World"; single-segment names have no other side.
pub(crate) fn alt_search_term(name: &str) -> Option<String> {
    let segments = separator_segments(name);
    if segments.len() < 2 {
        return None;
    }
    Some(segments[0].trim().to_string())
}

/// Switch/3DS dumps are sometimes named nothing but the console's
/// 16-hex-digit title id — as a search term that is noise, so callers
/// fall back to the library title.
pub(crate) fn looks_like_title_id(term: &str) -> bool {
    term.len() == 16 && term.chars().all(|c| c.is_ascii_hexdigit())
}

/// The name split at space-adjacent separators — the segments of
/// "Ace Combat 04 - Shattered Skies" are "Ace Combat 04" and "Shattered
/// Skies"; crossover "+" counts too ("Super Mario 3D World + Bowser's
/// Fury" would bury its tail under every other Mario game otherwise);
/// a trailing separator contributes nothing and leftover separator
/// characters at a segment's edges come off with it.
fn separator_segments(name: &str) -> Vec<&str> {
    let bytes = name.as_bytes();
    let mut segments: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = name[i..].chars().next().unwrap_or(' ');
        let len = c.len_utf8();
        let separator = matches!(c, '-' | ':' | '+' | '\u{2013}' | '\u{2014}');
        let spaced = (i > 0 && bytes[i - 1] == b' ') || bytes.get(i + len) == Some(&b' ');
        if separator && spaced {
            segments.push(&name[start..i]);
            i += len;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            start = i;
            continue;
        }
        i += len;
    }
    let tail = &name[start..];
    if !tail.trim().is_empty() {
        segments.push(tail);
    }
    segments
        .iter()
        .map(|s| {
            s.trim_matches(|c| {
                matches!(c, '-' | ':' | '+' | '\u{2013}' | '\u{2014}' | ' ')
            })
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// The separators a raw dump name carries outside its `(...)`/`[...]`
/// tags: the search-relevant punctuation plus the filesystem-safe
/// stand-ins `_` and `.` that cleaning turns back into word breaks.
/// This is the file-vs-title evidence — ScreenScraper's names keep
/// their punctuation and match byte-sensitively, and a dump whose
/// title-colon became `Echoes_` still carries the separator, while a
/// file renamed with the punctuation deleted ("NieRAutomata") carries
/// none and can only search as a glued token that exists in no
/// ScreenScraper name.
fn raw_separator_richness(name: &str) -> usize {
    let mut depth = 0usize;
    let mut separators = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '-' | ':' | '.' | '_' | '\'' | '\u{2013}' | '\u{2014}' | '\u{a789}'
                if depth == 0 =>
            {
                separators += 1
            }
            _ => {}
        }
    }
    separators
}

/// The name the search builds from. When the title comes from the game's
/// own data — trusted rows, and consoles whose internal titles are
/// authoritative (switch's NACP above all) — the title *is* the base:
/// the game's own name beats any file naming, and the cleaner has
/// already normalized the ™ and pseudo-colon glyphs it carries. Every
/// other console searches from the ROM file name (3DS-era titles drift
/// from the dumps), and the dump keeps the lead unless its raw name
/// carries fewer separators than the title — punctuation deleted, not
/// swapped for `_`/`.`, the one case where the file searches as a glued
/// token the source's names cannot match. Separator evidence is judged
/// on the raw names so a file's stand-in separators count as the word
/// breaks they clean into; the console menu's abbreviations (3DS SMDH
/// short titles fit 32 characters: "FE Echoes…") never outrank a healthy
/// dump. Nameless and bare-title-id candidates never qualify, and the
/// cleaner normalizes whatever wins.
pub(crate) fn pick_search_name(
    trusted_title: bool,
    raw_stem: &str,
    raw_title: &str,
    raw_display: &str,
) -> Option<String> {
    let raws = [raw_stem, raw_title, raw_display];
    let cleaned = [
        clean_rom_name(raw_stem),
        clean_rom_name(raw_title),
        clean_rom_name(raw_display),
    ];
    // [stem, title, display], with the title first when it is
    // authoritative.
    let order: [usize; 3] = if trusted_title { [1, 0, 2] } else { [0, 1, 2] };
    let usable = |i: usize| !cleaned[i].is_empty() && !looks_like_title_id(&cleaned[i]);
    if trusted_title {
        return order.into_iter().find(|&i| usable(i)).map(|i| cleaned[i].clone());
    }
    order
        .into_iter()
        .filter(|&i| usable(i))
        .fold(None::<usize>, |best, i| match best {
            Some(b) if raw_separator_richness(raws[b]) >= raw_separator_richness(raws[i]) => {
                Some(b)
            }
            _ => Some(i),
        })
        .map(|i| cleaned[i].clone())
}

/// The one term the word search gets. Live probes against jeuRecherche:
/// searching a series head buries the game among its siblings or drops
/// it entirely ("Simple 2000 Series Vol. 50" answers 28 other volumes
/// and not the one whose name starts with exactly that), while the
/// distinctive tail hit the target in every probed case — "Shattered
/// Skies", "The Daibijin", "Ace Attorney Trilogy", "Smiling Man", "Full
/// Body", "San Andreas" all answer one or two games, ours included.
/// So: the last separator segment, the part dump names and ScreenScraper
/// names agree is the game's own. Names with no separator keep a
/// two-word head — ScreenScraper's name may still hold one ("Phoenix
/// Wright: Ace Attorney Trilogy" against a dump named "Phoenix Wright
/// Ace Attorney Trilogy"), and two words stay a prefix as long as the
/// separator sits past them. Short terms go out as-is: "Rez" and "Z-A"
/// answer fine, the source errors on none of them. The acceptance
/// comparison still judges candidates against the full name.
pub(crate) fn search_term(name: &str) -> String {
    let segments = separator_segments(name);
    if segments.len() > 1 {
        return segments[segments.len() - 1].trim().to_string();
    }
    let only = name.trim();
    // Words with no letters or digits — a bare "&" — are not words;
    // searching "Fear &" finds nothing "Fear Hunger" wouldn't.
    let head: Vec<&str> = only
        .split_whitespace()
        .filter(|word| word.chars().any(|c| c.is_alphanumeric()))
        .take(2)
        .collect();
    head.join(" ")
}

/// Dump-tag words mapped to ScreenScraper's region codes. Matching never
/// requires them — plenty of dumps, especially on older platforms, carry
/// no region at all — they only break ties: between equally-shaped
/// candidates, the one whose matching name comes from the region the
/// dump says it is wins.
const REGION_WORDS: &[(&str, &str)] = &[
    ("japan", "jp"),
    ("usa", "us"),
    ("us", "us"),
    ("united states", "us"),
    ("europe", "eu"),
    ("eu", "eu"),
    ("world", "wor"),
    ("france", "fr"),
    ("germany", "de"),
    ("spain", "es"),
    ("italy", "it"),
    ("korea", "kr"),
    ("china", "cn"),
    ("taiwan", "tw"),
    ("hong kong", "hk"),
    ("australia", "au"),
    ("canada", "ca"),
    ("brazil", "br"),
    ("uk", "uk"),
    ("england", "uk"),
];

/// The region codes a ROM's `(...)`/`[...]` tags carry, in ScreenScraper's
/// vocabulary. Language lists ("En,Fr,De") and revision tags match
/// nothing and drop out.
pub(crate) fn region_hints(name: &str) -> Vec<&'static str> {
    let mut hints: Vec<&'static str> = Vec::new();
    for tag in bracket_groups(name) {
        let tag = tag.trim().to_lowercase();
        if let Some((_, code)) = REGION_WORDS.iter().find(|(word, _)| *word == tag) {
            if !hints.contains(code) {
                hints.push(code);
            }
        }
    }
    hints
}

/// The text inside every balanced `(...)`/`[...]` group.
fn bracket_groups(name: &str) -> Vec<String> {
    let mut groups = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => current.push(c),
            _ => {}
        }
        if depth == 0 && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::{
        alt_search_term, clean_rom_name, looks_like_title_id, pick_search_name,
        raw_separator_richness, region_hints, search_term,
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
    fn test_clean_rom_name_dots_are_spaces() {
        // ScreenScraper's database drops dots ("Plants vs Zombies"),
        // and its search stumbles on the title's own dot.
        assert_eq!(clean_rom_name("Plants vs. Zombies"), "Plants vs Zombies");
        assert_eq!(clean_rom_name("Super Mario Galaxy"), "Super Mario Galaxy");
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
    fn test_clean_rom_name_strips_hshop_naming() {
        // The hShop scheme: leading title id, serial and version tags,
        // and a stack of pseudo-extensions.
        assert_eq!(
            clean_rom_name(
                "0004000000054000 Super Mario 3D Land (CTR-P-AREE) (v0.3.0) (U).legit.decrypted"
            ),
            "Super Mario 3D Land"
        );
        // A short numeric head stays ("1943: The Battle of Midway").
        assert_eq!(clean_rom_name("1943 The Battle of Midway"), "1943 The Battle of Midway");
        // The last word is never stripped into an empty name.
        assert_eq!(clean_rom_name("Decrypted"), "Decrypted");
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
    fn test_alt_search_term_gives_the_other_side() {
        assert_eq!(
            alt_search_term("Super Mario 3D World + Bowser's Fury").as_deref(),
            Some("Super Mario 3D World")
        );
        assert_eq!(
            alt_search_term("The Hundred Line -Last Defense Academy-").as_deref(),
            Some("The Hundred Line")
        );
        // Single-segment names have no other side.
        assert_eq!(alt_search_term("Okami"), None);
        assert_eq!(alt_search_term(""), None);
    }

    #[test]
    fn test_search_term_sends_short_titles_as_they_are() {
        // Live probes: the source answers "Rez" and "Z-A" with hits and
        // no errors — the old four-character refusal only cost matches.
        assert_eq!(search_term("Rez"), "Rez");
        assert_eq!(search_term("Pok\u{e9}mon Legends: Z-A"), "Z-A");
    }

    #[test]
    fn test_search_term_handles_crossover_plus_and_edge_separators() {
        // The "+" of a crossover title is a separator: the two-word head
        // "Super Mario" buries the bundle under every other Mario game
        // (live answer: 16 hits, ours absent), while the tail answers
        // alone.
        assert_eq!(
            search_term("Super Mario 3D World + Bowser's Fury"),
            "Bowser's Fury"
        );
        // Separator characters hugging a segment come off with it.
        assert_eq!(
            search_term("The Hundred Line -Last Defense Academy-"),
            "Last Defense Academy"
        );
        // In-word plus signs and dashes are not separators.
        assert_eq!(search_term("C++"), "C++");
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
    fn test_pick_search_name_trusted_titles_lead_and_richness_rescues_the_rest() {
        // A trusted title IS the base, even against a punctuation-richer
        // file name: the game's own name outranks file naming.
        assert_eq!(
            pick_search_name(true, "Hades - Battle Out of Hell", "Hades", ""),
            Some("Hades".to_string())
        );
        // A stripped file name loses to the console header's title on
        // untrusted consoles: the glued token searches at nothing.
        assert_eq!(
            pick_search_name(false, "FooBar The Quest", "Foo: Bar The Quest", ""),
            Some("Foo: Bar The Quest".to_string())
        );
        // A healthy library keeps the file-first order: the scene file
        // carries the separator, a shortened library title does not.
        assert_eq!(
            pick_search_name(false, "Ace Combat 04 - Shattered Skies", "Ace Combat 04", ""),
            Some("Ace Combat 04 - Shattered Skies".to_string())
        );
        // Nameless candidates never win.
        assert_eq!(pick_search_name(false, "", "", ""), None);
        assert_eq!(pick_search_name(false, "", "Okami", ""), Some("Okami".to_string()));
        assert_eq!(
            pick_search_name(true, "", "", "Okami"),
            Some("Okami".to_string())
        );
    }

    #[test]
    fn test_raw_separator_richness_counts_stand_ins_skips_tags() {
        // Filesystem stand-ins count: cleaning turns them into word breaks.
        assert_eq!(raw_separator_richness("Echoes_ Shadows"), 1);
        assert_eq!(raw_separator_richness("Fire.Emblem.Echoes"), 2);
        // Punctuation deleted outright counts as nothing.
        assert_eq!(raw_separator_richness("NieRAutomata The End"), 0);
        // Dump tags are not the title's punctuation.
        assert_eq!(raw_separator_richness("Game (CTR-P-AJUE) [v1.0]"), 0);
        assert_eq!(
            raw_separator_richness("Emio \u{2013} The Smiling Man\u{a789} FDC"),
            2
        );
    }

    #[test]
    fn test_pick_search_name_keeps_a_file_whose_colon_went_underscore() {
        // The hShop scheme: leading title id, serial and version tags, and
        // the title's colon swapped for a filesystem-safe `_`. The
        // stand-in is still the separator — the console menu's SMDH short
        // title ("FE Echoes: …" fits 32 characters) must not take the
        // search over from a dump that accepts exactly.
        assert_eq!(
            pick_search_name(
                false,
                "00040000001B4000 Fire Emblem Echoes_ Shadows of Valentia (CTR-P-AJUE) (v0.1.0) (U)",
                "FE Echoes: Shadows of Valentia",
                "FE Echoes: Shadows of Valentia",
            )
            .as_deref(),
            Some("Fire Emblem Echoes Shadows of Valentia"),
        );
    }

    #[test]
    fn test_pick_search_name_glued_file_still_loses_to_punctuated_title() {
        // The punctuation was deleted outright: the file searches as a
        // glued token that exists in no ScreenScraper name, so the
        // punctuated title rescues the search.
        assert_eq!(
            pick_search_name(
                false,
                "NieRAutomata The End of YoRHa Edition",
                "NieR:Automata The End of YoRHa Edition",
                "",
            )
            .as_deref(),
            Some("NieR:Automata The End of YoRHa Edition"),
        );
        // A tag's hyphens are not the title's separators either.
        assert_eq!(
            pick_search_name(false, "Some Game (CTR-P-AJUE)", "Some Game: Sub", "").as_deref(),
            Some("Some Game: Sub"),
        );
    }

    #[test]
    fn test_search_term_prefers_the_distinctive_tail() {
        // The last segment is what both the dump and ScreenScraper's name
        // agree is the game's own; series heads bury the game.
        assert_eq!(search_term("Ace Combat 04 - Shattered Skies"), "Shattered Skies");
        assert_eq!(
            search_term("Simple 2000 Series Vol. 50 - The Daibijin"),
            "The Daibijin"
        );
        assert_eq!(search_term("13 Sentinels: Aegis Rim"), "Aegis Rim");
        // A trailing separator contributes nothing.
        assert_eq!(search_term("Katamari Damacy -"), "Katamari Damacy");
        // Separator-less names keep a two-word head: ScreenScraper's name
        // may hold a separator our words don't.
        assert_eq!(
            search_term("Phoenix Wright Ace Attorney Trilogy"),
            "Phoenix Wright"
        );
        // Punctuation-only words are not words.
        assert_eq!(search_term("Fear & Hunger"), "Fear Hunger");
        assert_eq!(search_term("Katamari Damacy"), "Katamari Damacy");
        assert_eq!(search_term("Okami"), "Okami");
    }

    #[test]
    fn test_region_hints_reads_dump_tags_only() {
        assert_eq!(
            region_hints("The Daibijin (Japan) [b]"),
            vec!["jp"]
        );
        assert_eq!(region_hints("Game (USA) (Europe)"), vec!["us", "eu"]);
        // Language lists and revisions are not regions; untagged names
        // and bracket-less names carry nothing.
        assert!(region_hints("Game (En,Fr,De) (Rev 1)").is_empty());
        assert!(region_hints("Katamari Damacy").is_empty());
        // Square brackets count too.
        assert_eq!(region_hints("Game [World]"), vec!["wor"]);
    }
}
