//! Steam-style command picker: one modal window, pill tab bar (Gamepad /
//! Mouse / Keyboard / Numpad / Action Sets), large tappable tiles. Replaces
//! the flat output dropdowns and mirrors Steam Input's output selector.

use super::css::{CSS_COMMAND_TILE, CSS_COMMAND_TILE_ACTIVE};
use adw::prelude::*;
use ira_input::{
    ChordMode, GamepadAxis, GamepadButton, MouseAxis, MouseButton, OutputAction,
    VirtualGamepadBackend,
};
use std::rc::Rc;

/// What the picker offers beyond plain device outputs: the profile's action
/// set and layer names, so the Action Sets tab can target them by index.
pub(crate) struct OutputPickerScope {
    pub backend: VirtualGamepadBackend,
    pub set_names: Vec<String>,
    pub layer_names: Vec<String>,
}

impl OutputPickerScope {
    pub(crate) fn flat(backend: VirtualGamepadBackend) -> Self {
        Self {
            backend,
            set_names: Vec::new(),
            layer_names: Vec::new(),
        }
    }
}

pub(crate) fn show_output_picker(
    parent: &gtk4::Window,
    input_title: &str,
    scope: &OutputPickerScope,
    current: Option<&OutputAction>,
    on_pick: impl Fn(OutputAction) + 'static,
) {
    let on_pick: Rc<dyn Fn(OutputAction)> = Rc::new(on_pick);
    let window = adw::Window::new();
    window.set_transient_for(Some(parent));
    window.set_modal(true);
    window.set_default_size(780, 560);
    window.set_title(Some(input_title));

    let stack = adw::ViewStack::new();
    let pages = [
        ("gamepad", crate::tr!("Gamepad"), "input-gaming-symbolic"),
        ("mouse", crate::tr!("Mouse"), "input-mouse-symbolic"),
        (
            "keyboard",
            crate::tr!("Keyboard"),
            "input-keyboard-symbolic",
        ),
        ("numpad", crate::tr!("Numpad"), "gnome-accessibility-keyboard-symbolic"),
        (
            "sets",
            crate::tr!("Action Sets"),
            "view-grid-symbolic",
        ),
    ];
    for (name, title, icon) in pages {
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);
        match name {
            "gamepad" => build_gamepad_page(&content, scope, current, &on_pick, &window),
            "mouse" => build_mouse_page(&content, current, &on_pick, &window),
            "keyboard" => build_keyboard_page(&content, &on_pick, &window),
            "numpad" => build_numpad_page(&content, &on_pick, &window),
            _ => build_sets_page(&content, scope, current, &on_pick, &window),
        }
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
        scroll.set_child(Some(&content));
        stack.add_titled(&scroll, Some(name), &title);
        stack.page(&scroll).set_icon_name(Some(icon));
    }

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    window.set_content(Some(&toolbar));
    window.present();
}

fn section(parent: &gtk4::Box, title: &str) -> gtk4::FlowBox {
    let label = gtk4::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_halign(gtk4::Align::Start);
    label.add_css_class(super::css::CSS_DIM_LABEL);
    parent.append(&label);
    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(6);
    flow.set_homogeneous(true);
    parent.append(&flow);
    flow
}

fn tile(
    flow: &gtk4::FlowBox,
    label: &str,
    action: Option<OutputAction>,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    let button = gtk4::Button::new();
    button.add_css_class(CSS_COMMAND_TILE);
    let text = gtk4::Label::new(Some(label));
    text.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    text.set_max_width_chars(18);
    button.set_child(Some(&text));
    match action {
        Some(action) => {
            if Some(&action) == current {
                button.add_css_class(CSS_COMMAND_TILE_ACTIVE);
            }
            let on_pick = on_pick.clone();
            let window = window.clone();
            button.connect_clicked(move |_| {
                on_pick(action.clone());
                window.close();
            });
        }
        None => {
            button.set_sensitive(false);
            button.set_tooltip_text(Some(&crate::tr!("Not supported yet")));
        }
    }
    flow.insert(&button, -1);
}

