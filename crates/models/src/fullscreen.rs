//! Fullscreen CLI arguments for emulator integrations without a
//! `ConsoleDef` (PS3/PS4/PS Vita/Wii U/3DS launch through dedicated
//! launchers, not the ROM-folder path). ROM-folder consoles carry their
//! flag in `ConsoleDef::fullscreen_flag` instead.

use crate::kind::GameKind;

/// Arguments that put the emulator in fullscreen, prepended to the game
/// arguments. Empty for kinds with no fullscreen flag (PC games and the
/// ROM-folder consoles resolved through `ConsoleDef`).
pub fn fullscreen_args(kind: GameKind) -> &'static [&'static str] {
    match kind {
        // shadPS4's flag takes an explicit boolean value.
        GameKind::Ps4 => &["--fullscreen", "true"],
        GameKind::Ps3 | GameKind::PsVita => &["--fullscreen"],
        GameKind::WiiU | GameKind::ThreeDS => &["-f"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fullscreen_args_per_kind() {
        assert_eq!(fullscreen_args(GameKind::Ps4), &["--fullscreen", "true"]);
        assert_eq!(fullscreen_args(GameKind::Ps3), &["--fullscreen"]);
        assert_eq!(fullscreen_args(GameKind::PsVita), &["--fullscreen"]);
        assert_eq!(fullscreen_args(GameKind::WiiU), &["-f"]);
        assert_eq!(fullscreen_args(GameKind::ThreeDS), &["-f"]);
    }

    #[test]
    fn test_fullscreen_args_empty_for_unsupported_kinds() {
        for kind in [
            GameKind::Wine,
            GameKind::Linux,
            GameKind::Other,
            GameKind::Switch,
            GameKind::Steam,
            GameKind::Retro,
        ] {
            assert!(fullscreen_args(kind).is_empty());
        }
    }
}
