pub struct ConsoleDef {
    pub id: &'static str,
    pub display_name: &'static str,
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
}

pub const CONSOLES: &[ConsoleDef] = &[
    ConsoleDef {
        id: "psx",
        display_name: "PS1",
        ra_console_id: 12,
        extensions: &["bin", "cue", "chd", "pbp", "iso", "ecm"],
        binary_names: &["duckstation-qt", "duckstation"],
        flatpak_id: "org.duckstation.DuckStation",
        emu_display_name: "DuckStation",
        fullscreen_flag: "-fullscreen",
    },
    ConsoleDef {
        id: "ps2",
        display_name: "PS2",
        ra_console_id: 21,
        extensions: &["iso", "bin", "cue", "chd", "gz", "elf"],
        binary_names: &["pcsx2-qt", "pcsx2"],
        flatpak_id: "net.pcsx2.PCSX2",
        emu_display_name: "PCSX2",
        fullscreen_flag: "-fullscreen",
    },
    ConsoleDef {
        id: "psp",
        display_name: "PSP",
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
        ra_console_id: 16,
        extensions: &["iso", "gcm", "gcz"],
        binary_names: &["dolphin-emu", "dolphin"],
        flatpak_id: "org.DolphinEmu.dolphin-emu",
        emu_display_name: "Dolphin",
        fullscreen_flag: "",
    },
    ConsoleDef {
        id: "wii",
        display_name: "Wii",
        ra_console_id: 19,
        extensions: &["iso", "wbfs", "gcm", "gcz"],
        binary_names: &["dolphin-emu", "dolphin"],
        flatpak_id: "org.DolphinEmu.dolphin-emu",
        emu_display_name: "Dolphin",
        fullscreen_flag: "",
    },
    ConsoleDef {
        id: "virtualboy",
        display_name: "Virtual Boy",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_console_psx() {
        let c = find_console("psx").unwrap();
        assert_eq!(c.display_name, "PS1");
        assert_eq!(c.ra_console_id, 12);
        assert!(c.extensions.contains(&"bin"));
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
    fn test_all_consoles_have_unique_ids() {
        let mut ids: Vec<_> = all_consoles().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all_consoles().count(), "duplicate console IDs");
    }
}