fn build_gamepad_page(
    content: &gtk4::Box,
    scope: &OutputPickerScope,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    let buttons = [
        GamepadButton::A,
        GamepadButton::B,
        GamepadButton::X,
        GamepadButton::Y,
    ];
    let dpad = [
        GamepadButton::DpadUp,
        GamepadButton::DpadRight,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
    ];
    let shoulders = [
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
    ];
    let system = [
        GamepadButton::Back,
        GamepadButton::Start,
        GamepadButton::Guide,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
    ];
    let flow = section(content, &crate::tr!("Face Buttons"));
    for button in buttons {
        tile(
            &flow,
            &super::input_profile_options::button_label(button),
            Some(OutputAction::GamepadButton(button)),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("D-pad"));
    for button in dpad {
        tile(
            &flow,
            &super::input_profile_options::button_label(button),
            Some(OutputAction::GamepadButton(button)),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("Bumpers & Triggers"));
    for button in shoulders {
        tile(
            &flow,
            &super::input_profile_options::button_label(button),
            Some(OutputAction::GamepadButton(button)),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("Sticks & System"));
    for button in system {
        tile(
            &flow,
            &super::input_profile_options::button_label(button),
            Some(OutputAction::GamepadButton(button)),
            current,
            on_pick,
            window,
        );
    }
    if scope.backend == VirtualGamepadBackend::DirectInput {
        let flow = section(content, &crate::tr!("Paddles"));
        for number in 1..=8 {
            let button = match number {
                1 => GamepadButton::Paddle1,
                2 => GamepadButton::Paddle2,
                3 => GamepadButton::Paddle3,
                4 => GamepadButton::Paddle4,
                5 => GamepadButton::Paddle5,
                6 => GamepadButton::Paddle6,
                7 => GamepadButton::Paddle7,
                _ => GamepadButton::Paddle8,
            };
            tile(
                &flow,
                &crate::tr!("Paddle {number}").replace("{number}", &number.to_string()),
                Some(OutputAction::GamepadButton(button)),
                current,
                on_pick,
                window,
            );
        }
    }
    let axes = [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ];
    let flow = section(content, &crate::tr!("Analog Axes"));
    for axis in axes {
        if scope.backend == VirtualGamepadBackend::SwitchPro
            && matches!(axis, GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
        {
            continue;
        }
        tile(
            &flow,
            &super::input_profile_options::axis_label(axis),
            Some(OutputAction::GamepadAxis(axis)),
            current,
            on_pick,
            window,
        );
    }
}

fn build_mouse_page(
    content: &gtk4::Box,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    let clicks = [
        (MouseButton::Left, crate::tr!("Left Mouse Click")),
        (MouseButton::Right, crate::tr!("Right Mouse Click")),
        (MouseButton::Middle, crate::tr!("Middle Mouse Click")),
        (MouseButton::Side, crate::tr!("Mouse 4 Click")),
        (MouseButton::Extra, crate::tr!("Mouse 5 Click")),
    ];
    let flow = section(content, &crate::tr!("Buttons"));
    for (button, label) in clicks {
        tile(
            &flow,
            &label,
            Some(OutputAction::MouseButton(button)),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("Scroll Wheel"));
    for (label, axis, amount) in [
        (crate::tr!("Scroll Wheel Up"), MouseAxis::Wheel, 1),
        (crate::tr!("Scroll Wheel Down"), MouseAxis::Wheel, -1),
        (crate::tr!("Scroll Wheel Right"), MouseAxis::WheelX, 1),
        (crate::tr!("Scroll Wheel Left"), MouseAxis::WheelX, -1),
    ] {
        tile(
            &flow,
            &label,
            Some(OutputAction::WheelClick { axis, amount }),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("Motion"));
    for (label, axis) in [
        (crate::tr!("Mouse Move X"), MouseAxis::X),
        (crate::tr!("Mouse Move Y"), MouseAxis::Y),
    ] {
        tile(
            &flow,
            &label,
            Some(OutputAction::MouseAxis(axis)),
            current,
            on_pick,
            window,
        );
    }
    let flow = section(content, &crate::tr!("Position"));
    for label in [crate::tr!("Move to Position"), crate::tr!("Move by Amount")] {
        tile(&flow, &label, None, current, on_pick, window);
    }
}

/// Keyboard rows mirroring Steam's keyboard tab. Labels are US-ASCII key
/// caps; long-press modifier combos are not expressible in our model yet.
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

fn build_keyboard_page(
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
    hint.add_css_class(super::css::CSS_DIM_LABEL);
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

fn build_numpad_page(
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

fn build_sets_page(
    content: &gtk4::Box,
    scope: &OutputPickerScope,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Window,
) {
    if scope.set_names.len() < 2 && scope.layer_names.is_empty() {
        let note = gtk4::Label::new(Some(&crate::tr!(
            "These commands require your layout to contain multiple action sets."
        )));
        note.set_wrap(true);
        note.set_xalign(0.0);
        note.add_css_class(super::css::CSS_DIM_LABEL);
        content.append(&note);
        return;
    }
    let flow = section(content, &crate::tr!("Change Action Set"));
    for (index, name) in scope.set_names.iter().enumerate() {
        tile(
            &flow,
            &crate::tr!("Switch to {name}").replace("{name}", name),
            Some(OutputAction::SwitchActionSet(index)),
            current,
            on_pick,
            window,
        );
    }
    if !scope.layer_names.is_empty() {
        let flow = section(content, &crate::tr!("Action Set Layers"));
        for (index, name) in scope.layer_names.iter().enumerate() {
            tile(
                &flow,
                &crate::tr!("Hold {name}").replace("{name}", name),
                Some(OutputAction::EnableLayer {
                    layer: index,
                    mode: ChordMode::Hold,
                }),
                current,
                on_pick,
                window,
            );
            tile(
                &flow,
                &crate::tr!("Toggle {name}").replace("{name}", name),
                Some(OutputAction::EnableLayer {
                    layer: index,
                    mode: ChordMode::Toggle,
                }),
                current,
                on_pick,
                window,
            );
        }
    }
}
