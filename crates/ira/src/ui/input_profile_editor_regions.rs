//! Region data for the action-set editor: the Steam Input page list
//! (Buttons with the bumpers, D-pad, Triggers, Joysticks, System and
//! Paddles), the inputs each page groups where they sit on the hardware,
//! and the human labels shared by rows, pickers, and sheets.

use super::input_profile_options::{axis_label, button_label};
use ira_input::{GamepadAxis, GamepadButton, InputSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Buttons,
    Dpad,
    Triggers,
    Joysticks,
    SystemPaddles,
}

impl Region {
    pub(crate) const ALL: [Region; 5] = [
        Region::Buttons,
        Region::Dpad,
        Region::Triggers,
        Region::Joysticks,
        Region::SystemPaddles,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Region::Buttons => "buttons",
            Region::Dpad => "dpad",
            Region::Triggers => "triggers",
            Region::Joysticks => "joysticks",
            Region::SystemPaddles => "system",
        }
    }

    pub(crate) fn title(self) -> String {
        match self {
            Region::Buttons => crate::tr!("Buttons"),
            Region::Dpad => crate::tr!("D-pad"),
            Region::Triggers => crate::tr!("Triggers"),
            Region::Joysticks => crate::tr!("Joysticks"),
            Region::SystemPaddles => crate::tr!("System and Paddles"),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Region::Buttons => "input-gaming-symbolic",
            Region::Dpad => "view-grid-symbolic",
            Region::Triggers => "media-seek-forward-symbolic",
            Region::Joysticks => "media-playback-start-symbolic",
            Region::SystemPaddles => "emblem-system-symbolic",
        }
    }
}

/// Human label for any bindable source.
pub(crate) fn source_label(source: InputSource) -> String {
    match source {
        InputSource::Button(button) => button_label(button),
        // A stick row speaks of the whole stick, not one of its axes.
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::LeftY) => crate::tr!("Left Stick"),
        InputSource::Axis(GamepadAxis::RightX | GamepadAxis::RightY) => crate::tr!("Right Stick"),
        InputSource::Axis(axis) => axis_label(axis),
        InputSource::AxisDirection { axis, direction } => {
            let sign = match direction {
                ira_input::AxisDirection::Negative => "-",
                ira_input::AxisDirection::Positive => "+",
            };
            crate::tr!("{} ({})")
                .replacen("{}", &axis_label(axis), 1)
                .replacen("{}", sign, 1)
        }
    }
}

/// Every button source a device supports, in stable display order.
pub(crate) fn supported_button_sources(device: Option<&ira_input::DeviceInfo>) -> Vec<InputSource> {
    let all = [
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
        GamepadButton::Paddle1,
        GamepadButton::Paddle2,
        GamepadButton::Paddle3,
        GamepadButton::Paddle4,
        GamepadButton::Paddle5,
        GamepadButton::Paddle6,
        GamepadButton::Paddle7,
        GamepadButton::Paddle8,
    ];
    all.into_iter()
        .filter(|button| {
            device.is_none_or(|device| {
                device.supported_buttons.contains(button)
                    || (button.is_paddle() && device.supported_buttons.contains(button))
            })
        })
        .map(InputSource::Button)
        .collect()
}

/// One titled block of inputs on a region page — Steam groups the face
/// buttons with the bumpers, and each joystick with its click.
pub(crate) struct SourceGroup {
    pub(crate) title: String,
    pub(crate) sources: Vec<InputSource>,
}

/// The rows of one region page, grouped the way the hardware sits.
pub(crate) fn region_groups(
    region: Region,
    device: Option<&ira_input::DeviceInfo>,
) -> Vec<SourceGroup> {
    let supported = supported_button_sources(device);
    let buttons = |wanted: &[GamepadButton]| -> Vec<InputSource> {
        supported
            .iter()
            .filter(
                |source| matches!(source, InputSource::Button(button) if wanted.contains(button)),
            )
            .copied()
            .collect()
    };
    let mut groups = match region {
        Region::Buttons => vec![
            SourceGroup {
                title: crate::tr!("Face Buttons"),
                sources: buttons(&[
                    GamepadButton::A,
                    GamepadButton::B,
                    GamepadButton::X,
                    GamepadButton::Y,
                ]),
            },
            SourceGroup {
                title: crate::tr!("Bumpers"),
                sources: buttons(&[GamepadButton::LeftShoulder, GamepadButton::RightShoulder]),
            },
        ],
        Region::Dpad => vec![SourceGroup {
            title: crate::tr!("D-pad"),
            sources: buttons(&[
                GamepadButton::DpadUp,
                GamepadButton::DpadDown,
                GamepadButton::DpadLeft,
                GamepadButton::DpadRight,
            ]),
        }],
        // Each trigger pairs its analog value with the digital click some
        // pads report, the way joysticks pair behavior with click.
        Region::Triggers => {
            let trigger_sources = |axis: GamepadAxis, click: GamepadButton| -> Vec<InputSource> {
                let mut sources = vec![InputSource::Axis(axis)];
                sources.extend(buttons(&[click]));
                sources
            };
            vec![
                SourceGroup {
                    title: crate::tr!("Left Trigger"),
                    sources: trigger_sources(
                        GamepadAxis::LeftTrigger,
                        GamepadButton::LeftTrigger,
                    ),
                },
                SourceGroup {
                    title: crate::tr!("Right Trigger"),
                    sources: trigger_sources(
                        GamepadAxis::RightTrigger,
                        GamepadButton::RightTrigger,
                    ),
                },
            ]
        }
        // A stick is one input: its behavior lives on the X axis, and the
        // click is the group's second row, like Steam's joystick list.
        Region::Joysticks => vec![
            SourceGroup {
                title: crate::tr!("Left Joystick"),
                sources: vec![
                    InputSource::Axis(GamepadAxis::LeftX),
                    InputSource::Button(GamepadButton::LeftStick),
                ],
            },
            SourceGroup {
                title: crate::tr!("Right Joystick"),
                sources: vec![
                    InputSource::Axis(GamepadAxis::RightX),
                    InputSource::Button(GamepadButton::RightStick),
                ],
            },
        ],
        Region::SystemPaddles => {
            let mut groups = vec![SourceGroup {
                title: crate::tr!("System"),
                sources: buttons(&[
                    GamepadButton::Back,
                    GamepadButton::Start,
                    GamepadButton::Guide,
                ]),
            }];
            groups.push(SourceGroup {
                title: crate::tr!("Paddles"),
                sources: buttons(&[
                    GamepadButton::Paddle1,
                    GamepadButton::Paddle2,
                    GamepadButton::Paddle3,
                    GamepadButton::Paddle4,
                    GamepadButton::Paddle5,
                    GamepadButton::Paddle6,
                    GamepadButton::Paddle7,
                    GamepadButton::Paddle8,
                ]),
            });
            groups
        }
    };
    // Devices without, say, a guide button skip the otherwise empty group.
    groups.retain(|group| !group.sources.is_empty());
    groups
}

