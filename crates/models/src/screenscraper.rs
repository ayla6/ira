//! ScreenScraper.fr system ids for Ira's platforms, from the ES-DE
//! mapping (`references/emulationstation-de`, ScreenScraper.cpp): the id
//! every ScreenScraper API call carries as `systemeid` to keep searches
//! on one console.

/// A ScreenScraper-referenced entity — company or genre — kept by its
/// ScreenScraper id so several entries naming the same company resolve
/// to one thing.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScraperEntity {
    pub id: String,
    pub name: String,
}

/// One age-rating classification (CERO / PEGI / ESRB / ...): the board
/// and the value it assigned. Game answers carry no classification ids —
/// board and value are the whole identity.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScraperClassification {
    /// The board's name as ScreenScraper reports it ("PEGI", "CERO", ...).
    pub kind: String,
    /// The value the board assigned ("18", "A", "E10+", ...).
    pub value: String,
}

/// The metadata a ScreenScraper match provides, ready for storage.
/// Multi-valued fields serialize to JSON in their columns: companies and
/// genres carry ids, dates keep every region, synopses keep every
/// language the source answered with.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScraperMetadata {
    pub ss_id: String,
    /// The display date: the first dated region of the match.
    pub release_date: String,
    pub release_timestamp: i64,
    /// Every dated region as `(region, YYYY-MM-DD)` pairs.
    #[serde(default)]
    pub release_dates: Vec<(String, String)>,
    pub developers: Vec<ScraperEntity>,
    pub publishers: Vec<ScraperEntity>,
    /// All English genres, the primary one first.
    #[serde(default)]
    pub genres: Vec<ScraperEntity>,
    pub players: String,
    /// ScreenScraper's community note, 0..20 (-1 = none given).
    pub rating: f64,
    #[serde(default)]
    pub classifications: Vec<ScraperClassification>,
    /// Every synopsis as `(language, text)` pairs.
    #[serde(default)]
    pub synopses: Vec<(String, String)>,
}

/// Consoles whose games are identified on ScreenScraper by disc serial
/// instead of any file digest: multi-gigabyte images (PS2 DVDs, Wii/GC
/// discs) whose repacks — chd, rvz, wux — scramble every hash, while the
/// serial printed on the disc survives all of them.
pub fn screenscraper_matches_by_serial(platform_id: &str) -> bool {
    matches!(platform_id, "ps2" | "ps3" | "wii" | "gc" | "psx" | "psp")
}

/// Consoles whose images are too big for content hashing to be worth it.
/// Disc consoles here match by serial instead; Switch and Wii U have no
/// serials on ScreenScraper at all and match by title.
pub fn screenscraper_hashes_content(platform_id: &str) -> bool {
    !matches!(platform_id, "ps2" | "ps3" | "wii" | "gc" | "switch" | "wiiu")
}

/// Consoles whose scan-time titles come from official metadata (param.sfo,
/// nsw/Eden lists, xml) rather than file names or shortened ROM headers —
/// good enough to keep even when a ScreenScraper match lands. Everything
/// else — filename stems, 3DS internal names — gets replaced by the match.
pub fn title_from_trusted_source(platform_id: &str) -> bool {
    matches!(platform_id, "ps3" | "ps4" | "psvita" | "switch" | "wiiu")
}

/// The console a game's ScreenScraper lookups run under. PS3 and PS4
/// games store their native product code as the platform id — NPUB30698,
/// CUSA12112 — which names a release, not a console; their kind is what
/// carries the console. Every other game's platform_id already is one.
pub fn scraper_console_id(kind: crate::GameKind, platform_id: &str) -> String {
    match kind {
        crate::GameKind::Ps4 => "ps4".to_string(),
        crate::GameKind::Ps3 => "ps3".to_string(),
        _ => platform_id.to_string(),
    }
}

/// The corporate words a store appends to a studio's name — Steam says
/// "Naughty Dog, LLC" where ScreenScraper writes "Naughty Dog", "Sega
/// Games" where it writes "Sega" — dropped when companies compare.
const COMPANY_FILLER: &[&str] = &[
    "llc", "inc", "ltd", "limited", "gmbh", "co", "corp", "corporation", "studio", "studios",
    "games", "entertainment", "interactive", "software", "digital", "sa", "sas", "srl", "bv",
    "nv", "plc", "ag", "kk",
];

/// A company name reduced to its identifying tokens: lowercased, split
/// on everything non-alphanumeric ("Inc." splits clean away), corporate
/// filler dropped, order ignored ("Bandai Namco" and "Namco Bandai"
/// agree).
pub fn company_tokens(name: &str) -> Vec<String> {
    let mut tokens: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty() && !COMPANY_FILLER.contains(token))
        .map(str::to_string)
        .collect();
    tokens.sort();
    tokens
}

/// The ScreenScraper system id for an Ira platform id, or `None` when the
/// platform has no mapping — callers search without a system, exactly
/// like ES-DE does for unmapped platforms.
pub fn screenscraper_system_id(platform_id: &str) -> Option<u32> {
    SYSTEM_IDS
        .iter()
        .find(|(id, _)| *id == platform_id)
        .map(|(_, ss)| *ss)
}

