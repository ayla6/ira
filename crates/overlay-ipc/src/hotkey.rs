//! Hotkey parsing — converts human-readable strings ("Shift+Tab", "F12")
//! into evdev keycodes + modifier masks for use by the input hooks.
//!
//! Evdev keycodes are used as the common format:
//!   - Wayland wl_keyboard.key events use evdev keycodes directly.
//!   - X11 keycodes = evdev keycode + 8 (on modern Linux).
//!   - evdev /dev/input events use evdev keycodes directly.
//!
//! Modifier masks are the same for X11 and Wayland on Linux:
//!   Shift=0x01, Ctrl=0x04, Alt=0x08, Super=0x40

/// Modifier mask bits (same for X11 and Wayland on Linux).
pub const MOD_SHIFT: u32 = 0x01;
pub const MOD_CTRL: u32 = 0x04;
pub const MOD_ALT: u32 = 0x08;
pub const MOD_SUPER: u32 = 0x40;

/// X11 keycode offset: X11 keycode = evdev keycode + 8.
pub const X11_KEYCODE_OFFSET: u32 = 8;

/// Default evdev keycodes.
pub const DEFAULT_TOGGLE_KEYCODE: u32 = 15; // Tab
pub const DEFAULT_TOGGLE_MODS: u32 = MOD_SHIFT;
pub const DEFAULT_SCREENSHOT_KEYCODE: u32 = 88; // F12
pub const DEFAULT_SCREENSHOT_MODS: u32 = 0;
pub const DEFAULT_RECORD_KEYCODE: u32 = 87; // F11
pub const DEFAULT_RECORD_MODS: u32 = 0;

/// Parses a hotkey string like "Shift+Tab", "Ctrl+F12", "F11" into
/// (evdev_keycode, modifier_mask).
/// Returns (0, 0) on parse failure — callers should treat 0 as "use defaults".
pub fn parse_hotkey(s: &str) -> (u32, u32) {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return (0, 0);
    }
    let (mods, key) = parts.split_at(parts.len() - 1);

    let mut mod_mask = 0u32;
    for m in mods {
        mod_mask |= match m.to_lowercase().as_str() {
            "shift" => MOD_SHIFT,
            "ctrl" | "control" => MOD_CTRL,
            "alt" => MOD_ALT,
            "super" | "meta" | "win" => MOD_SUPER,
            _ => return (0, 0),
        };
    }

    match key_name_to_evdev(key[0]) {
        Some(kc) => (kc, mod_mask),
        None => (0, 0),
    }
}

/// Resolves a (keycode, mods) pair, falling back to defaults if keycode is 0.
pub fn resolve_defaults(keycode: u32, mods: u32, default_kc: u32, default_mods: u32) -> (u32, u32) {
    if keycode == 0 {
        (default_kc, default_mods)
    } else {
        (keycode, mods)
    }
}

/// Converts a key name string to an evdev keycode.
fn key_name_to_evdev(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "tab" => Some(15),
        "capslock" => Some(58),
        "return" | "enter" => Some(28),
        "escape" | "esc" => Some(1),
        "space" => Some(57),
        "backspace" => Some(14),
        "insert" | "ins" => Some(110),
        "delete" | "del" => Some(111),
        "home" => Some(102),
        "end" => Some(107),
        "pageup" | "pgup" => Some(104),
        "pagedown" | "pgdn" => Some(109),
        "up" => Some(103),
        "down" => Some(108),
        "left" => Some(105),
        "right" => Some(106),
        "a" => Some(30), "b" => Some(48), "c" => Some(46), "d" => Some(32),
        "e" => Some(18), "f" => Some(33), "g" => Some(34), "h" => Some(35),
        "i" => Some(23), "j" => Some(36), "k" => Some(37), "l" => Some(38),
        "m" => Some(50), "n" => Some(49), "o" => Some(24), "p" => Some(25),
        "q" => Some(16), "r" => Some(19), "s" => Some(31), "t" => Some(20),
        "u" => Some(22), "v" => Some(47), "w" => Some(17), "x" => Some(45),
        "y" => Some(21), "z" => Some(44),
        "0" => Some(11), "1" => Some(2), "2" => Some(3), "3" => Some(4),
        "4" => Some(5), "5" => Some(6), "6" => Some(7), "7" => Some(8),
        "8" => Some(9), "9" => Some(10),
        "f1" => Some(59), "f2" => Some(60), "f3" => Some(61), "f4" => Some(62),
        "f5" => Some(63), "f6" => Some(64), "f7" => Some(65), "f8" => Some(66),
        "f9" => Some(67), "f10" => Some(68), "f11" => Some(87), "f12" => Some(88),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shift_tab() {
        let (kc, mods) = parse_hotkey("Shift+Tab");
        assert_eq!(kc, 15);
        assert_eq!(mods, MOD_SHIFT);
    }

    #[test]
    fn test_parse_f12() {
        let (kc, mods) = parse_hotkey("F12");
        assert_eq!(kc, 88);
        assert_eq!(mods, 0);
    }

    #[test]
    fn test_parse_ctrl_shift_f11() {
        let (kc, mods) = parse_hotkey("Ctrl+Shift+F11");
        assert_eq!(kc, 87);
        assert_eq!(mods, MOD_CTRL | MOD_SHIFT);
    }

    #[test]
    fn test_parse_invalid() {
        let (kc, mods) = parse_hotkey("nonsense");
        assert_eq!(kc, 0);
        assert_eq!(mods, 0);
    }

    #[test]
    fn test_resolve_defaults() {
        let (kc, mods) = resolve_defaults(0, 0, DEFAULT_TOGGLE_KEYCODE, DEFAULT_TOGGLE_MODS);
        assert_eq!(kc, 15);
        assert_eq!(mods, MOD_SHIFT);
    }
}
