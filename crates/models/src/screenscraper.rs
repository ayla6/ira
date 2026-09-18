//! ScreenScraper.fr system ids for Ira's platforms, from the ES-DE
//! mapping (`references/emulationstation-de`, ScreenScraper.cpp): the id
//! every ScreenScraper API call carries as `systemeid` to keep searches
//! on one console.

/// A ScreenScraper-referenced entity — company or genre — kept by its
/// ScreenScraper id so several entries naming the same company resolve
/// to one thing.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScraperEntity {
    pub id: String,
    pub name: String,
}

/// One age-rating classification (CERO / PEGI / ESRB / ...): the board
/// and the value it assigned. Game answers carry no classification ids —
/// board and value are the whole identity.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

impl ScraperMetadata {
    /// Whether `list` already holds this entity: same id, or the same
    /// identifying tokens — a hand-minted local row and the source's own
    /// row describe one studio/genre, and filler-only names agree with
    /// nothing.
    fn holds(list: &[ScraperEntity], entity: &ScraperEntity) -> bool {
        let tokens = company_tokens(&entity.name);
        list.iter().any(|held| {
            held.id == entity.id
                || (!tokens.is_empty() && company_tokens(&held.name) == tokens)
        })
    }

    /// Diff one entity list against a fresh answer, entry by entry:
    ///
    /// - same id → the source's spelling of the name wins in place (it
    ///   corrects typos);
    /// - same identifying tokens but the fresh entry carries a real
    ///   ScreenScraper id where the stored one is a hand-minted local →
    ///   the stored entry graduates to the source's row, in place — the
    ///   same merge the store's local-company reconciliation performs,
    ///   so the draft shows one entry and never a spelling pair;
    /// - anything else the tokens call the same → the stored entry stays
    ///   (the source namespace itself distinguishes two real ids);
    /// - genuinely new → appended.
    fn fold_entities(list: &mut Vec<ScraperEntity>, fresh: &[ScraperEntity]) {
        'answer: for entity in fresh {
            for held in list.iter_mut() {
                if held.id == entity.id {
                    held.name = entity.name.clone();
                    continue 'answer;
                }
            }
            let fresh_is_real = entity.id.parse::<i64>().is_ok_and(|id| id > 0);
            if fresh_is_real {
                let tokens = company_tokens(&entity.name);
                if !tokens.is_empty() {
                    for held in list.iter_mut() {
                        let held_is_local =
                            held.id.parse::<i64>().is_ok_and(|id| id < 0);
                        if held_is_local && company_tokens(&held.name) == tokens {
                            *held = entity.clone();
                            continue 'answer;
                        }
                    }
                }
            }
            if !Self::holds(list, entity) {
                list.push(entity.clone());
            }
        }
    }

    /// Whether the classification boards of the two records differ —
    /// the revert button's changed test for the age-ratings row. Boards
    /// compare canonically, so an entry stored under an alias and an
    /// answer naming the board directly agree.
    pub fn classifications_differ(&self, other: &ScraperMetadata) -> bool {
        self.classifications.len() != other.classifications.len()
            || self.classifications.iter().any(|c| {
                !other
                    .classifications
                    .iter()
                    .any(|o| {
                        crate::ratings::canonical_kind(&o.kind) == crate::ratings::canonical_kind(&c.kind)
                            && o.value == c.value
                    })
            })
    }

    /// Whether the synopses of the two records differ — the revert
    /// button's changed test for the synopsis row.
    pub fn synopses_differ(&self, other: &ScraperMetadata) -> bool {
        self.synopses.len() != other.synopses.len()
            || self
                .synopses
                .iter()
                .any(|(lang, text)| other.synopses.iter().any(|(l, t)| l == lang && t != text))
    }

    /// Fold a fresh ScreenScraper answer in as *the match*: the picked
    /// entry's id always wins, scalar fields (date, players, rating,
    /// synopses) win where the answer has them — and the multi-valued
    /// fields (companies, genres, age ratings) are diffed entry by
    /// entry, merging spellings of one studio/genre into the source's
    /// row and keeping genuinely distinct ones side by side. Nothing
    /// already stored is dropped: for age boards the stored value stays
    /// (Steam garnish runs first, so the store's say on a board wins)
    /// and every board the answer brings that isn't stored yet is kept.
    pub fn merge_match(&mut self, fresh: &ScraperMetadata) {
        self.ss_id = fresh.ss_id.clone();
        if !fresh.release_date.is_empty() {
            self.release_date = fresh.release_date.clone();
            self.release_timestamp = fresh.release_timestamp;
            self.release_dates = fresh.release_dates.clone();
        }
        if !fresh.players.is_empty() {
            self.players = fresh.players.clone();
        }
        if fresh.rating > 0.0 {
            self.rating = fresh.rating;
        }
        if !fresh.synopses.is_empty() {
            self.synopses = fresh.synopses.clone();
        }
        Self::fold_entities(&mut self.developers, &fresh.developers);
        Self::fold_entities(&mut self.publishers, &fresh.publishers);
        Self::fold_entities(&mut self.genres, &fresh.genres);
        for class in &fresh.classifications {
            let canonical = crate::ratings::canonical_kind(&class.kind);
            if !self
                .classifications
                .iter()
                .any(|held| crate::ratings::canonical_kind(&held.kind) == canonical)
            {
                self.classifications.push(ScraperClassification {
                    kind: canonical,
                    value: class.value.clone(),
                });
            }
        }
    }

    /// Fill only the gaps from a fresh answer: anything already stored —
    /// including an entry the source has since emptied — stays. Returns
    /// whether anything was filled. The epoch date an old bug wrote
    /// counts as a hole, not as data.
    pub fn fill_gaps(&mut self, fresh: &ScraperMetadata) -> bool {
        let mut changed = false;
        if self.ss_id.is_empty() && !fresh.ss_id.is_empty() {
            self.ss_id = fresh.ss_id.clone();
            changed = true;
        }
        let date_missing = self.release_date.is_empty() || self.release_date == "1970-01-01";
        if date_missing && !fresh.release_date.is_empty() {
            self.release_date = fresh.release_date.clone();
            self.release_timestamp = fresh.release_timestamp;
            self.release_dates = fresh.release_dates.clone();
            changed = true;
        }
        if self.players.is_empty() && !fresh.players.is_empty() {
            self.players = fresh.players.clone();
            changed = true;
        }
        if self.rating <= 0.0 && fresh.rating > 0.0 {
            self.rating = fresh.rating;
            changed = true;
        }
        if self.synopses.is_empty() && !fresh.synopses.is_empty() {
            self.synopses = fresh.synopses.clone();
            changed = true;
        }
        for (list, fresh_list) in [
            (&mut self.developers, &fresh.developers),
            (&mut self.publishers, &fresh.publishers),
            (&mut self.genres, &fresh.genres),
        ] {
            for entity in fresh_list {
                if !list.iter().any(|held| held.id == entity.id) {
                    list.push(entity.clone());
                    changed = true;
                }
            }
        }
        for class in &fresh.classifications {
            let canonical = crate::ratings::canonical_kind(&class.kind);
            if !self
                .classifications
                .iter()
                .any(|held| crate::ratings::canonical_kind(&held.kind) == canonical)
            {
                self.classifications.push(class.clone());
                changed = true;
            }
        }
        changed
    }
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
    use super::{screenscraper_system_id, ScraperClassification, ScraperEntity, ScraperMetadata};

    fn entity(id: &str, name: &str) -> ScraperEntity {
        ScraperEntity {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn class(kind: &str, value: &str) -> ScraperClassification {
        ScraperClassification {
            kind: kind.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn test_merge_match_replaces_scalars_but_grows_lists() {
        let mut stored = ScraperMetadata {
            ss_id: String::new(),
            release_date: "15 Sep, 2014".into(),
            developers: vec![entity("-1", "Valve Corporation")],
            genres: vec![entity("100", "Shooter")],
            classifications: vec![class("ESRB", "M")],
            synopses: vec![("en".into(), "Steam's words.".into())],
            ..Default::default()
        };
        let fresh = ScraperMetadata {
            ss_id: "2124".into(),
            release_date: "1993-12-18".into(),
            release_timestamp: 1,
            developers: vec![entity("2911", "Chunsoft")],
            genres: vec![entity("2620", "Role Playing Game")],
            classifications: vec![class("PEGI", "12"), class("usk", "16")],
            players: "1".into(),
            rating: 15.0,
            synopses: vec![("en".into(), "SS's words.".into())],
            ..Default::default()
        };
        stored.merge_match(&fresh);
        // Scalars: the match wins where it has data.
        assert_eq!(stored.ss_id, "2124");
        assert_eq!(stored.release_date, "1993-12-18");
        assert_eq!(stored.players, "1");
        assert_eq!(stored.rating, 15.0);
        assert_eq!(stored.synopses[0].1, "SS's words.");
        // Lists: mixed together, nothing replaced.
        assert_eq!(
            stored.developers.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["-1", "2911"]
        );
        assert_eq!(
            stored.genres.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["100", "2620"]
        );
        // Boards fold to the canonical kind; the stored value wins for
        // a board both sides rated.
        assert_eq!(
            stored
                .classifications
                .iter()
                .map(|c| (c.kind.as_str(), c.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("ESRB", "M"), ("PEGI", "12"), ("USK", "16")]
        );
    }

    #[test]
    fn test_merge_match_dedupes_by_name_tokens() {
        let mut stored = ScraperMetadata {
            developers: vec![entity("-1", "Valve Corporation")],
            ..Default::default()
        };
        // The source's own row for the same studio: it graduates the
        // local entry instead of piling a spelling pair on top.
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("594", "Valve")],
            ..Default::default()
        });
        assert_eq!(stored.developers, vec![entity("594", "Valve")]);
        // Different id and different name: appended.
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("2911", "Chunsoft")],
            ..Default::default()
        });
        assert_eq!(stored.developers.len(), 2);
    }

    #[test]
    fn test_merge_match_graduates_a_local_company_to_the_source_row() {
        // Steam garnish minted the studio as a local row; the match
        // carries the real ScreenScraper one. The stored entry becomes
        // the source's — same studio, canonical id and spelling — and
        // no spelling pair survives the merge.
        let mut stored = ScraperMetadata {
            developers: vec![entity("-1", "Valve Corporation")],
            genres: vec![entity("-3", "role playing game")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("594", "Valve")],
            genres: vec![entity("2620", "Role Playing Game")],
            ..Default::default()
        });
        assert_eq!(
            stored.developers,
            vec![entity("594", "Valve")],
            "local id and spelling graduate to the source's row"
        );
        assert_eq!(
            stored.genres,
            vec![entity("2620", "Role Playing Game")],
            "hand-typed genres graduate the same way"
        );
    }

    #[test]
    fn test_merge_match_never_downgrades_a_real_id_to_a_local_one() {
        // The stored row is the source's already; a hand-minted local
        // spelling arriving late must not replace it.
        let mut stored = ScraperMetadata {
            developers: vec![entity("594", "Valve")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("-1", "Valve Corporation")],
            ..Default::default()
        });
        assert_eq!(stored.developers, vec![entity("594", "Valve")]);
    }

    #[test]
    fn test_merge_match_keeps_two_real_rows_the_source_distinguishes() {
        // Two different ScreenScraper ids whose token sets agree: the
        // stored one wins and the answer's adds nothing — one row, not
        // a spelling pair.
        let mut stored = ScraperMetadata {
            developers: vec![entity("10", "Sega")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("20", "Sega Games")],
            ..Default::default()
        });
        assert_eq!(stored.developers, vec![entity("10", "Sega")]);
    }

    #[test]
    fn test_merge_match_refreshes_a_name_under_the_same_id() {
        // The source fixed a spelling: the correction rides in.
        let mut stored = ScraperMetadata {
            developers: vec![entity("2911", "Chunsoft")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            developers: vec![entity("2911", "ChunSoft")],
            ..Default::default()
        });
        assert_eq!(stored.developers, vec![entity("2911", "ChunSoft")]);
    }

    #[test]
    fn test_merge_match_folds_board_aliases_and_keeps_the_stored_value() {
        // Steam stored the board under its canonical id; ScreenScraper
        // names an alias. One board, one entry — and the stored value
        // stays, since the garnish ran first.
        let mut stored = ScraperMetadata {
            classifications: vec![class("USK", "16")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            classifications: vec![class("STEAM_GERMANY", "12"), class("PEGI", "12")],
            ..Default::default()
        });
        assert_eq!(
            stored
                .classifications
                .iter()
                .map(|c| (c.kind.as_str(), c.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("USK", "16"), ("PEGI", "12")]
        );
        // A board only the source knows is kept — every rating survives.
        let mut bare = ScraperMetadata::default();
        bare.merge_match(&ScraperMetadata {
            classifications: vec![class("steam_germany", "6")],
            ..Default::default()
        });
        assert_eq!(
            bare.classifications,
            vec![class("USK", "6")],
            "aliases fold to the canonical kind on the way in"
        );
    }

    #[test]
    fn test_merge_match_keeps_data_when_the_answer_is_empty() {
        let mut stored = ScraperMetadata {
            release_date: "2015-09-15".into(),
            rating: 18.0,
            genres: vec![entity("100", "Shooter")],
            ..Default::default()
        };
        stored.merge_match(&ScraperMetadata {
            ss_id: "9".into(),
            ..Default::default()
        });
        // An empty answer must not erase anything, not even the pieces a
        // match normally owns.
        assert_eq!(stored.release_date, "2015-09-15");
        assert_eq!(stored.rating, 18.0);
        assert_eq!(stored.genres.len(), 1);
        assert_eq!(stored.ss_id, "9");
    }

    #[test]
    fn test_fill_gaps_leaves_everything_stored_alone() {
        let mut stored = ScraperMetadata {
            ss_id: "2124".into(),
            release_date: "1993-12-18".into(),
            release_timestamp: 1,
            developers: vec![entity("2911", "Chunsoft")],
            classifications: vec![class("PEGI", "12")],
            synopses: vec![("en".into(), "Kept.".into())],
            ..Default::default()
        };
        let fresh = ScraperMetadata {
            ss_id: "other".into(),
            release_date: "2020-01-01".into(),
            release_timestamp: 2,
            players: "1-4".into(),
            rating: 15.0,
            developers: vec![entity("2911", "Chunsoft"), entity("1", "Nintendo")],
            genres: vec![entity("2620", "Role Playing Game")],
            classifications: vec![class("ESRB", "E"), class("PEGI", "18")],
            synopses: vec![("en".into(), "Not kept.".into())],
            ..Default::default()
        };
        assert!(stored.fill_gaps(&fresh));
        // The id and every stored piece stay; only holes were filled.
        assert_eq!(stored.ss_id, "2124");
        assert_eq!(stored.release_date, "1993-12-18");
        assert_eq!(stored.players, "1-4");
        assert_eq!(stored.rating, 15.0);
        assert_eq!(stored.synopses[0].1, "Kept.");
        assert_eq!(stored.developers.len(), 2);
        assert_eq!(stored.genres.len(), 1);
        assert_eq!(stored.classifications.len(), 2);
        assert_eq!(stored.classifications[1].value, "E");
        // A second pass finds nothing left to fill.
        assert!(!stored.fill_gaps(&fresh));
    }

    #[test]
    fn test_fill_gaps_treats_the_epoch_date_as_a_hole() {
        let mut stored = ScraperMetadata {
            release_date: "1970-01-01".into(),
            ..Default::default()
        };
        assert!(stored.fill_gaps(&ScraperMetadata {
            release_date: "2015-09-15".into(),
            release_timestamp: 1_442_275_200,
            ..Default::default()
        }));
        assert_eq!(stored.release_date, "2015-09-15");
    }

    #[test]
    fn test_differ_checks_compare_by_content() {
        let a = ScraperMetadata {
            classifications: vec![class("PEGI", "12")],
            synopses: vec![("en".into(), "One.".into())],
            ..Default::default()
        };
        let same = ScraperMetadata {
            classifications: vec![class("pegi", "12")],
            synopses: vec![("en".into(), "One.".into())],
            ..Default::default()
        };
        // Board aliases fold: an entry stored as USK and an answer
        // naming STEAM_GERMANY are the same rating, not a difference.
        let aliased = ScraperMetadata {
            classifications: vec![class("STEAM_GERMANY", "16")],
            synopses: vec![],
            ..Default::default()
        };
        let canonical = ScraperMetadata {
            classifications: vec![class("USK", "16")],
            synopses: vec![],
            ..Default::default()
        };
        let other = ScraperMetadata {
            classifications: vec![class("PEGI", "16")],
            synopses: vec![("en".into(), "Two.".into()), ("de".into(), "Zwei.".into())],
            ..Default::default()
        };
        assert!(!a.classifications_differ(&same));
        assert!(!a.synopses_differ(&same));
        assert!(!aliased.classifications_differ(&canonical));
        assert!(a.classifications_differ(&other));
        assert!(a.synopses_differ(&other));
    }

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
