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

use crate::config::{
    DEFAULT_RECORD_GAMEPAD_HOTKEY, DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY, DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
};

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

impl crate::protocol::ShmHeader {
    /// Navigation keys as X11 keycodes (evdev + [`X11_KEYCODE_OFFSET`]),
    /// in [Return, Up, Down, Left, Right] order.
    pub const NAV_KEYCODES_X11: [u32; 5] = [
        Self::NAV_RETURN_EVDEV + X11_KEYCODE_OFFSET,
        Self::NAV_UP_EVDEV + X11_KEYCODE_OFFSET,
        Self::NAV_DOWN_EVDEV + X11_KEYCODE_OFFSET,
        Self::NAV_LEFT_EVDEV + X11_KEYCODE_OFFSET,
        Self::NAV_RIGHT_EVDEV + X11_KEYCODE_OFFSET,
    ];

    /// Navigation keys as evdev keycodes (Wayland / /dev/input domain),
    /// in [Return, Up, Down, Left, Right] order.
    pub const NAV_KEYCODES_EVDEV: [u32; 5] = [
        Self::NAV_RETURN_EVDEV,
        Self::NAV_UP_EVDEV,
        Self::NAV_DOWN_EVDEV,
        Self::NAV_LEFT_EVDEV,
        Self::NAV_RIGHT_EVDEV,
    ];

    /// Return/Enter key, evdev keycode.
    pub const NAV_RETURN_EVDEV: u32 = 28;
    /// Up arrow, evdev keycode.
    pub const NAV_UP_EVDEV: u32 = 103;
    /// Down arrow, evdev keycode.
    pub const NAV_DOWN_EVDEV: u32 = 108;
    /// Left arrow, evdev keycode.
    pub const NAV_LEFT_EVDEV: u32 = 105;
    /// Right arrow, evdev keycode.
    pub const NAV_RIGHT_EVDEV: u32 = 106;

    /// Keyboard-hotkey fallback table for headers that carry no configured
    /// values (all fields zero). Order:
    /// (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
    pub fn default_hotkeys() -> (u32, u32, u32, u32, u32, u32) {
        (
            DEFAULT_TOGGLE_KEYCODE,
            DEFAULT_TOGGLE_MODS,
            DEFAULT_SCREENSHOT_KEYCODE,
            DEFAULT_SCREENSHOT_MODS,
            DEFAULT_RECORD_KEYCODE,
            DEFAULT_RECORD_MODS,
        )
    }

    /// Gamepad-mask fallback table for headers that carry no configured
    /// values: (toggle_mask, screenshot_mask, record_mask).
    pub fn default_gamepad_hotkeys() -> (u32, u32, u32) {
        (
            DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
            DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
            DEFAULT_RECORD_GAMEPAD_HOTKEY,
        )
    }

    /// Canonical keyboard-hotkey decode from SHM: applies
    /// [`resolve_defaults`] per slot, so a 0 keycode means "use the built-in
    /// default chord". Returns
    /// (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
    pub fn hotkeys(&self) -> (u32, u32, u32, u32, u32, u32) {
        let (tog_kc, tog_mods) = resolve_defaults(
            self.toggle_keysym,
            self.toggle_mods,
            DEFAULT_TOGGLE_KEYCODE,
            DEFAULT_TOGGLE_MODS,
        );
        let (ss_kc, ss_mods) = resolve_defaults(
            self.screenshot_keysym,
            self.screenshot_mods,
            DEFAULT_SCREENSHOT_KEYCODE,
            DEFAULT_SCREENSHOT_MODS,
        );
        let (rec_kc, rec_mods) = resolve_defaults(
            self.record_keysym,
            self.record_mods,
            DEFAULT_RECORD_KEYCODE,
            DEFAULT_RECORD_MODS,
        );
        (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods)
    }

    /// Canonical gamepad-hotkey decode from SHM: a 0 mask falls back to the
    /// matching default. Returns (toggle_mask, screenshot_mask, record_mask).
    pub fn gamepad_hotkeys(&self) -> (u32, u32, u32) {
        (
            nonzero_or_default(self.toggle_gamepad, DEFAULT_TOGGLE_GAMEPAD_HOTKEY),
            nonzero_or_default(self.screenshot_gamepad, DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY),
            nonzero_or_default(self.record_gamepad, DEFAULT_RECORD_GAMEPAD_HOTKEY),
        )
    }
}

