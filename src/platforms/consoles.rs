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

pub const CONSOLES: &[ConsoleDef] = &[
    ConsoleDef {
        id: "psx",
        display_name: "PS1",
        ra_console_id: 12,
        extensions: &["bin", "cue", "chd", "pbp", "iso", "ecm"],
        binary_names: &["duckstation-qt", "duckstation"],
        flatpak_id: "org.duckstation.DuckStation",
        emu_display_name: "DuckStation",
        fullscreen_flag: "--fullscreen",
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
        extensions: &["iso", "cso", "pbp", "prx"],
        binary_names: &["ppsspp", "PPSSPPSDL"],
        flatpak_id: "org.ppsspp.PPSSPP",
        emu_display_name: "PPSSPP",
        fullscreen_flag: "--fullscreen",
    },
];

pub fn find_console(id: &str) -> Option<&'static ConsoleDef> {
    CONSOLES.iter().find(|c| c.id == id)
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
        assert!(find_console("n64").is_none());
    }

    #[test]
    fn test_all_consoles_have_unique_ids() {
        let mut ids: Vec<_> = CONSOLES.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CONSOLES.len(), "duplicate console IDs");
    }
}
