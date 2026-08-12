use ira_input::{
    Activation, AxisDirection, DeviceInfo, GamepadAxis, GamepadButton, GyroAxis, GyroMode,
    InputSource, OutputAction, RecenterMode,
};

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
        GamepadButton::LeftTrigger => "Left Trigger".to_string(),
        GamepadButton::RightTrigger => "Right Trigger".to_string(),
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
        GamepadAxis::LeftTrigger => "Left Trigger".to_string(),
        GamepadAxis::RightTrigger => "Right Trigger".to_string(),
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

pub(super) fn output_action(index: u32) -> Result<OutputAction, String> {
    let buttons = output_buttons();
    if let Some(button) = buttons.get(index as usize) {
        return Ok(OutputAction::GamepadButton(*button));
    }
    let axis_index = index as usize - buttons.len();
    let axes = gamepad_axes();
    if let Some(axis) = axes.get(axis_index) {
        return Ok(OutputAction::GamepadAxis(*axis));
    }
    Err("Invalid XInput binding output".to_string())
}

pub(super) fn output_labels() -> Vec<String> {
    let mut labels = output_buttons()
        .into_iter()
        .map(button_label)
        .collect::<Vec<_>>();
    labels.extend(gamepad_axes().into_iter().map(axis_label));
    labels
}

pub(super) fn output_index(output: &OutputAction) -> u32 {
    match output {
        OutputAction::GamepadButton(button) => output_buttons()
            .iter()
            .position(|candidate| candidate == button)
            .unwrap_or(0) as u32,
        OutputAction::GamepadAxis(axis) => output_buttons().len() as u32 + output_axis_index(*axis),
        _ => 0,
    }
}

fn output_axis_index(axis: GamepadAxis) -> u32 {
    match axis {
        GamepadAxis::LeftX => 0,
        GamepadAxis::LeftY => 1,
        GamepadAxis::RightX => 2,
        GamepadAxis::RightY => 3,
        GamepadAxis::LeftTrigger => 4,
        GamepadAxis::RightTrigger => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        activator_index, gyro_mode_index, gyro_mode_labels, output_action, output_index,
        output_labels, source_options_for_device,
    };
    use ira_input::{
        Activation, DeviceInfo, GamepadAxis, GamepadButton, GyroAxis, GyroMode, InputSource,
        OutputAction,
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
    fn test_output_options_only_include_xinput_controls() {
        let labels = output_labels();
        assert_eq!(labels.len(), 23);
        assert!(!labels.iter().any(|label| label.contains("Paddle")));
        assert!(!labels.iter().any(|label| label.contains("Mouse")));
        assert!(!labels.iter().any(|label| label.contains("Keyboard")));
        assert_eq!(
            output_action(output_index(&OutputAction::GamepadAxis(
                GamepadAxis::RightX
            )))
            .unwrap(),
            OutputAction::GamepadAxis(GamepadAxis::RightX)
        );
        assert!(output_action(23).is_err());
    }

    #[test]
    fn test_output_labels_are_concise() {
        let labels = output_labels();
        assert!(labels.contains(&"A Button".to_string()));
        assert!(!labels.iter().any(|label| label.starts_with("Gamepad ")));
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
