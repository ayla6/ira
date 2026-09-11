//! Keyboard and Numpad tab contents for the command picker: full QWERTY
//! rows with evdev codes, the navigation and arrow clusters, numpad, and
//! media/volume keys.

use super::css::{CSS_COMMAND_TILE, CSS_DIM_LABEL};
use super::input_output_picker::{section, tile};
use adw::prelude::*;
use ira_input::OutputAction;
use std::rc::Rc;

const KEY_ROWS: [&[(&str, u16)]; 6] = [
    &[
        ("Esc", 1),
        ("F1", 59),
        ("F2", 60),
        ("F3", 61),
        ("F4", 62),
        ("F5", 63),
        ("F6", 64),
        ("F7", 65),
        ("F8", 66),
        ("F9", 67),
        ("F10", 68),
        ("F11", 87),
        ("F12", 88),
    ],
    &[
        ("`", 41),
        ("1", 2),
        ("2", 3),
        ("3", 4),
        ("4", 5),
        ("5", 6),
        ("6", 7),
        ("7", 8),
        ("8", 9),
        ("9", 10),
        ("0", 11),
        ("-", 12),
        ("=", 13),
        ("Backspace", 14),
    ],
    &[
        ("Tab", 15),
        ("Q", 16),
        ("W", 17),
        ("E", 18),
        ("R", 19),
        ("T", 20),
        ("Y", 21),
        ("U", 22),
        ("I", 23),
        ("O", 24),
        ("P", 25),
        ("[", 26),
        ("]", 27),
        ("\\", 43),
    ],
    &[
        ("Caps", 58),
        ("A", 30),
        ("S", 31),
        ("D", 32),
        ("F", 33),
        ("G", 34),
        ("H", 35),
        ("J", 36),
        ("K", 37),
        ("L", 38),
        (";", 39),
        ("'", 40),
        ("Enter", 28),
    ],
    &[
        ("Shift", 42),
        ("Z", 44),
        ("X", 45),
        ("C", 46),
        ("V", 47),
        ("B", 48),
        ("N", 49),
        ("M", 50),
        (",", 51),
        (".", 52),
        ("/", 53),
        ("Shift", 54),
    ],
    &[
        ("Ctrl", 29),
        ("Win", 125),
        ("Alt", 56),
        ("Space", 57),
        ("Alt", 100),
        ("Ctrl", 97),
    ],
];

/// The navigation cluster, shared by the Keyboard and Numpad pages.
fn navigation_keys() -> Vec<(String, u16)> {
    vec![
        (crate::tr!("Insert"), 110),
        (crate::tr!("Home"), 102),
        (crate::tr!("Page Up"), 104),
        (crate::tr!("Delete"), 111),
        (crate::tr!("End"), 107),
        (crate::tr!("Page Down"), 109),
    ]
}

/// The arrow keys, shared by the Keyboard and Numpad pages.
fn arrow_keys() -> Vec<(String, u16)> {
    vec![
        (crate::tr!("Up Arrow"), 103),
        (crate::tr!("Down Arrow"), 108),
        (crate::tr!("Left Arrow"), 105),
        (crate::tr!("Right Arrow"), 106),
    ]
}

fn key_label(raw: &str) -> String {
    match raw {
        "Esc" => crate::tr!("Esc"),
        "Backspace" => crate::tr!("Backspace"),
        "Tab" => crate::tr!("Tab"),
        "Caps" => crate::tr!("Caps"),
        "Enter" => crate::tr!("Enter"),
        "Shift" => crate::tr!("Shift"),
        "Ctrl" => crate::tr!("Ctrl"),
        "Win" => crate::tr!("Win"),
        "Alt" => crate::tr!("Alt"),
        "Space" => crate::tr!("Space"),
        other => other.to_string(),
    }
}

fn char_at(s: &str, index: u16) -> String {
    s.chars()
        .nth(index as usize)
        .expect("keycode table index stays within its label row")
        .to_string()
}

/// The ten keyboard modifiers a hotkey capture treats as part of a chord
/// rather than as the captured key itself.
pub(crate) fn is_modifier_key(keyval: gtk4::gdk::Key) -> bool {
    matches!(
        keyval,
        gtk4::gdk::Key::Shift_L
            | gtk4::gdk::Key::Shift_R
            | gtk4::gdk::Key::Control_L
            | gtk4::gdk::Key::Control_R
            | gtk4::gdk::Key::Alt_L
            | gtk4::gdk::Key::Alt_R
            | gtk4::gdk::Key::Meta_L
            | gtk4::gdk::Key::Meta_R
            | gtk4::gdk::Key::Super_L
            | gtk4::gdk::Key::Super_R
    )
}