/// The ScreenScraper system a PC game scrapes under: Windows for Steam
/// and Wine games (ScreenScraper's own Steam system redirects there, the
/// same way ES-DE maps Valve Steam), Linux for native Linux games. PC
/// platform ids are store app ids rather than console names, so the
/// game's kind is what carries the distinction.
pub fn screenscraper_pc_system_id(kind: crate::GameKind) -> Option<u32> {
    match kind {
        crate::GameKind::Linux => Some(145),
        crate::GameKind::Wine | crate::GameKind::Steam => Some(138),
        _ => None,
    }
}

/// `(ira platform id, screenscraper systemeid)`.
const SYSTEM_IDS: &[(&str, u32)] = &[
    ("psx", 57),
    ("ps2", 58),
    ("ps3", 59),
    ("ps4", 60),
    ("psp", 61),
    ("psvita", 62),
    ("nes", 3),
    ("snes", 4),
    ("gb", 9),
    ("gbc", 10),
    ("gba", 12),
    ("sgb", 127),
    ("sufami", 108),
    ("n64", 14),
    ("n64dd", 14),
    ("nds", 15),
    ("3ds", 17),
    ("gc", 13),
    ("wii", 16),
    ("wiiu", 18),
    ("switch", 225),
    ("virtualboy", 11),
    ("sat", 107),
    ("gameandwatch", 52),
    ("md", 1),
    ("sms", 2),
    ("gg", 21),
    ("saturn", 22),
    ("dc", 23),
    ("segacd", 20),
    ("sega32x", 19),
    ("neogeo", 142),
    ("neogeocdjp", 70),
    ("ngp", 25),
    ("ngpc", 82),
    ("pce", 31),
    ("pcecd", 114),
    ("pcfx", 72),
    ("supergrafx", 105),
    ("ws", 45),
    ("wsc", 46),
    ("supervision", 207),
    ("pokemini", 211),
    ("gamecom", 121),
    ("3do", 29),
    ("amiga", 64),
    ("amstradcpc", 65),
    ("apple2", 86),
    ("arcade", 75),
    ("arduboy", 263),
    ("atari2600", 26),
    ("atari5200", 40),
    ("atari7800", 41),
    ("atari800", 43),
    ("atarijaguar", 27),
    ("atarijaguarcd", 171),
    ("atarilynx", 28),
    ("atarist", 42),
    ("c64", 66),
    ("cdimono1", 133),
    ("cdtv", 129),
    ("channelf", 80),
    ("coco", 144),
    ("colecovision", 48),
    ("intellivision", 115),
    ("megaduck", 90),
    ("msx", 113),
    ("msx1", 113),
    ("msx2", 116),
    ("odyssey2", 104),
    ("videopac", 104),
    ("pc88", 221),
    ("pc98", 208),
    ("vectrex", 102),
    ("vsmile", 120),
    ("x68000", 79),
    ("xbox", 32),
    ("zxspectrum", 76),
    ("zx81", 77),
    ("vircon32", 272),
];

#[cfg(test)]
mod tests {
    use super::screenscraper_system_id;

    #[test]
    fn test_screenscraper_system_id_known_platforms() {
        assert_eq!(screenscraper_system_id("psx"), Some(57));
        // The azahar integration's console has no ConsoleDef, so only this
        // pin keeps it mapped.
        assert_eq!(screenscraper_system_id("3ds"), Some(17));
        assert_eq!(screenscraper_system_id("snes"), Some(4));
        assert_eq!(screenscraper_system_id("wii"), Some(16));
        assert_eq!(screenscraper_system_id("switch"), Some(225));
    }

    #[test]
    fn test_screenscraper_system_id_unmapped_platforms() {
        // Steam, Wine and Linux games carry store app ids as their
        // platform, not these names — they map through the kind instead.
        assert_eq!(screenscraper_system_id("steam"), None);
        assert_eq!(screenscraper_system_id("wine"), None);
        assert_eq!(screenscraper_system_id("madeup"), None);
    }

    #[test]
    fn test_company_tokens_drop_filler_and_order() {
        assert_eq!(
            super::company_tokens("Naughty Dog, LLC"),
            super::company_tokens("naughty dog")
        );
        // Punctuation rides on the token ("Inc." is not "inc") — split
        // before the filler compare.
        assert_eq!(
            super::company_tokens("Team Salvato Inc."),
            super::company_tokens("Team Salvato")
        );
        assert_eq!(
            super::company_tokens("Bandai Namco"),
            super::company_tokens("Namco Bandai")
        );
        assert!(super::company_tokens("LLC").is_empty());
        assert!(super::company_tokens("Studios, Inc.").is_empty());
    }

    #[test]
    fn test_screenscraper_pc_system_id_follows_the_kind() {
        use crate::screenscraper_pc_system_id;
        // Steam and Wine games scrape as Windows, Linux games as Linux;
        // console kinds have no PC system.
        assert_eq!(
            screenscraper_pc_system_id(crate::GameKind::Steam),
            Some(138)
        );
        assert_eq!(
            screenscraper_pc_system_id(crate::GameKind::Wine),
            Some(138)
        );
        assert_eq!(
            screenscraper_pc_system_id(crate::GameKind::Linux),
            Some(145)
        );
        assert_eq!(screenscraper_pc_system_id(crate::GameKind::Retro), None);
    }

    #[test]
    fn test_screenscraper_system_id_covers_every_rom_console() {
        for console in crate::all_consoles() {
            if console.ra_console_id != 0 || console.id == "switch" {
                assert!(
                    screenscraper_system_id(console.id).is_some(),
                    "console {} lacks a ScreenScraper id",
                    console.id
                );
            }
        }
    }
}