/// Single-slot fallback used by [`ShmHeader::gamepad_hotkeys`], built on
/// [`resolve_defaults`] so all SHM hotkey decoding shares one pipeline.
fn nonzero_or_default(value: u32, default: u32) -> u32 {
    resolve_defaults(value, 0, default, 0).0
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
        "a" => Some(30),
        "b" => Some(48),
        "c" => Some(46),
        "d" => Some(32),
        "e" => Some(18),
        "f" => Some(33),
        "g" => Some(34),
        "h" => Some(35),
        "i" => Some(23),
        "j" => Some(36),
        "k" => Some(37),
        "l" => Some(38),
        "m" => Some(50),
        "n" => Some(49),
        "o" => Some(24),
        "p" => Some(25),
        "q" => Some(16),
        "r" => Some(19),
        "s" => Some(31),
        "t" => Some(20),
        "u" => Some(22),
        "v" => Some(47),
        "w" => Some(17),
        "x" => Some(45),
        "y" => Some(21),
        "z" => Some(44),
        "0" => Some(11),
        "1" => Some(2),
        "2" => Some(3),
        "3" => Some(4),
        "4" => Some(5),
        "5" => Some(6),
        "6" => Some(7),
        "7" => Some(8),
        "8" => Some(9),
        "9" => Some(10),
        "f1" => Some(59),
        "f2" => Some(60),
        "f3" => Some(61),
        "f4" => Some(62),
        "f5" => Some(63),
        "f6" => Some(64),
        "f7" => Some(65),
        "f8" => Some(66),
        "f9" => Some(67),
        "f10" => Some(68),
        "f11" => Some(87),
        "f12" => Some(88),
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

/// Zero-initialized header for decoder tests; shared with sibling-module
/// tests (e.g. shm.rs toggle/clock tests) via `crate::hotkey::zeroed_header`.
#[cfg(test)]
pub(crate) fn zeroed_header() -> crate::protocol::ShmHeader {
    use std::sync::atomic::AtomicU32;
    crate::protocol::ShmHeader {
        magic: 0,
        version: 0,
        game_db_id: 0,
        game_name: [0; 256],
        game_kind: [0; 32],
        cover_image_path: [0; 512],
        total_achievements: 0,
        unlocked_achievements: 0,
        playtime_seconds: 0,
        notification_write_index: AtomicU32::new(0),
        overlay_position: 0,
        video_encoder: 0,
        recording_quality: 0,
        recording_format: 0,
        toggle_keysym: 0,
        toggle_mods: 0,
        screenshot_keysym: 0,
        screenshot_mods: 0,
        record_keysym: 0,
        record_mods: 0,
        toggle_gamepad: 0,
        screenshot_gamepad: 0,
        record_gamepad: 0,
        overlay_visible: AtomicU32::new(0),
        last_toggle_ms: AtomicU32::new(0),
        replay_buffer_enabled: 0,
        replay_buffer_seconds: 0,
        direct_capture_ready: AtomicU32::new(0),
        padding: [0; 16],
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;
    use crate::protocol::ShmHeader;

    #[test]
    fn test_header_hotkeys_apply_defaults_when_zero() {
        let hdr = zeroed_header();
        assert_eq!(hdr.hotkeys(), ShmHeader::default_hotkeys());
        assert_eq!(
            hdr.hotkeys(),
            (
                DEFAULT_TOGGLE_KEYCODE,
                MOD_SHIFT,
                DEFAULT_SCREENSHOT_KEYCODE,
                0,
                DEFAULT_RECORD_KEYCODE,
                0
            )
        );
    }

    #[test]
    fn test_header_hotkeys_pass_configured_values_through() {
        let mut hdr = zeroed_header();
        hdr.toggle_keysym = 20;
        hdr.toggle_mods = MOD_CTRL;
        hdr.screenshot_keysym = 61;
        hdr.record_keysym = 68;
        hdr.record_mods = MOD_ALT;
        assert_eq!(hdr.hotkeys(), (20, MOD_CTRL, 61, 0, 68, MOD_ALT));
    }

    #[test]
    fn test_header_hotkeys_slots_are_independent() {
        let mut hdr = zeroed_header();
        hdr.screenshot_keysym = 61;
        hdr.screenshot_mods = MOD_SUPER;
        // Toggle and record slots stay on defaults.
        assert_eq!(hdr.hotkeys().0, DEFAULT_TOGGLE_KEYCODE);
        assert_eq!(hdr.hotkeys().1, DEFAULT_TOGGLE_MODS);
        assert_eq!(hdr.hotkeys().2, 61);
        assert_eq!(hdr.hotkeys().3, MOD_SUPER);
        assert_eq!(hdr.hotkeys().4, DEFAULT_RECORD_KEYCODE);
        assert_eq!(hdr.hotkeys().5, DEFAULT_RECORD_MODS);
    }

    #[test]
    fn test_header_gamepad_hotkeys_apply_defaults_when_zero() {
        let hdr = zeroed_header();
        assert_eq!(hdr.gamepad_hotkeys(), ShmHeader::default_gamepad_hotkeys());
    }

    #[test]
    fn test_header_gamepad_hotkeys_pass_masks_through() {
        let mut hdr = zeroed_header();
        hdr.toggle_gamepad = 1 << 10;
        hdr.screenshot_gamepad = 1 << 13 | 1 << 10;
        hdr.record_gamepad = 1 << 14 | 1 << 10;
        assert_eq!(
            hdr.gamepad_hotkeys(),
            (1 << 10, 1 << 13 | 1 << 10, 1 << 14 | 1 << 10)
        );
    }

    #[test]
    fn test_nav_keycode_arrays_follow_offset_relation() {
        for (x11, evdev) in ShmHeader::NAV_KEYCODES_X11
            .iter()
            .zip(ShmHeader::NAV_KEYCODES_EVDEV)
        {
            assert_eq!(*x11, evdev + X11_KEYCODE_OFFSET);
        }
        assert_eq!(ShmHeader::NAV_KEYCODES_EVDEV, [28, 103, 108, 105, 106]);
        assert_eq!(ShmHeader::NAV_KEYCODES_X11[0], 36);
        assert_eq!(ShmHeader::NAV_KEYCODES_X11[1], 111);
    }
}