/// The human name of an evdev keycode, matching the picker's tile labels.
/// `None` for codes Ira cannot name; callers fall back to the number.
pub(crate) fn keycode_display_name(keycode: u16) -> Option<String> {
    match keycode {
        1 => Some(crate::tr!("Esc")),
        2..=10 => Some(char_at("123456789", keycode - 2)),
        11 => Some("0".to_string()),
        12 => Some("-".to_string()),
        13 => Some("=".to_string()),
        14 => Some(crate::tr!("Backspace")),
        15 => Some(crate::tr!("Tab")),
        16..=25 => Some(char_at("QWERTYUIOP", keycode - 16)),
        26 => Some("[".to_string()),
        27 => Some("]".to_string()),
        28 => Some(crate::tr!("Enter")),
        29 => Some(crate::tr!("Left Ctrl")),
        30..=38 => Some(char_at("ASDFGHJKL", keycode - 30)),
        39 => Some(";".to_string()),
        40 => Some("'".to_string()),
        41 => Some("`".to_string()),
        42 => Some(crate::tr!("Left Shift")),
        43 => Some("\\".to_string()),
        44..=50 => Some(char_at("ZXCVBNM", keycode - 44)),
        51 => Some(",".to_string()),
        52 => Some(".".to_string()),
        53 => Some("/".to_string()),
        54 => Some(crate::tr!("Right Shift")),
        55 => Some(crate::tr!("Num *")),
        56 => Some(crate::tr!("Left Alt")),
        57 => Some(crate::tr!("Space")),
        58 => Some(crate::tr!("Caps Lock")),
        59..=68 => Some(format!("F{}", keycode - 58)),
        69 => Some(crate::tr!("Num Lock")),
        70 => Some(crate::tr!("Scroll Lock")),
        71..=73 => Some(crate::tr!("Num {}").replacen("{}", &(keycode - 64).to_string(), 1)),
        74 => Some(crate::tr!("Num -")),
        75..=77 => Some(crate::tr!("Num {}").replacen("{}", &(keycode - 71).to_string(), 1)),
        78 => Some(crate::tr!("Num +")),
        79..=81 => Some(crate::tr!("Num {}").replacen("{}", &(keycode - 78).to_string(), 1)),
        82 => Some(crate::tr!("Num 0")),
        83 => Some(crate::tr!("Num .")),
        86 => Some("\\".to_string()),
        87 => Some("F11".to_string()),
        88 => Some("F12".to_string()),
        96 => Some(crate::tr!("Num Enter")),
        97 => Some(crate::tr!("Right Ctrl")),
        98 => Some(crate::tr!("Num /")),
        99 => Some(crate::tr!("Print Screen")),
        100 => Some(crate::tr!("Right Alt")),
        102 => Some(crate::tr!("Home")),
        103 => Some(crate::tr!("Up Arrow")),
        104 => Some(crate::tr!("Page Up")),
        105 => Some(crate::tr!("Left Arrow")),
        106 => Some(crate::tr!("Right Arrow")),
        107 => Some(crate::tr!("End")),
        108 => Some(crate::tr!("Down Arrow")),
        109 => Some(crate::tr!("Page Down")),
        110 => Some(crate::tr!("Insert")),
        111 => Some(crate::tr!("Delete")),
        113 => Some(crate::tr!("Mute")),
        114 => Some(crate::tr!("Volume Down")),
        115 => Some(crate::tr!("Volume Up")),
        117 => Some(crate::tr!("Num =")),
        119 => Some(crate::tr!("Pause")),
        125 => Some(crate::tr!("Left Win")),
        126 => Some(crate::tr!("Right Win")),
        127 => Some(crate::tr!("Menu")),
        140 => Some(crate::tr!("Calculator")),
        163 => Some(crate::tr!("Next Track")),
        164 => Some(crate::tr!("Play / Pause")),
        165 => Some(crate::tr!("Previous Track")),
        166 => Some(crate::tr!("Stop")),
        _ => None,
    }
}

/// One QWERTY row as a wrapping strip: a FlowBox capped at the row's own
/// length reflows to fewer tiles per line instead of clipping when the
/// dialog runs narrow.
fn key_strip(
    parent: &gtk4::Box,
    keys: &[(String, u16)],
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(Option<OutputAction>)>,
    window: &adw::Dialog,
) {
    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(4);
    flow.set_max_children_per_line(keys.len() as u32);
    flow.set_halign(gtk4::Align::Center);
    flow.set_valign(gtk4::Align::Start);
    for (label, keycode) in keys {
        tile(
            &flow,
            label,
            Some(OutputAction::Keyboard { keycode: *keycode }),
            current,
            on_pick,
            window,
        );
    }
    parent.append(&flow);
}

