use super::consoles::ConsoleDef;

macro_rules! esde_console {
    ($id:literal, $display_name:literal, $extensions:expr, $binary_names:expr, $emu:literal) => {
        ConsoleDef {
            id: $id,
            display_name: $display_name,
            ra_console_id: 0,
            extensions: $extensions,
            binary_names: $binary_names,
            flatpak_id: "",
            emu_display_name: $emu,
            fullscreen_flag: "-f",
        }
    };
}

pub const ESDE_CONSOLES: &[ConsoleDef] = &[
    esde_console!(
        "3do",
        "3DO Interactive Multiplayer",
        &["bin", "chd", "cue", "iso", "7z", "zip"],
        &["opera"],
        "Opera"
    ),
    esde_console!(
        "amiga",
        "Commodore Amiga",
        &["adf", "adz", "chd", "cue", "dms", "hdf", "lha", "m3u", "uae", "7z", "zip"],
        &["fs-uae", "amiberry"],
        "FS-UAE"
    ),
    esde_console!(
        "amstradcpc",
        "Amstrad CPC",
        &["cpr", "dsk", "m3u", "sna", "tap", "tzx", "7z", "zip"],
        &["caprice32"],
        "Caprice32"
    ),
    esde_console!(
        "apple2",
        "Apple II",
        &["dsk", "do", "nib", "po", "woz", "2mg", "7z", "zip"],
        &["gsplus"],
        "GSplus"
    ),
    esde_console!(
        "arcade",
        "Arcade",
        &["7z", "zip"],
        &["mame", "mame64"],
        "MAME"
    ),
    esde_console!(
        "arduboy",
        "Arduboy",
        &["hex", "bin", "7z", "zip"],
        &["arduboy"],
        "Arduboy"
    ),
    esde_console!(
        "atari2600",
        "Atari 2600",
        &["a26", "bin", "rom", "7z", "zip"],
        &["stella"],
        "Stella"
    ),
    esde_console!(
        "atari5200",
        "Atari 5200",
        &["a52", "bin", "car", "rom", "7z", "zip"],
        &["atari800"],
        "Atari800"
    ),
    esde_console!(
        "atari7800",
        "Atari 7800",
        &["a78", "bin", "pro", "7z", "zip"],
        &["prosystem"],
        "ProSystem"
    ),
    esde_console!(
        "atari800",
        "Atari 800",
        &["a52", "atr", "atx", "car", "cas", "com", "rom", "xex", "7z", "zip"],
        &["atari800"],
        "Atari800"
    ),
    esde_console!(
        "atarijaguar",
        "Atari Jaguar",
        &["abs", "bin", "cdi", "cue", "j64", "jag", "rom", "7z", "zip"],
        &["virtualjaguar"],
        "Virtual Jaguar"
    ),
    esde_console!(
        "atarijaguarcd",
        "Atari Jaguar CD",
        &["bin", "cdi", "cue", "j64", "jag", "rom", "7z", "zip"],
        &["virtualjaguar"],
        "Virtual Jaguar"
    ),
    esde_console!(
        "atarilynx",
        "Atari Lynx",
        &["lnx", "lyx", "7z", "zip"],
        &["mednafen"],
        "Mednafen"
    ),
    esde_console!(
        "atarist",
        "Atari ST",
        &["st", "msa", "stx", "dim", "ipf", "m3u", "7z", "zip"],
        &["hatari"],
        "Hatari"
    ),
    esde_console!(
        "c64",
        "Commodore 64",
        &["crt", "d64", "d71", "d81", "prg", "tap", "t64", "x64", "7z", "zip"],
        &["x64sc", "vice"],
        "VICE"
    ),
    esde_console!(
        "cdimono1",
        "Philips CD-i",
        &["chd", "cue", "iso"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "cdtv",
        "Commodore CDTV",
        &["adf", "chd", "cue", "hdf", "iso", "m3u", "uae", "7z", "zip"],
        &["fs-uae", "amiberry"],
        "FS-UAE"
    ),
    esde_console!(
        "channelf",
        "Fairchild Channel F",
        &["bin", "chf", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "coco",
        "Tandy Color Computer",
        &["cas", "ccc", "dsk", "rom", "7z", "zip"],
        &["xroar", "mame"],
        "XRoar"
    ),
    esde_console!(
        "colecovision",
        "ColecoVision",
        &["bin", "col", "cv", "rom", "7z", "zip"],
        &["openmsx", "mame"],
        "openMSX"
    ),
    esde_console!(
        "gameandwatch",
        "Game & Watch",
        &["mgw", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "gamecom",
        "Game.com",
        &["tgc", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "intellivision",
        "Intellivision",
        &["int", "bin", "7z", "zip"],
        &["jzintv", "mame"],
        "jzIntv"
    ),
    esde_console!(
        "megaduck",
        "Mega Duck",
        &["bin", "rom", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "msx",
        "MSX",
        &["cas", "dsk", "mx1", "mx2", "rom", "sc", "7z", "zip"],
        &["openmsx"],
        "openMSX"
    ),
    esde_console!(
        "msx1",
        "MSX1",
        &["cas", "dsk", "mx1", "mx2", "rom", "sc", "7z", "zip"],
        &["openmsx"],
        "openMSX"
    ),
    esde_console!(
        "msx2",
        "MSX2",
        &["cas", "dsk", "mx1", "mx2", "rom", "sc", "7z", "zip"],
        &["openmsx"],
        "openMSX"
    ),
    esde_console!("neogeocd", "Neo Geo CD", &["chd", "cue"], &["mame"], "MAME"),
    esde_console!(
        "neogeocdjp",
        "Neo Geo CD (Japan)",
        &["chd", "cue"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "ngpc",
        "Neo Geo Pocket Color",
        &["ngc", "ngp", "ngpc", "npc", "7z", "zip"],
        &["mednafen"],
        "Mednafen"
    ),
    esde_console!(
        "odyssey2",
        "Magnavox Odyssey 2",
        &["bin", "7z", "zip"],
        &["o2em", "mame"],
        "O2EM"
    ),
    esde_console!(
        "pc88",
        "NEC PC-8800",
        &["88d", "cmt", "d88", "m3u", "t88", "u88"],
        &["quasi88"],
        "QUASI88"
    ),
    esde_console!(
        "pc98",
        "NEC PC-9800",
        &["2hd", "88d", "98d", "d88", "fdi", "hdi", "hdd", "m3u", "7z", "zip"],
        &["np2"],
        "Neko Project II"
    ),
    esde_console!(
        "pcfx",
        "NEC PC-FX",
        &["chd", "cue", "m3u", "toc", "7z", "zip"],
        &["mednafen"],
        "Mednafen"
    ),
    esde_console!(
        "pokemini",
        "Pokemon Mini",
        &["min", "7z", "zip"],
        &["pokemini"],
        "PokeMini"
    ),
    esde_console!(
        "ps3",
        "PlayStation 3",
        &["iso", "ps3", "ps3dir"],
        &["rpcs3"],
        "RPCS3"
    ),
    esde_console!(
        "ps4",
        "PlayStation 4",
        &["7z", "zip"],
        &["shadps4"],
        "shadPS4"
    ),
    esde_console!(
        "psvita",
        "PlayStation Vita",
        &["psvita"],
        &["vita3k"],
        "Vita3K"
    ),
    esde_console!(
        "sega32x",
        "Sega 32X",
        &["32x", "bin", "chd", "cue", "gen", "md", "smd", "7z", "zip"],
        &["picodrive"],
        "PicoDrive"
    ),
    esde_console!(
        "segacd",
        "Sega CD",
        &["bin", "chd", "cue", "iso", "m3u", "md", "smd", "7z", "zip"],
        &["kega-fusion"],
        "Kega Fusion"
    ),
    esde_console!(
        "sg-1000",
        "Sega SG-1000",
        &["bin", "gg", "rom", "sg", "sms", "7z", "zip"],
        &["mednafen"],
        "Mednafen"
    ),
    esde_console!(
        "sgb",
        "Super Game Boy",
        &["gb", "gbc", "sgb", "7z", "zip"],
        &["mgba"],
        "mGBA"
    ),
    esde_console!(
        "sufami",
        "SuFami Turbo",
        &["bs", "fig", "sfc", "smc", "7z", "zip"],
        &["snes9x"],
        "Snes9x"
    ),
    esde_console!(
        "supergrafx",
        "NEC SuperGrafx",
        &["chd", "cue", "pce", "rom", "sgx", "7z", "zip"],
        &["mednafen"],
        "Mednafen"
    ),
    esde_console!(
        "supervision",
        "Watara Supervision",
        &["bin", "sv", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "switch",
        "Nintendo Switch",
        &["nca", "nro", "nso", "nsp", "xci"],
        &["ryujinx"],
        "Ryujinx"
    ),
    esde_console!(
        "vectrex",
        "Vectrex",
        &["bin", "gam", "vc", "vec", "7z", "zip"],
        &["vecx", "mame"],
        "VecX"
    ),
    esde_console!(
        "videopac",
        "Philips Videopac G7000",
        &["bin", "7z", "zip"],
        &["o2em", "mame"],
        "O2EM"
    ),
    esde_console!(
        "vircon32",
        "Vircon32",
        &["v32", "7z", "zip"],
        &["vircon32"],
        "Vircon32"
    ),
    esde_console!(
        "vsmile",
        "VTech V.Smile",
        &["bin", "7z", "zip"],
        &["mame"],
        "MAME"
    ),
    esde_console!(
        "wiiu",
        "Nintendo Wii U",
        &["elf", "rpx", "wua", "wud", "wux"],
        &["cemu"],
        "Cemu"
    ),
    esde_console!(
        "x68000",
        "Sharp X68000",
        &["2hd", "88d", "d88", "dim", "hdf", "img", "m3u", "xdf", "7z", "zip"],
        &["px68k"],
        "PX68k"
    ),
    esde_console!(
        "xbox",
        "Microsoft Xbox",
        &["iso", "xiso"],
        &["xemu"],
        "xemu"
    ),
    esde_console!(
        "zx81",
        "Sinclair ZX81",
        &["p", "tzx", "7z", "zip"],
        &["81"],
        "EightyOne"
    ),
    esde_console!(
        "zxnext",
        "Sinclair ZX Spectrum Next",
        &["nex", "sna"],
        &["cspect"],
        "CSpect"
    ),
    esde_console!(
        "zxspectrum",
        "Sinclair ZX Spectrum",
        &["dsk", "mgt", "sna", "tap", "tzx", "z80", "7z", "zip"],
        &["fuse"],
        "Fuse"
    ),
];
