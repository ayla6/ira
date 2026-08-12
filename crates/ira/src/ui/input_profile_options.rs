use ira_input::{
    Activation, AxisDirection, DeviceInfo, GamepadAxis, GamepadButton, GyroAxis, GyroMode,
    InputSource, MouseAxis, MouseButton, OutputAction, RecenterMode,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum OutputOption {
    Action(OutputAction),
    CaptureKeyboard,
    CaptureMouseButton,
}

pub(super) fn source_options_for_device(device: Option<&DeviceInfo>) -> Vec<(InputSource, String)> {
    let mut options = gamepad_buttons()
        .into_iter()
        .filter(|button| {
            device.map_or(!button.is_paddle(), |device| {
                device.supported_buttons.contains(button)
            })
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
    options.extend(
        [
            (GyroAxis::X, "Gyro X (Pitch)"),
            (GyroAxis::Y, "Gyro Y (Yaw)"),
            (GyroAxis::Z, "Gyro Z (Roll)"),
        ]
        .into_iter()
        .map(|(axis, label)| (InputSource::Gyro(axis), label.to_string())),
    );
    options
}

fn button_label_for_device(button: GamepadButton, device: Option<&DeviceInfo>) -> String {
    if device.is_some_and(|device| device.family() == ira_input::ControllerFamily::EightBitDo) {
        match button {
            GamepadButton::Back => return "Minus".to_string(),
            GamepadButton::Start => return "Plus".to_string(),
            GamepadButton::Guide => return "Home".to_string(),
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
        GamepadButton::A => "A Button".to_string(),
        GamepadButton::B => "B Button".to_string(),
        GamepadButton::X => "X Button".to_string(),
        GamepadButton::Y => "Y Button".to_string(),
        GamepadButton::LeftShoulder => "Left Bumper".to_string(),
        GamepadButton::RightShoulder => "Right Bumper".to_string(),
        GamepadButton::LeftTrigger => "Left Trigger (full)".to_string(),
        GamepadButton::RightTrigger => "Right Trigger (full)".to_string(),
        GamepadButton::Back => "Select".to_string(),
        GamepadButton::Start => "Start".to_string(),
        GamepadButton::Guide => "Guide".to_string(),
        GamepadButton::LeftStick => "Left Stick Click".to_string(),
        GamepadButton::RightStick => "Right Stick Click".to_string(),
        GamepadButton::DpadUp => "D-pad Up".to_string(),
        GamepadButton::DpadDown => "D-pad Down".to_string(),
        GamepadButton::DpadLeft => "D-pad Left".to_string(),
        GamepadButton::DpadRight => "D-pad Right".to_string(),
        GamepadButton::Paddle1 => "Paddle 1".to_string(),
        GamepadButton::Paddle2 => "Paddle 2".to_string(),
        GamepadButton::Paddle3 => "Paddle 3".to_string(),
        GamepadButton::Paddle4 => "Paddle 4".to_string(),
        GamepadButton::Paddle5 => "Paddle 5".to_string(),
        GamepadButton::Paddle6 => "Paddle 6".to_string(),
        GamepadButton::Paddle7 => "Paddle 7".to_string(),
        GamepadButton::Paddle8 => "Paddle 8".to_string(),
    }
}

pub(super) fn axis_label(axis: GamepadAxis) -> String {
    match axis {
        GamepadAxis::LeftX => "Left Stick X".to_string(),
        GamepadAxis::LeftY => "Left Stick Y".to_string(),
        GamepadAxis::RightX => "Right Stick X".to_string(),
        GamepadAxis::RightY => "Right Stick Y".to_string(),
        GamepadAxis::LeftTrigger => "Left Trigger (soft)".to_string(),
        GamepadAxis::RightTrigger => "Right Trigger (soft)".to_string(),
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
                        format!("{} ({sign})", axis_label(axis)),
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
    ["Always", "Hold", "Toggle", "Disable while held", "Chord"]
        .into_iter()
        .map(str::to_string)
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

pub(super) fn recenter_index(recenter: RecenterMode) -> u32 {
    match recenter {
        RecenterMode::OnEnable => 1,
        RecenterMode::OnDisable => 2,
        RecenterMode::OnEnableOrDisable => 3,
        RecenterMode::Never => 0,
    }
}

pub(super) fn gyro_mode_labels() -> Vec<String> {
    ["Rate", "Joystick position"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(super) fn gyro_mode_index(mode: GyroMode) -> u32 {
    match mode {
        GyroMode::Rate => 0,
        GyroMode::HoldLast => 1,
    }
}

fn output_buttons() -> [GamepadButton; 17] {
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
    ]
}

pub(super) fn output_options() -> Vec<OutputOption> {
    let mut options = output_buttons()
        .into_iter()
        .map(|button| OutputOption::Action(OutputAction::GamepadButton(button)))
        .collect::<Vec<_>>();
    options.extend(
        gamepad_axes()
            .into_iter()
            .map(|axis| OutputOption::Action(OutputAction::GamepadAxis(axis))),
    );
    options.extend(
        [MouseAxis::X, MouseAxis::Y, MouseAxis::Wheel]
            .into_iter()
            .map(|axis| OutputOption::Action(OutputAction::MouseAxis(axis))),
    );
    options.extend([
        OutputOption::CaptureKeyboard,
        OutputOption::CaptureMouseButton,
    ]);
    options
}

pub(super) fn output_labels() -> Vec<String> {
    output_options().iter().map(output_option_label).collect()
}

pub(super) fn output_index(output: &OutputAction) -> u32 {
    output_options()
        .iter()
        .position(|option| output_option_matches(option, output))
        .unwrap_or(0) as u32
}

pub(super) fn output_option(index: u32) -> Option<OutputOption> {
    output_options().get(index as usize).cloned()
}

pub(super) fn output_display_label(output: &OutputAction) -> String {
    match output {
        OutputAction::GamepadButton(button) => button_label(*button),
        OutputAction::GamepadAxis(axis) => axis_label(*axis),
        OutputAction::Keyboard { keycode } => format!("Keyboard key {keycode}"),
        OutputAction::MouseButton(button) => match button {
            MouseButton::Left => "Mouse Left".to_string(),
            MouseButton::Right => "Mouse Right".to_string(),
            MouseButton::Middle => "Mouse Middle".to_string(),
            MouseButton::Side => "Mouse Side".to_string(),
            MouseButton::Extra => "Mouse Extra".to_string(),
        },
        OutputAction::MouseAxis(axis) => match axis {
            MouseAxis::X => "Mouse X".to_string(),
            MouseAxis::Y => "Mouse Y".to_string(),
            MouseAxis::Wheel => "Mouse Wheel".to_string(),
        },
        OutputAction::RecenterGyro => "Recenter gyro".to_string(),
    }
}

fn output_option_matches(option: &OutputOption, output: &OutputAction) -> bool {
    match option {
        OutputOption::Action(action) => action == output,
        OutputOption::CaptureKeyboard => matches!(output, OutputAction::Keyboard { .. }),
        OutputOption::CaptureMouseButton => matches!(output, OutputAction::MouseButton(_)),
    }
}

fn output_option_label(option: &OutputOption) -> String {
    match option {
        OutputOption::Action(action) => output_display_label(action),
        OutputOption::CaptureKeyboard => "Keyboard key...".to_string(),
        OutputOption::CaptureMouseButton => "Mouse button...".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        activator_index, gyro_mode_index, gyro_mode_labels, output_display_label, output_index,
        output_labels, output_option, source_options_for_device, OutputOption,
    };
    use ira_input::{
        Activation, DeviceInfo, GamepadAxis, GamepadButton, GyroAxis, GyroMode, InputSource,
        MouseButton, OutputAction,
    };
    use std::path::PathBuf;

    #[test]
    fn test_source_options_without_device_omit_paddles() {
        let options = source_options_for_device(None);
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
        let options = source_options_for_device(Some(&device));
        assert!(options
            .iter()
            .any(|(source, _)| *source == InputSource::Button(GamepadButton::Paddle1)));
        assert!(!options
            .iter()
            .any(|(source, _)| *source == InputSource::Button(GamepadButton::Paddle2)));
    }

    #[test]
    fn test_source_options_show_raw_gyro_axes_with_semantics() {
        let options = source_options_for_device(None);
        assert!(options.contains(&(InputSource::Gyro(GyroAxis::X), "Gyro X (Pitch)".to_string())));
        assert!(options.contains(&(InputSource::Gyro(GyroAxis::Y), "Gyro Y (Yaw)".to_string())));
        assert!(options.contains(&(InputSource::Gyro(GyroAxis::Z), "Gyro Z (Roll)".to_string())));
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
    fn test_output_options_include_supported_virtual_controls() {
        let labels = output_labels();
        assert_eq!(labels.len(), 28);
        assert!(!labels.iter().any(|label| label.contains("Paddle")));
        assert!(labels.contains(&"Mouse X".to_string()));
        assert!(labels.contains(&"Keyboard key...".to_string()));
        assert_eq!(
            output_option(output_index(&OutputAction::GamepadAxis(
                GamepadAxis::RightX
            ))),
            Some(OutputOption::Action(OutputAction::GamepadAxis(
                GamepadAxis::RightX
            )))
        );
        assert_eq!(
            output_option(output_index(&OutputAction::Keyboard { keycode: 30 })),
            Some(OutputOption::CaptureKeyboard)
        );
        assert!(output_option(28).is_none());
    }

    #[test]
    fn test_output_labels_are_concise() {
        let labels = output_labels();
        assert!(labels.contains(&"A Button".to_string()));
        assert!(!labels.iter().any(|label| label.starts_with("Gamepad ")));
    }

    #[test]
    fn test_trigger_labels_distinguish_full_and_soft_bindings() {
        let sources = source_options_for_device(None);
        assert!(sources.contains(&(
            InputSource::Button(GamepadButton::LeftTrigger),
            "Left Trigger (full)".to_string()
        )));
        assert!(sources.contains(&(
            InputSource::Axis(GamepadAxis::LeftTrigger),
            "Left Trigger (soft)".to_string()
        )));

        let outputs = output_labels();
        assert!(outputs.contains(&"Right Trigger (full)".to_string()));
        assert!(outputs.contains(&"Right Trigger (soft)".to_string()));
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
        let labels = source_options_for_device(Some(&device));
        let extended = labels
            .into_iter()
            .filter_map(|(source, label)| match source {
                InputSource::Button(button) if button.is_paddle() => Some(label),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(extended, vec!["L4", "R4", "PL", "PR"]);
    }

    #[test]
    fn test_gyro_mode_labels_roundtrip_indices() {
        let labels = gyro_mode_labels();
        assert_eq!(labels, vec!["Rate", "Joystick position"]);
        assert_eq!(gyro_mode_index(GyroMode::Rate), 0);
        assert_eq!(gyro_mode_index(GyroMode::HoldLast), 1);
    }
}
