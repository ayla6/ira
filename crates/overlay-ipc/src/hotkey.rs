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

/// Evdev codes of the modifier keys, for chord matching straight off
/// `/dev/input` event codes (this crate has no evdev dependency).
pub const EVDEV_LEFTSHIFT: u16 = 42;
pub const EVDEV_RIGHTSHIFT: u16 = 54;
pub const EVDEV_LEFTCTRL: u16 = 29;
pub const EVDEV_RIGHTCTRL: u16 = 97;
pub const EVDEV_LEFTALT: u16 = 56;
pub const EVDEV_RIGHTALT: u16 = 100;
pub const EVDEV_LEFTMETA: u16 = 125;

/// Modifier bits verifiable from evdev key state. Anything else in a chord
/// mask (Super, CapsLock, …) can never match here — those chords keep their
/// X11-grab path.
const TRACKED_KBD_MODS: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;

/// Pure keyboard-chord matcher over evdev `(code, value)` pairs: only the
/// press edge that newly completes the chord returns true; repeats and
/// releases never fire. Shared by the daemon's read-only snoop (opens the
/// overlay) and the host's evdev grab (closes it while consuming).
pub struct KbdChord {
    key: u32,
    mods: u32,
    pressed: std::collections::HashSet<u16>,
}

impl KbdChord {
    pub fn new(key: u32, mods: u32) -> Self {
        Self {
            key,
            mods,
            pressed: std::collections::HashSet::new(),
        }
    }

    pub fn feed(&mut self, code: u16, value: i32) -> bool {
        if value == 0 {
            self.pressed.remove(&code);
            return false;
        }
        if !self.pressed.insert(code) {
            return false;
        }
        u32::from(code) == self.key && Self::modifier_ok(&self.pressed, self.mods)
    }

    fn modifier_ok(pressed: &std::collections::HashSet<u16>, mask: u32) -> bool {
        if mask & !TRACKED_KBD_MODS != 0 {
            return false;
        }
        if mask & MOD_SHIFT != 0 && !Self::has_mod(pressed, EVDEV_LEFTSHIFT, EVDEV_RIGHTSHIFT) {
            return false;
        }
        if mask & MOD_CTRL != 0 && !Self::has_mod(pressed, EVDEV_LEFTCTRL, EVDEV_RIGHTCTRL) {
            return false;
        }
        if mask & MOD_ALT != 0 && !Self::has_mod(pressed, EVDEV_LEFTALT, EVDEV_RIGHTALT) {
            return false;
        }
        true
    }

    fn has_mod(pressed: &std::collections::HashSet<u16>, left: u16, right: u16) -> bool {
        pressed.contains(&left) || pressed.contains(&right)
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
            or_default(self.toggle_gamepad, DEFAULT_TOGGLE_GAMEPAD_HOTKEY),
            or_default(self.screenshot_gamepad, DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY),
            or_default(self.record_gamepad, DEFAULT_RECORD_GAMEPAD_HOTKEY),
        )
    }
}

/// Single-slot fallback: 0 means "use the built-in default".
fn or_default(value: u32, default: u32) -> u32 {
    if value == 0 { default } else { value }
}

/// Key name → evdev keycode. Aliases get their own row so lookup is a scan.
const KEY_TABLE: &[(&str, u32)] = &[
    ("tab", 15),
    ("capslock", 58),
    ("return", 28),
    ("enter", 28),
    ("escape", 1),
    ("esc", 1),
    ("space", 57),
    ("backspace", 14),
    ("insert", 110),
    ("ins", 110),
    ("delete", 111),
    ("del", 111),
    ("home", 102),
    ("end", 107),
    ("pageup", 104),
    ("pgup", 104),
    ("pagedown", 109),
    ("pgdn", 109),
    ("up", 103),
    ("down", 108),
    ("left", 105),
    ("right", 106),
    ("a", 30),
    ("b", 48),
    ("c", 46),
    ("d", 32),
    ("e", 18),
    ("f", 33),
    ("g", 34),
    ("h", 35),
    ("i", 23),
    ("j", 36),
    ("k", 37),
    ("l", 38),
    ("m", 50),
    ("n", 49),
    ("o", 24),
    ("p", 25),
    ("q", 16),
    ("r", 19),
    ("s", 31),
    ("t", 20),
    ("u", 22),
    ("v", 47),
    ("w", 17),
    ("x", 45),
    ("y", 21),
    ("z", 44),
    ("0", 11),
    ("1", 2),
    ("2", 3),
    ("3", 4),
    ("4", 5),
    ("5", 6),
    ("6", 7),
    ("7", 8),
    ("8", 9),
    ("9", 10),
    ("f1", 59),
    ("f2", 60),
    ("f3", 61),
    ("f4", 62),
    ("f5", 63),
    ("f6", 64),
    ("f7", 65),
    ("f8", 66),
    ("f9", 67),
    ("f10", 68),
    ("f11", 87),
    ("f12", 88),
];

