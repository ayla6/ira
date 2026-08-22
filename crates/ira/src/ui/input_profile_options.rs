use ira_input::{
    Activation, AxisDirection, DeviceInfo, GamepadAxis, GamepadButton, InputSource, MouseAxis,
    MouseButton, OutputAction, VirtualGamepadBackend,
};

pub(super) fn source_options_for_device(
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
) -> Vec<(InputSource, String)> {
    let mut options = gamepad_buttons()
        .into_iter()
        .filter(|button| {
            device.map_or(
                backend == VirtualGamepadBackend::DirectInput || !button.is_paddle(),
                |device| device.supported_buttons.contains(button),
            )
        })
        .map(|button| {
            (
                InputSource::Button(button),
                button_label_for_device(button, device),
            )
        })
        .collect::<Vec<_>>();
    options.extend(
        gamepad_axes()
            .into_iter()
            .map(|axis| (InputSource::Axis(axis), axis_label(axis))),
    );
    options.extend(axis_directions());
    options
}

fn button_label_for_device(button: GamepadButton, device: Option<&DeviceInfo>) -> String {
    if device.is_some_and(|device| device.family() == ira_input::ControllerFamily::EightBitDo) {
        match button {
            GamepadButton::Back => return crate::tr!("Minus"),
            GamepadButton::Start => return crate::tr!("Plus"),
            GamepadButton::Guide => return crate::tr!("Home"),
            GamepadButton::Paddle1 => return "R4".to_string(),
            GamepadButton::Paddle2 => return "L4".to_string(),
            GamepadButton::Paddle3 => return "PR".to_string(),
            GamepadButton::Paddle4 => return "PL".to_string(),
            _ => {}
        }
    }
    button_label(button)
}

fn gamepad_buttons() -> [GamepadButton; 25] {
    [
        GamepadButton::A,
        GamepadButton::B,
        GamepadButton::X,
        GamepadButton::Y,
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::Back,
        GamepadButton::Start,
        GamepadButton::Guide,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
        GamepadButton::Paddle2,
        GamepadButton::Paddle1,
        GamepadButton::Paddle4,
        GamepadButton::Paddle3,
        GamepadButton::Paddle5,
        GamepadButton::Paddle6,
        GamepadButton::Paddle7,
        GamepadButton::Paddle8,
    ]
}

fn gamepad_axes() -> [GamepadAxis; 6] {
    [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ]
}

pub(super) fn button_label(button: GamepadButton) -> String {
    match button {
        GamepadButton::A => crate::tr!("A Button"),
        GamepadButton::B => crate::tr!("B Button"),
        GamepadButton::X => crate::tr!("X Button"),
        GamepadButton::Y => crate::tr!("Y Button"),
        GamepadButton::LeftShoulder => crate::tr!("Left Bumper"),
        GamepadButton::RightShoulder => crate::tr!("Right Bumper"),
        GamepadButton::LeftTrigger => crate::tr!("Left Trigger Button"),
        GamepadButton::RightTrigger => crate::tr!("Right Trigger Button"),
        GamepadButton::Back => crate::tr!("Select"),
        GamepadButton::Start => crate::tr!("Start"),
        GamepadButton::Guide => crate::tr!("Guide"),
        GamepadButton::LeftStick => crate::tr!("Left Stick Click"),
        GamepadButton::RightStick => crate::tr!("Right Stick Click"),
        GamepadButton::DpadUp => crate::tr!("D-pad Up"),
        GamepadButton::DpadDown => crate::tr!("D-pad Down"),
        GamepadButton::DpadLeft => crate::tr!("D-pad Left"),
        GamepadButton::DpadRight => crate::tr!("D-pad Right"),
        GamepadButton::Paddle1 => crate::tr!("Paddle 1"),
        GamepadButton::Paddle2 => crate::tr!("Paddle 2"),
        GamepadButton::Paddle3 => crate::tr!("Paddle 3"),
        GamepadButton::Paddle4 => crate::tr!("Paddle 4"),
        GamepadButton::Paddle5 => crate::tr!("Paddle 5"),
        GamepadButton::Paddle6 => crate::tr!("Paddle 6"),
        GamepadButton::Paddle7 => crate::tr!("Paddle 7"),
        GamepadButton::Paddle8 => crate::tr!("Paddle 8"),
    }
}