pub(crate) fn activator_kind_label(kind: &ira_input::ActivatorKind) -> String {
    match kind {
        ira_input::ActivatorKind::FullPress => crate::tr!("Click"),
        ira_input::ActivatorKind::DoublePress { .. } => crate::tr!("Double press"),
        ira_input::ActivatorKind::LongPress { .. } => crate::tr!("Long press"),
        ira_input::ActivatorKind::StartPress => crate::tr!("On press down"),
        ira_input::ActivatorKind::Release => crate::tr!("On release"),
        ira_input::ActivatorKind::SoftPress { .. } => crate::tr!("Soft pull"),
    }
}

#[cfg(test)]
mod tests {
    use super::{region_groups, Region};
    use ira_input::{GamepadAxis, GamepadButton, InputSource};

    #[test]
    fn test_region_groups_buttons_split_face_and_bumpers() {
        let groups = region_groups(Region::Buttons, None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, crate::tr!("Face Buttons"));
        assert_eq!(groups[0].sources.len(), 4);
        assert!(groups[0]
            .sources
            .contains(&InputSource::Button(GamepadButton::A)));
        assert_eq!(groups[1].title, crate::tr!("Bumpers"));
        assert_eq!(
            groups[1].sources,
            vec![
                InputSource::Button(GamepadButton::LeftShoulder),
                InputSource::Button(GamepadButton::RightShoulder),
            ]
        );
    }

    #[test]
    fn test_region_groups_joysticks_pair_behavior_with_click() {
        let groups = region_groups(Region::Joysticks, None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, crate::tr!("Left Joystick"));
        assert_eq!(
            groups[0].sources,
            vec![
                InputSource::Axis(GamepadAxis::LeftX),
                InputSource::Button(GamepadButton::LeftStick),
            ]
        );
        assert_eq!(groups[1].title, crate::tr!("Right Joystick"));
        assert_eq!(
            groups[1].sources,
            vec![
                InputSource::Axis(GamepadAxis::RightX),
                InputSource::Button(GamepadButton::RightStick),
            ]
        );
    }

    #[test]
    fn test_region_groups_pair_trigger_axis_with_button() {
        let groups = region_groups(Region::Triggers, None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, crate::tr!("Left Trigger"));
        assert_eq!(
            groups[0].sources,
            vec![
                InputSource::Axis(GamepadAxis::LeftTrigger),
                InputSource::Button(GamepadButton::LeftTrigger),
            ]
        );
        assert_eq!(groups[1].title, crate::tr!("Right Trigger"));
        assert_eq!(
            groups[1].sources,
            vec![
                InputSource::Axis(GamepadAxis::RightTrigger),
                InputSource::Button(GamepadButton::RightTrigger),
            ]
        );
    }

    #[test]
    fn test_region_groups_drop_empty_groups_for_narrow_devices() {
        let device = ira_input::DeviceInfo {
            path: std::path::PathBuf::from("/dev/input/event0"),
            name: "Minimal pad".to_string(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: vec![GamepadButton::A],
        };
        let groups = region_groups(Region::Buttons, Some(&device));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, crate::tr!("Face Buttons"));
        assert_eq!(
            groups[0].sources,
            vec![InputSource::Button(GamepadButton::A)]
        );

        // Without a device every standard group is present.
        let system = region_groups(Region::SystemPaddles, None);
        assert_eq!(system.len(), 2);
        assert_eq!(system[0].title, crate::tr!("System"));
        assert_eq!(system[1].title, crate::tr!("Paddles"));
    }
}
