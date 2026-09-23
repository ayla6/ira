use crate::kind::GameKind;

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
        display_name: "NES",
        search_aliases: &["nintendo entertainment system"],
        ra_console_id: 7,
        extensions: &["nes", "unf", "fds", "7z", "zip"],
        binary_names: &[],
        flatpak_id: "",
        emu_display_name: "RetroArch",
        fullscreen_flag: "-f",
    },
    ConsoleDef {
        id: "snes",
        display_name: "SNES",
        search_aliases: &["super nintendo entertainment system"],
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
        display_name: "GameCube",
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
        display_name: "Wii",
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
    find_console(platform_id).is_some_and(|def| def.ra_console_id != 0)
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