pub(super) fn axis_label(axis: GamepadAxis) -> String {
    match axis {
        GamepadAxis::LeftX => crate::tr!("Left Stick X"),
        GamepadAxis::LeftY => crate::tr!("Left Stick Y"),
        GamepadAxis::RightX => crate::tr!("Right Stick X"),
        GamepadAxis::RightY => crate::tr!("Right Stick Y"),
        GamepadAxis::LeftTrigger => crate::tr!("Left Trigger"),
        GamepadAxis::RightTrigger => crate::tr!("Right Trigger"),
    }
}

fn axis_directions() -> Vec<(InputSource, String)> {
    gamepad_axes()
        .into_iter()
        .filter(|axis| {
            matches!(
                axis,
                GamepadAxis::LeftX | GamepadAxis::LeftY | GamepadAxis::RightX | GamepadAxis::RightY
            )
        })
        .flat_map(|axis| {
            [AxisDirection::Negative, AxisDirection::Positive]
                .into_iter()
                .map(move |direction| {
                    let sign = match direction {
                        AxisDirection::Negative => "-",
                        AxisDirection::Positive => "+",
                    };
                    (
                        InputSource::AxisDirection { axis, direction },
                        crate::tr!("{} ({})")
                            .replacen("{}", &axis_label(axis), 1)
                            .replacen("{}", sign, 1),
                    )
                })
        })
        .collect()
}

pub(super) fn activator_index(activation: &Activation, options: &[(InputSource, String)]) -> u32 {
    let source_index = |source| {
        options
            .iter()
            .position(|(candidate, _)| *candidate == source)
            .unwrap_or(0) as u32
    };
    match activation {
        Activation::Hold(source)
        | Activation::Toggle(source)
        | Activation::DisableWhile(source) => source_index(*source),
        Activation::Chord { sources, .. } => {
            sources.first().copied().map(source_index).unwrap_or(0)
        }
        Activation::Always => 0,
    }
}

pub(super) fn activation_labels() -> Vec<String> {
    [
        crate::tr!("Always"),
        crate::tr!("Hold"),
        crate::tr!("Toggle"),
        crate::tr!("Disable while held"),
        crate::tr!("Chord"),
    ]
    .into_iter()
    .collect()
}

pub(super) fn activation_index(activation: &Activation) -> u32 {
    match activation {
        Activation::Hold(_) => 1,
        Activation::Toggle(_) => 2,
        Activation::DisableWhile(_) => 3,
        Activation::Chord { .. } => 4,
        Activation::Always => 0,
    }
}

pub(super) fn output_display_label(output: &OutputAction) -> String {
    match output {
        OutputAction::GamepadButton(button) => button_label(*button),
        OutputAction::GamepadAxis(axis) => axis_label(*axis),
        OutputAction::Keyboard { keycode } => {
            crate::tr!("Keyboard key {}").replacen("{}", &keycode.to_string(), 1)
        }
        OutputAction::MouseButton(button) => match button {
            MouseButton::Left => crate::tr!("Mouse Left"),
            MouseButton::Right => crate::tr!("Mouse Right"),
            MouseButton::Middle => crate::tr!("Mouse Middle"),
            MouseButton::Side => crate::tr!("Mouse Side"),
            MouseButton::Extra => crate::tr!("Mouse Extra"),
        },
        OutputAction::MouseAxis(axis) => match axis {
            MouseAxis::X => crate::tr!("Mouse X"),
            MouseAxis::Y => crate::tr!("Mouse Y"),
            MouseAxis::Wheel => crate::tr!("Mouse Wheel"),
            MouseAxis::WheelX => crate::tr!("Mouse Wheel (Horizontal)"),
        },
        OutputAction::WheelClick { axis, amount } => match (axis, *amount < 0) {
            (MouseAxis::Wheel, false) => crate::tr!("Scroll Wheel Up"),
            (MouseAxis::Wheel, true) => crate::tr!("Scroll Wheel Down"),
            (MouseAxis::WheelX, false) => crate::tr!("Scroll Wheel Right"),
            (MouseAxis::WheelX, true) => crate::tr!("Scroll Wheel Left"),
            _ => crate::tr!("Scroll Wheel"),
        },
        OutputAction::SwitchActionSet(index) => {
            crate::tr!("Switch to action set {index}").replace("{index}", &index.to_string())
        }
        OutputAction::EnableLayer { layer, .. } => crate::tr!("Toggle action layer {layer}")
            .replace("{layer}", &layer.to_string()),
        OutputAction::ModeShiftActivate { .. } => crate::tr!("Activate mode shift"),
    }
}

