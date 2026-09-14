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

/// One age-rating classification (CERO / PEGI / ESRB / ...).
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScraperClassification {
    /// The board's name as ScreenScraper reports it ("PEGI", "CERO", ...).
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: String,
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

/// The ScreenScraper system id for an Ira platform id, or `None` when the
/// platform has no mapping — callers search without a system, exactly
/// like ES-DE does for unmapped platforms.
pub fn screenscraper_system_id(platform_id: &str) -> Option<u32> {
    SYSTEM_IDS
        .iter()
        .find(|(id, _)| *id == platform_id)
        .map(|(_, ss)| *ss)
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
        assert_eq!(screenscraper_system_id("snes"), Some(4));
        assert_eq!(screenscraper_system_id("wii"), Some(16));
        assert_eq!(screenscraper_system_id("switch"), Some(225));
    }

    #[test]
    fn test_screenscraper_system_id_unmapped_platforms() {
        // Steam, Wine and Linux games scrape from Steam/SGDB, not here.
        assert_eq!(screenscraper_system_id("steam"), None);
        assert_eq!(screenscraper_system_id("wine"), None);
        assert_eq!(screenscraper_system_id("madeup"), None);
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
