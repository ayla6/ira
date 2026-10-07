use crate::kind::GameKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConsoleDef {
    pub id: &'static str,
    pub display_name: &'static str,
    /// Search-only terms: abbreviations and alternate names that should
    /// find the console without being shown anywhere ("ps1" for
    /// "PlayStation 1").
    pub search_aliases: &'static [&'static str],
    pub ra_console_id: u32,
    pub extensions: &'static [&'static str],
    pub binary_names: &'static [&'static str],
    pub flatpak_id: &'static str,
    pub emu_display_name: &'static str,
    pub fullscreen_flag: &'static str,
}

impl ConsoleDef {
    pub fn uses_rom_folder(&self) -> bool {
        !matches!(self.id, "ps3" | "ps4" | "psvita" | "wiiu")
    }

    /// The compact name for space-constrained lists (the settings
    /// sidebar): the abbreviations for the two long Nintendo titles,
    /// the display name everywhere else.
    pub fn short_name(&self) -> &'static str {
        match self.id {
            "nes" => "NES",
            "snes" => "SNES",
            _ => self.display_name,
        }
    }

    /// Everything a hidden search should match for this console: the
    /// display name, the id, and the aliases, lowercased with all
    /// whitespace dropped, so queries match regardless of spacing
    /// ("gameboy" finds "Game Boy"). Only the display name is ever shown.
    pub fn search_haystack(&self) -> String {
        let mut terms = String::new();
        terms.push_str(&self.display_name.to_lowercase());
        terms.push(' ');
        terms.push_str(self.id);
        for alias in self.search_aliases {
            terms.push(' ');
            terms.push_str(alias);
        }
        terms.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// The kind ROM-library entries of this console carry: Switch is a
    /// first-class emulator integration, every other ROM-folder console
    /// keeps the generic Retro kind.
    pub fn game_kind(&self) -> GameKind {
        if self.id == GameKind::Switch.as_str() {
            GameKind::Switch
        } else {
            GameKind::Retro
        }
    }

    /// Extensions a scan accepts for this console. The compressed Switch
    /// containers (NSZ/XCZ) stay invisible unless their toggle is on —
    /// only the Switch lists carry them, so other consoles never see
    /// the flag.
    pub fn scan_extensions(&self, compressed_switch_roms: bool) -> Vec<&'static str> {
        self.extensions
            .iter()
            .copied()
            .filter(|ext| compressed_switch_roms || !is_compressed_switch_extension(ext))
            .collect()
    }
}