#[cfg(test)]
mod tests {
    use super::{activator_index, output_display_label, source_options_for_device};
    use ira_input::{
        Activation, DeviceInfo, GamepadAxis, GamepadButton, InputSource, MouseAxis, MouseButton,
        OutputAction, VirtualGamepadBackend,
    };
    use std::path::PathBuf;

    #[test]
    fn test_source_options_without_device_omit_paddles() {
        let options = source_options_for_device(None, VirtualGamepadBackend::XInput);
        assert!(options.contains(&(
            InputSource::Button(GamepadButton::A),
            "A Button".to_string()
        )));
        assert!(!options
            .iter()
            .any(|(source, _)| *source == InputSource::Button(GamepadButton::Paddle1)));
    }

    #[test]
    fn test_source_options_use_reported_device_buttons() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Test controller".to_string(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: vec![GamepadButton::A, GamepadButton::Paddle1],
        };
        let options = source_options_for_device(Some(&device), VirtualGamepadBackend::DirectInput);
        assert!(options
            .iter()
            .any(|(source, _)| *source == InputSource::Button(GamepadButton::Paddle1)));
        assert!(!options
            .iter()
            .any(|(source, _)| *source == InputSource::Button(GamepadButton::Paddle2)));
    }

    #[test]
    fn test_activator_index_keeps_paddle_selection() {
        let options = vec![
            (InputSource::Button(GamepadButton::A), "A".to_string()),
            (
                InputSource::Button(GamepadButton::Paddle4),
                "P4".to_string(),
            ),
        ];
        assert_eq!(
            activator_index(
                &Activation::DisableWhile(InputSource::Button(GamepadButton::Paddle4)),
                &options,
            ),
            1
        );
    }

    #[test]
    fn test_trigger_labels_distinguish_axis_and_button_bindings() {
        let sources = source_options_for_device(None, VirtualGamepadBackend::XInput);
        assert!(sources.contains(&(
            InputSource::Button(GamepadButton::LeftTrigger),
            "Left Trigger Button".to_string()
        )));
        assert!(sources.contains(&(
            InputSource::Axis(GamepadAxis::LeftTrigger),
            "Left Trigger".to_string()
        )));
    }

    #[test]
    fn test_output_display_label_names_keyboard_and_mouse_actions() {
        assert_eq!(
            output_display_label(&OutputAction::Keyboard { keycode: 57 }),
            "Keyboard key 57"
        );
        assert_eq!(
            output_display_label(&OutputAction::MouseButton(MouseButton::Side)),
            "Mouse Side"
        );
        assert_eq!(
            output_display_label(&OutputAction::WheelClick {
                axis: MouseAxis::Wheel,
                amount: -1
            }),
            "Scroll Wheel Down"
        );
    }

    #[test]
    fn test_eightbitdo_extended_labels_follow_steam_order() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "8BitDo Ultimate".to_string(),
            vendor: 0x2dc8,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: vec![
                GamepadButton::Paddle1,
                GamepadButton::Paddle2,
                GamepadButton::Paddle3,
                GamepadButton::Paddle4,
            ],
        };
        let labels = source_options_for_device(Some(&device), VirtualGamepadBackend::DirectInput);
        let extended = labels
            .into_iter()
            .filter_map(|(source, label)| match source {
                InputSource::Button(button) if button.is_paddle() => Some(label),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(extended, vec!["L4", "R4", "PL", "PR"]);
    }
}