fn key_section(
    parent: &gtk4::Box,
    title: &str,
    keys: &[(String, u16)],
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(Option<OutputAction>)>,
    window: &adw::Dialog,
) {
    let flow = section(parent, title);
    for (label, keycode) in keys {
        tile(
            &flow,
            label,
            Some(OutputAction::Keyboard { keycode: *keycode }),
            current,
            on_pick,
            window,
        );
    }
}

pub(crate) fn build_keyboard_page(
    content: &gtk4::Box,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(Option<OutputAction>)>,
    window: &adw::Dialog,
) {
    for row in KEY_ROWS {
        let labels: Vec<(String, u16)> = row
            .iter()
            .map(|(label, keycode)| (key_label(label), *keycode))
            .collect();
        key_strip(content, &labels, current, on_pick, window);
    }
    key_section(content, &crate::tr!("Navigation"), &navigation_keys(), current, on_pick, window);
    key_section(content, &crate::tr!("Arrow Keys"), &arrow_keys(), current, on_pick, window);
    let hint = gtk4::Label::new(Some(&crate::tr!(
        "Use “Any key” below for keys not shown here."
    )));
    hint.set_xalign(0.0);
    hint.set_halign(gtk4::Align::Start);
    hint.add_css_class(CSS_DIM_LABEL);
    content.append(&hint);
    let flow = section(content, &crate::tr!("Other"));
    let any = gtk4::Button::with_label(&crate::tr!("Any key…"));
    any.add_css_class(CSS_COMMAND_TILE);
    let any_pick = on_pick.clone();
    let window = window.clone();
    any.connect_clicked(move |any_button| {
        let Some(parent) = any_button.root().and_downcast::<gtk4::Window>() else {
            return;
        };
        super::input_profile_output_capture::show_keyboard_output_capture(&parent, {
            let on_pick = any_pick.clone();
            let window = window.clone();
            move |keycode| {
                on_pick(Some(OutputAction::Keyboard { keycode }));
                window.close();
            }
        });
    });
    flow.insert(&any, -1);
}

pub(crate) fn build_numpad_page(
    content: &gtk4::Box,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(Option<OutputAction>)>,
    window: &adw::Dialog,
) {
    key_section(content, &crate::tr!("Navigation"), &navigation_keys(), current, on_pick, window);
    key_section(content, &crate::tr!("Arrow Keys"), &arrow_keys(), current, on_pick, window);
    let numpad = [
        ("Num Lock", 69),
        ("Num /", 98),
        ("Num *", 55),
        ("Num -", 74),
        ("Num 7", 71),
        ("Num 8", 72),
        ("Num 9", 73),
        ("Num +", 78),
        ("Num 4", 75),
        ("Num 5", 76),
        ("Num 6", 77),
        ("Num 1", 79),
        ("Num 2", 80),
        ("Num 3", 81),
        ("Num Enter", 96),
        ("Num 0", 82),
        ("Num .", 83),
    ];
    let flow = section(content, &crate::tr!("Numpad"));
    for (label, keycode) in numpad {
        tile(
            &flow,
            label,
            Some(OutputAction::Keyboard { keycode }),
            current,
            on_pick,
            window,
        );
    }
    let media = [
        (crate::tr!("Play / Pause"), 164),
        (crate::tr!("Stop"), 166),
        (crate::tr!("Next Track"), 163),
        (crate::tr!("Previous Track"), 165),
        (crate::tr!("Volume Up"), 115),
        (crate::tr!("Volume Down"), 114),
        (crate::tr!("Mute"), 113),
    ];
    let flow = section(content, &crate::tr!("Media & Volume"));
    for (label, keycode) in media {
        tile(
            &flow,
            &label,
            Some(OutputAction::Keyboard { keycode }),
            current,
            on_pick,
            window,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::keycode_display_name;

    #[test]
    fn test_keycode_display_name_covers_the_picker_keys() {
        assert_eq!(keycode_display_name(32).as_deref(), Some("D"));
        assert_eq!(keycode_display_name(11).as_deref(), Some("0"));
        assert_eq!(keycode_display_name(57).as_deref(), Some("Space"));
        assert_eq!(keycode_display_name(103).as_deref(), Some("Up Arrow"));
        assert_eq!(keycode_display_name(71).as_deref(), Some("Num 7"));
        assert_eq!(keycode_display_name(96).as_deref(), Some("Num Enter"));
        assert_eq!(keycode_display_name(125).as_deref(), Some("Left Win"));
        assert_eq!(keycode_display_name(88).as_deref(), Some("F12"));
    }

    #[test]
    fn test_keycode_display_name_none_for_unknown_codes() {
        assert_eq!(keycode_display_name(250), None);
        assert_eq!(keycode_display_name(0), None);
    }
}