/// Converts a key name string to an evdev keycode.
fn key_name_to_evdev(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    KEY_TABLE
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ShmHeader;

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

    #[test]
    fn test_header_hotkeys_apply_defaults_when_zero() {
        let hdr = ShmHeader::default();
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
        let hdr = ShmHeader {
            toggle_keysym: 20,
            toggle_mods: MOD_CTRL,
            screenshot_keysym: 61,
            record_keysym: 68,
            record_mods: MOD_ALT,
            ..ShmHeader::default()
        };
        assert_eq!(hdr.hotkeys(), (20, MOD_CTRL, 61, 0, 68, MOD_ALT));
    }

    #[test]
    fn test_header_hotkeys_slots_are_independent() {
        let hdr = ShmHeader {
            screenshot_keysym: 61,
            screenshot_mods: MOD_SUPER,
            ..ShmHeader::default()
        };
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
        let hdr = ShmHeader::default();
        assert_eq!(hdr.gamepad_hotkeys(), ShmHeader::default_gamepad_hotkeys());
    }

    #[test]
    fn test_header_gamepad_hotkeys_pass_masks_through() {
        let hdr = ShmHeader {
            toggle_gamepad: 1 << 10,
            screenshot_gamepad: 1 << 13 | 1 << 10,
            record_gamepad: 1 << 14 | 1 << 10,
            ..ShmHeader::default()
        };
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

    #[test]
    fn test_kbd_chord_fires_once_per_press() {
        // Arrange: default chord (Tab + Shift).
        let mut chord = KbdChord::new(15, MOD_SHIFT);
        // Act + Assert: lone Tab forwards…
        assert!(!chord.feed(15, 1));
        chord.feed(15, 0);
        // …Shift then Tab fires…
        assert!(!chord.feed(EVDEV_LEFTSHIFT, 1));
        assert!(chord.feed(15, 1));
        // …repeat while held does not re-fire…
        assert!(!chord.feed(15, 2));
        // …release forwards…
        assert!(!chord.feed(15, 0));
        // …and the next press fires again.
        assert!(chord.feed(15, 1));
    }

    #[test]
    fn test_kbd_chord_needs_its_modifiers() {
        // Arrange
        let mut chord = KbdChord::new(15, MOD_SHIFT);
        // Act + Assert: Tab with Ctrl held does not fire…
        assert!(!chord.feed(EVDEV_LEFTCTRL, 1));
        assert!(!chord.feed(15, 1));
        // …releasing Ctrl and pressing Shift does.
        chord.feed(EVDEV_LEFTCTRL, 0);
        chord.feed(15, 0);
        assert!(!chord.feed(EVDEV_RIGHTSHIFT, 1));
        assert!(chord.feed(15, 1));
    }

    #[test]
    fn test_kbd_chord_plain_key_without_modifiers() {
        // F12-style chord: no modifiers required, modifiers tolerated.
        let mut chord = KbdChord::new(88, 0);
        assert!(chord.feed(88, 1));
    }

    #[test]
    fn test_kbd_chord_untracked_modifiers_never_match() {
        // A Super+Tab chord cannot be verified from evdev state here, so it
        // must never fire (the X grab remains its path).
        let mut chord = KbdChord::new(15, MOD_SUPER);
        assert!(!chord.feed(EVDEV_LEFTMETA, 1));
        assert!(!chord.feed(15, 1));
    }
}
