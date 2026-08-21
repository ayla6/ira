use super::assets::source_badge;
use ira_input::{
    Activation, Binding, DeviceInfo, GamepadAxis, GamepadButton, InputCategory, InputSource,
    VirtualGamepadBackend,
};

pub(crate) fn binding_page_index(binding: &Binding) -> usize {
    match binding.source.category() {
        InputCategory::Buttons => 0,
        InputCategory::Dpad => 1,
        InputCategory::Triggers => 2,
        InputCategory::Joysticks => 3,
        InputCategory::Gyro => 4,
    }
}

pub(crate) fn binding_section_title(binding: &Binding) -> &'static str {
    match binding.source {
        InputSource::Button(GamepadButton::LeftTrigger | GamepadButton::RightTrigger) => "Triggers",
        InputSource::Button(
            GamepadButton::A | GamepadButton::B | GamepadButton::X | GamepadButton::Y,
        ) => "Face Buttons",
        InputSource::Button(GamepadButton::LeftShoulder | GamepadButton::RightShoulder) => {
            "Bumpers"
        }
        InputSource::Button(GamepadButton::Back | GamepadButton::Start | GamepadButton::Guide) => {
            "Menu Buttons"
        }
        InputSource::Button(GamepadButton::LeftStick | GamepadButton::RightStick) => "Stick Clicks",
        InputSource::Button(
            GamepadButton::DpadUp
            | GamepadButton::DpadDown
            | GamepadButton::DpadLeft
            | GamepadButton::DpadRight,
        ) => "D-pad",
        InputSource::Button(
            GamepadButton::Paddle1
            | GamepadButton::Paddle2
            | GamepadButton::Paddle3
            | GamepadButton::Paddle4
            | GamepadButton::Paddle5
            | GamepadButton::Paddle6
            | GamepadButton::Paddle7
            | GamepadButton::Paddle8,
        ) => "Extended Buttons",
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
        | InputSource::AxisDirection {
            axis: GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger,
            ..
        } => "Triggers",
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::LeftY)
        | InputSource::AxisDirection {
            axis: GamepadAxis::LeftX | GamepadAxis::LeftY,
            ..
        } => "Left Stick",
        InputSource::Axis(GamepadAxis::RightX | GamepadAxis::RightY)
        | InputSource::AxisDirection {
            axis: GamepadAxis::RightX | GamepadAxis::RightY,
            ..
        } => "Right Stick",
    }
}

pub(crate) fn section_title_label(title: &str) -> String {
    match title {
        "Triggers" => crate::tr!("Triggers"),
        "Face Buttons" => crate::tr!("Face Buttons"),
        "Bumpers" => crate::tr!("Bumpers"),
        "Menu Buttons" => crate::tr!("Menu Buttons"),
        "Stick Clicks" => crate::tr!("Stick Clicks"),
        "D-pad" => crate::tr!("D-pad"),
        "Extended Buttons" => crate::tr!("Extended Buttons"),
        "Left Stick" => crate::tr!("Left Stick"),
        "Right Stick" => crate::tr!("Right Stick"),
        "Gyro" => crate::tr!("Gyro"),
        "Custom" => crate::tr!("Custom"),
        _ => title.to_string(),
    }
}

pub(super) fn section_source_options(
    page_index: usize,
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
    current_source: Option<InputSource>,
) -> Vec<(InputSource, String)> {
    let category = match page_index {
        0 => InputCategory::Buttons,
        1 => InputCategory::Dpad,
        2 => InputCategory::Triggers,
        3 => InputCategory::Joysticks,
        4 => InputCategory::Gyro,
        _ => return Vec::new(),
    };
    let mut options =
        super::super::input_profile_options::source_options_for_device(device, backend)
            .into_iter()
            .filter(|(source, _)| source.category() == category)
            .collect::<Vec<_>>();
    if let Some(source) =
        current_source.filter(|source| !options.iter().any(|(candidate, _)| candidate == source))
    {
        options.push((
            source,
            format!(
                "{} (unavailable)",
                source_badge(source, ira_input::ControllerFamily::default())
            ),
        ));
    }
    options
}

pub(super) fn activation_sources(activation: &Activation) -> Vec<InputSource> {
    match activation {
        Activation::Hold(source)
        | Activation::Toggle(source)
        | Activation::DisableWhile(source) => vec![*source],
        Activation::Chord { sources, .. } => sources.clone(),
        Activation::Always => Vec::new(),
    }
}

pub(super) fn chord_text_for_options(
    activation: &Activation,
    options: &[(InputSource, String)],
) -> String {
    match activation {
        Activation::Chord { sources, .. } => sources
            .iter()
            .filter_map(|source| {
                options
                    .iter()
                    .find(|(candidate, _)| candidate == source)
                    .map(|(_, label)| label.clone())
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

pub(super) fn source_index_for(options: &[(InputSource, String)], source: InputSource) -> u32 {
    options
        .iter()
        .position(|(candidate, _)| *candidate == source)
        .unwrap_or(0) as u32
}

pub(super) fn is_analog_source(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(_) | InputSource::AxisDirection { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::{activation_sources, is_analog_source};
    use ira_input::{
        Activation, AxisDirection, ChordMode, GamepadAxis, GamepadButton, InputSource,
    };

    #[test]
    fn test_button_binding_does_not_use_axis_controls() {
        assert!(!is_analog_source(InputSource::Button(GamepadButton::A)));
        assert!(is_analog_source(InputSource::Axis(GamepadAxis::LeftX)));
        assert!(is_analog_source(InputSource::AxisDirection {
            axis: GamepadAxis::RightY,
            direction: AxisDirection::Positive,
        }));
    }

    #[test]
    fn test_activation_sources_preserves_unavailable_chord_inputs() {
        let sources = activation_sources(&Activation::Chord {
            sources: vec![
                InputSource::Button(GamepadButton::Paddle1),
                InputSource::Button(GamepadButton::Guide),
            ],
            mode: ChordMode::Hold,
        });
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0], InputSource::Button(GamepadButton::Paddle1));
    }
}
