//! Keyboard and Numpad tab contents for the command picker: full QWERTY
//! rows with evdev codes, navigation cluster, numpad, and media/volume keys.

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

pub(crate) fn build_keyboard_page(
    content: &gtk4::Box,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    for row in KEY_ROWS {
        let strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        strip.set_halign(gtk4::Align::Center);
        for (label, keycode) in row {
            let button = gtk4::Button::with_label(&key_label(label));
            button.add_css_class(CSS_COMMAND_TILE);
            let on_pick = on_pick.clone();
            let window = window.clone();
            button.connect_clicked(move |_| {
                on_pick(OutputAction::Keyboard { keycode: *keycode });
                window.close();
            });
            strip.append(&button);
        }
        content.append(&strip);
    }
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
                on_pick(OutputAction::Keyboard { keycode });
                window.close();
            }
        });
    });
    flow.insert(&any, -1);
}

pub(crate) fn build_numpad_page(
    content: &gtk4::Box,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    let navigation = [
        (crate::tr!("Insert"), 110),
        (crate::tr!("Home"), 102),
        (crate::tr!("Page Up"), 104),
        (crate::tr!("Delete"), 111),
        (crate::tr!("End"), 107),
        (crate::tr!("Page Down"), 109),
        (crate::tr!("Up Arrow"), 103),
        (crate::tr!("Down Arrow"), 108),
        (crate::tr!("Left Arrow"), 105),
        (crate::tr!("Right Arrow"), 106),
    ];
    let flow = section(content, &crate::tr!("Navigation"));
    for (label, keycode) in navigation {
        tile(
            &flow,
            &label,
            Some(OutputAction::Keyboard { keycode }),
            None,
            on_pick,
            window,
        );
    }
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
            None,
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
            None,
            on_pick,
            window,
        );
    }
}
