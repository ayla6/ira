//! Steam-style command picker: one modal window, pill tab bar (Gamepad /
//! Mouse / Keyboard / Numpad / Action Sets), large tappable tiles. Replaces
//! the flat output dropdowns and mirrors Steam Input's output selector.

use super::css::{CSS_COMMAND_TILE, CSS_COMMAND_TILE_ACTIVE};
use super::input_output_keys::{build_keyboard_page, build_numpad_page};
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

pub(crate) fn show_output_picker(
    parent: &impl IsA<gtk4::Widget>,
    input_title: &str,
    scope: &OutputPickerScope,
    current: Option<&OutputAction>,
    on_pick: impl Fn(OutputAction) + 'static,
) {
    let on_pick: Rc<dyn Fn(OutputAction)> = Rc::new(on_pick);
    let window = adw::Dialog::new();
    window.set_content_width(780);
    window.set_content_height(560);
    window.set_title(input_title);

    let stack = adw::ViewStack::new();
    let pages = [
        ("gamepad", crate::tr!("Gamepad"), "input-gaming-symbolic"),
        ("mouse", crate::tr!("Mouse"), "input-mouse-symbolic"),
        (
            "keyboard",
            crate::tr!("Keyboard"),
            "input-keyboard-symbolic",
        ),
        (
            "numpad",
            crate::tr!("Numpad"),
            "gnome-accessibility-keyboard-symbolic",
        ),
        ("sets", crate::tr!("Action Sets"), "view-grid-symbolic"),
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
    window.set_child(Some(&toolbar));
    window.present(Some(parent));
}

pub(crate) fn section(parent: &gtk4::Box, title: &str) -> gtk4::FlowBox {
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

pub(crate) fn tile(
    flow: &gtk4::FlowBox,
    label: &str,
    action: Option<OutputAction>,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Dialog,
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
    window: &adw::Dialog,
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
    window: &adw::Dialog,
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

fn build_sets_page(
    content: &gtk4::Box,
    scope: &OutputPickerScope,
    current: Option<&OutputAction>,
    on_pick: &Rc<dyn Fn(OutputAction)>,
    window: &adw::Dialog,
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