pub const CONSOLES: &[ConsoleDef] = &[
    ConsoleDef {
        id: "psx",
        display_name: "PlayStation 1",
        search_aliases: &["ps1"],
        ra_console_id: 12,
        extensions: &["bin", "cue", "chd", "pbp", "iso", "ecm"],
        binary_names: &["duckstation-qt", "duckstation"],
        flatpak_id: "org.duckstation.DuckStation",
        emu_display_name: "DuckStation",
        fullscreen_flag: "-fullscreen",
    },
    ConsoleDef {
        id: "ps2",
        display_name: "PlayStation 2",
        search_aliases: &[],
        ra_console_id: 21,
        extensions: &["iso", "bin", "cue", "chd", "gz", "elf"],
        binary_names: &["pcsx2-qt", "pcsx2"],
        flatpak_id: "net.pcsx2.PCSX2",
        emu_display_name: "PCSX2",
        fullscreen_flag: "-fullscreen",
    },
    ConsoleDef {
        id: "psp",
        display_name: "PlayStation Portable",
        search_aliases: &[],
        ra_console_id: 41,
        extensions: &["iso", "cso", "chd", "pbp", "prx"],
        binary_names: &["ppsspp", "PPSSPPSDL"],
        flatpak_id: "org.ppsspp.PPSSPP",
        emu_display_name: "PPSSPP",
        fullscreen_flag: "--fullscreen",
    },
    ConsoleDef {
        id: "nes",
        display_name: "Nintendo Entertainment System",
        search_aliases: &["nes"],
        ra_console_id: 7,
        extensions: &["nes", "unf", "fds", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "snes",
        display_name: "Super Nintendo Entertainment System",
        search_aliases: &["snes"],
        ra_console_id: 3,
        extensions: &["smc", "sfc", "fig", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "gb",
        display_name: "Game Boy",
        search_aliases: &[],
        ra_console_id: 4,
        extensions: &["gb", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "gbc",
        display_name: "Game Boy Color",
        search_aliases: &[],
        ra_console_id: 6,
        extensions: &["gbc", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "gba",
        display_name: "Game Boy Advance",
        search_aliases: &[],
        ra_console_id: 5,
        extensions: &["gba", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "n64",
        display_name: "Nintendo 64",
        search_aliases: &[],
        ra_console_id: 2,
        extensions: &["n64", "z64", "v64", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "n64dd",
        display_name: "Nintendo 64DD",
        search_aliases: &[],
        ra_console_id: 2,
        extensions: &["ndd", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "nds",
        display_name: "Nintendo DS",
        search_aliases: &[],
        ra_console_id: 18,
        // DS ROMs (nds/srl/dsi/ids, optionally Zstandard-compressed) plus
        // the archive formats melonDS opens; the scanner matches the final
        // path suffix, so `game.nds.zst` and `game.tar.zst` both hit `zst`.
        extensions: &[
            "nds", "srl", "dsi", "ids", "zst", "zip", "7z", "tar", "gz", "tgz", "xz", "txz", "bz2",
            "tbz2", "lz4", "tlz4", "tzst", "z", "taz", "lz", "lzma", "tlz", "lrz", "tlrz", "lzo",
            "tzo",
        ],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "gc",
        display_name: "Nintendo GameCube",
        search_aliases: &[],
        ra_console_id: 16,
        extensions: &["iso", "gcm", "rvz", "gcz"],
        binary_names: &["dolphin-emu", "dolphin_emulator", "dolphin"],
        flatpak_id: "org.DolphinEmu.dolphin-emu",
        emu_display_name: "Dolphin",
        fullscreen_flag: "",
    },
    ConsoleDef {
        id: "wii",
        display_name: "Nintendo Wii",
        search_aliases: &[],
        ra_console_id: 19,
        extensions: &["iso", "wbfs", "gcm", "rvz", "gcz"],
        binary_names: &["dolphin-emu", "dolphin_emulator", "dolphin"],
        flatpak_id: "org.DolphinEmu.dolphin-emu",
        emu_display_name: "Dolphin",
        fullscreen_flag: "",
    },
    ConsoleDef {
        id: "virtualboy",
        display_name: "Virtual Boy",
        search_aliases: &["vb"],
        ra_console_id: 28,
        extensions: &["vb", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "sat",
        display_name: "Satellaview",
        search_aliases: &[],
        ra_console_id: 3,
        extensions: &["bs", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "md",
        display_name: "Mega Drive",
        search_aliases: &["sega genesis"],
        ra_console_id: 1,
        extensions: &["md", "bin", "gen", "smd", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "sms",
        display_name: "Master System",
        search_aliases: &["sega master system"],
        ra_console_id: 11,
        extensions: &["sms", "bin", "sg", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "saturn",
        display_name: "Saturn",
        search_aliases: &["sega saturn"],
        ra_console_id: 39,
        extensions: &["bin", "cue", "chd", "iso"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "dc",
        display_name: "Dreamcast",
        search_aliases: &["sega dreamcast"],
        ra_console_id: 40,
        extensions: &["cdi", "gdi", "chd"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "gg",
        display_name: "Game Gear",
        search_aliases: &["sega game gear"],
        ra_console_id: 15,
        extensions: &["gg", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "neogeo",
        display_name: "Neo Geo",
        search_aliases: &[],
        ra_console_id: 27,
        extensions: &["neo", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "ngp",
        display_name: "Neo Geo Pocket",
        search_aliases: &[],
        ra_console_id: 14,
        extensions: &["ngp", "ngc", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "pce",
        display_name: "PC Engine",
        search_aliases: &["turbografx-16"],
        ra_console_id: 8,
        extensions: &["pce", "bin", "cue", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "pcecd",
        display_name: "PC Engine CD",
        search_aliases: &["turbografx cd"],
        ra_console_id: 76,
        extensions: &["chd", "cue"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "ws",
        display_name: "WonderSwan",
        search_aliases: &[],
        ra_console_id: 53,
        extensions: &["ws", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "wsc",
        display_name: "WonderSwan Color",
        search_aliases: &[],
        ra_console_id: 53,
        extensions: &["wsc", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
];

pub fn all_consoles() -> impl Iterator<Item = &'static ConsoleDef> {
    CONSOLES
        .iter()
        .chain(super::esde_consoles::ESDE_CONSOLES.iter())
}

pub fn find_console(id: &str) -> Option<&'static ConsoleDef> {
    all_consoles().find(|c| c.id == id)
}

/// A platform id resolved once into a typed value, at the moment it is
/// read: every branch below answers from the value afterwards, so display
/// names never re-scan tables or re-match strings at each call site.
///
/// The fixed variants are load-bearing, not legacy, and must never gain
/// console-table entries: PC platform ids (`wine`, `linux`, `steam`,
/// `gog`) are kinds, not consoles — a table entry would enable
/// ROM-folder scanning for them — and `3ds` is emulator-integrated
/// outside the table by design (dedicated config fields,
/// emulator-library discovery, no RA). `Unknown` keeps forward
/// compatibility with ids no known source issued (title ids stored as
/// the platform on legacy rows): they render raw, exactly as before.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// A console-table entry (ROM-library and emulator-library consoles).
    Console(&'static ConsoleDef),
    Wine,
    Linux,
    Steam,
    Gog,
    ThreeDS,
    Unknown(String),
}

impl Platform {
    /// Resolve a stored platform id. The five fixed ids answer from a
    /// match — faster than the table scan they can never hit — and only
    /// genuine console ids reach `find_console`.
    pub fn parse(platform_id: &str) -> Self {
        match platform_id {
            "wine" => Self::Wine,
            "linux" => Self::Linux,
            "steam" => Self::Steam,
            "gog" => Self::Gog,
            "3ds" => Self::ThreeDS,
            _ => match find_console(platform_id) {
                Some(def) => Self::Console(def),
                None => Self::Unknown(platform_id.to_string()),
            },
        }
    }

    /// The stored id back: what the database, the queries and the data
    /// directories key on.
    pub fn id(&self) -> &str {
        match self {
            Self::Console(def) => def.id,
            Self::Wine => "wine",
            Self::Linux => "linux",
            Self::Steam => "steam",
            Self::Gog => "gog",
            Self::ThreeDS => "3ds",
            Self::Unknown(raw) => raw,
        }
    }

    /// Whether this platform carries no id at all (the empty unknown).
    pub fn is_empty(&self) -> bool {
        self.id().is_empty()
    }

    /// The name shown anywhere a platform renders: the console's full
    /// name, the fixed friendly name, or the raw id. No lookup involved.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Console(def) => def.display_name,
            Self::Wine => "Windows",
            Self::Linux => "Linux",
            Self::Steam => "Steam",
            Self::Gog => "GOG",
            Self::ThreeDS => "Nintendo 3DS",
            Self::Unknown(raw) => raw,
        }
    }

    /// Whether RetroAchievements covers this platform at all. Unlike
    /// [`console_has_ra`], this never scans: the answer rode in at parse
    /// time.
    pub fn has_ra(&self) -> bool {
        matches!(self, Self::Console(def) if def.ra_console_id != 0)
    }

    /// The console-table entry, if this platform is a table console.
    /// Unlike [`find_console`], this never scans.
    pub fn find_console_def(&self) -> Option<&'static ConsoleDef> {
        match self {
            Self::Console(def) => Some(def),
            _ => None,
        }
    }
}

impl Default for Platform {
    /// The empty id parses like every other unknown one.
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl From<&str> for Platform {
    /// Parse at the boundary: `"switch".into()` is `parse` spelled for
    /// struct literals and test fixtures.
    fn from(platform_id: &str) -> Self {
        Platform::parse(platform_id)
    }
}

impl std::str::FromStr for Platform {
    /// Infallible: unknown ids ride through as [`Platform::Unknown`].
    type Err = std::convert::Infallible;

    fn from_str(platform_id: &str) -> Result<Self, Self::Err> {
        Ok(Platform::parse(platform_id))
    }
}

/// String comparisons keep working through the id, so call sites read
/// unchanged while the value stays parsed: `platform == "switch"`.
impl PartialEq<&str> for Platform {
    fn eq(&self, other: &&str) -> bool {
        self.id() == *other
    }
}

impl PartialEq<Platform> for &str {
    fn eq(&self, other: &Platform) -> bool {
        *self == other.id()
    }
}

impl PartialEq<String> for Platform {
    fn eq(&self, other: &String) -> bool {
        self.id() == other
    }
}

impl PartialEq<Platform> for String {
    fn eq(&self, other: &Platform) -> bool {
        self.as_str() == other.id()
    }
}

/// The display name for a game's platform — the same fallback the
/// sidebar grouping and the autogroup Platform matcher use.
pub fn platform_display_name(platform_id: &str) -> String {
    Platform::parse(platform_id).display_name().to_string()
}

/// The compressed Switch containers, gated behind the
/// `compressed_switch_roms` toggle (off by default).
pub const COMPRESSED_SWITCH_EXTENSIONS: &[&str] = &["nsz", "xcz"];

/// True for the toggle-gated Switch extensions, matched case-insensitively
/// like every other extension check.
pub fn is_compressed_switch_extension(ext: &str) -> bool {
    COMPRESSED_SWITCH_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(ext))
}

/// True when the platform has RetroAchievements support at all. Consoles
/// with an `ra_console_id` of 0 (Nintendo Switch, and other ESDE-sourced
/// entries without an RA mapping) must never be offered RA matching.
pub fn console_has_ra(platform_id: &str) -> bool {
    Platform::parse(platform_id).has_ra()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_console_psx() {
        let c = find_console("psx").unwrap();
        assert_eq!(c.display_name, "PlayStation 1");
        assert_eq!(c.ra_console_id, 12);
        assert!(c.extensions.contains(&"bin"));
    }

    #[test]
    fn test_platform_display_name_falls_back_to_raw_id() {
        assert_eq!(platform_display_name("psx"), "PlayStation 1");
        assert_eq!(
            platform_display_name("snes"),
            "Super Nintendo Entertainment System"
        );
        // PC platforms show friendly names, matching the sidebar
        // grouping and the autogroup matcher.
        for (id, shown) in [
            ("wine", "Windows"),
            ("linux", "Linux"),
            ("steam", "Steam"),
            ("gog", "GOG"),
            ("3ds", "Nintendo 3DS"),
        ] {
            assert_eq!(platform_display_name(id), shown);
        }
        assert_eq!(platform_display_name("unknown-thing"), "unknown-thing");
    }

    #[test]
    fn test_platform_parse_round_trips_every_shape() {
        // Fixed ids parse without touching the table; consoles resolve to
        // their def; anything else rides through untouched.
        assert_eq!(Platform::parse("wine"), Platform::Wine);
        assert_eq!(Platform::parse("3ds"), Platform::ThreeDS);
        assert_eq!(
            Platform::parse("psx"),
            Platform::Console(find_console("psx").unwrap())
        );
        assert_eq!(
            Platform::parse("0004000000038800"),
            Platform::Unknown("0004000000038800".to_string())
        );
        for id in ["wine", "psx", "3ds", "0004000000038800", ""] {
            assert_eq!(Platform::parse(id).id(), id);
        }
    }

    #[test]
    fn test_platform_display_name_never_scans() {
        // The type carries the answer: display needs no lookup.
        assert_eq!(Platform::parse("snes").display_name(), "Super Nintendo Entertainment System");
        assert_eq!(Platform::parse("3ds").display_name(), "Nintendo 3DS");
        assert_eq!(Platform::parse("whatever").display_name(), "whatever");
    }

    #[test]
    fn test_search_haystack_finds_by_alias_only() {
        let psx = find_console("psx").unwrap();
        let hay = psx.search_haystack();
        for query in ["playstation1", "ps1", "psx"] {
            assert!(hay.contains(query), "{query} not found in haystack: {hay}");
        }
        let gba = find_console("gba").unwrap();
        let hay = gba.search_haystack();
        for query in ["gameboyadvance", "gba", "gameboy"] {
            assert!(hay.contains(query), "{query} not found in haystack: {hay}");
        }
    }

    #[test]
    fn test_search_haystack_is_normalized() {
        for c in all_consoles() {
            let hay = c.search_haystack();
            assert_eq!(hay, hay.to_lowercase(), "{}", c.id);
            assert!(!hay.chars().any(|ch| ch.is_whitespace()), "{}", c.id);
        }
    }

    #[test]
    fn test_search_haystack_covers_display_name_and_id() {
        for c in all_consoles() {
            let hay = c.search_haystack();
            let name = c.display_name.to_lowercase();
            let name: String = name.chars().filter(|ch| !ch.is_whitespace()).collect();
            assert!(hay.contains(&name), "{}", c.id);
            assert!(hay.contains(c.id), "{}", c.id);
        }
    }

    #[test]
    fn test_short_name_abbreviates_only_the_long_titles() {
        assert_eq!(find_console("nes").unwrap().short_name(), "NES");
        assert_eq!(find_console("snes").unwrap().short_name(), "SNES");
        assert_eq!(
            find_console("gba").unwrap().short_name(),
            "Game Boy Advance"
        );
        assert_eq!(
            find_console("psx").unwrap().short_name(),
            "PlayStation 1"
        );
    }

    #[test]
    fn test_find_console_unknown() {
        assert!(find_console("nonexistent").is_none());
    }

    #[test]
    fn test_find_console_virtualboy_uses_new_id() {
        assert_eq!(
            find_console("virtualboy").unwrap().display_name,
            "Virtual Boy"
        );
        assert!(find_console("vb").is_none());
    }

    #[test]
    fn test_find_console_includes_esde_system() {
        assert_eq!(
            find_console("3do").unwrap().display_name,
            "3DO Interactive Multiplayer"
        );
    }

    #[test]
    fn test_special_platforms_do_not_use_rom_folders() {
        assert!(!find_console("ps3").unwrap().uses_rom_folder());
        assert!(!find_console("ps4").unwrap().uses_rom_folder());
        assert!(!find_console("psvita").unwrap().uses_rom_folder());
        assert!(!find_console("wiiu").unwrap().uses_rom_folder());
        assert!(find_console("ps2").unwrap().uses_rom_folder());
    }

    #[test]
    fn test_game_kind_switch_vs_retro() {
        assert_eq!(
            find_console("switch").unwrap().game_kind(),
            GameKind::Switch
        );
        assert_eq!(find_console("saturn").unwrap().game_kind(), GameKind::Retro);
    }

    #[test]
    fn test_scan_extensions_hide_compressed_switch_roms_by_default() {
        let switch = find_console("switch").unwrap();
        let plain = switch.scan_extensions(false);
        assert!(plain.contains(&"nsp"));
        assert!(plain.contains(&"xci"));
        assert!(!plain.contains(&"nsz"));
        assert!(!plain.contains(&"xcz"));
        let full = switch.scan_extensions(true);
        assert!(full.contains(&"nsz"));
        assert!(full.contains(&"xcz"));
    }

    #[test]
    fn test_scan_extensions_leave_other_consoles_alone() {
        let gba = find_console("gba").unwrap();
        assert_eq!(gba.scan_extensions(false), gba.scan_extensions(true));
        assert_eq!(gba.scan_extensions(false), gba.extensions.to_vec());
    }

    #[test]
    fn test_is_compressed_switch_extension_matches_case_insensitively() {
        assert!(is_compressed_switch_extension("nsz"));
        assert!(is_compressed_switch_extension("XCZ"));
        assert!(!is_compressed_switch_extension("xci"));
        assert!(!is_compressed_switch_extension("nsp"));
    }

    #[test]
    fn test_all_consoles_have_unique_ids() {
        let mut ids: Vec<_> = all_consoles().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all_consoles().count(), "duplicate console IDs");
    }
}
