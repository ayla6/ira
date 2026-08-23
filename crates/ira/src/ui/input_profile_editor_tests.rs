use super::{default_action_sets, test_default_output};
use super::super::input_profile_editor_save::profile_path_for_save;
use ira_input::{
    DeviceInfo, GamepadAxis, GamepadButton, InputSource, OutputAction, SourceMode,
    StickOutput, VirtualGamepadBackend,
};
use std::path::PathBuf;

#[test]
fn test_default_action_sets_map_identity_buttons() {
    let sets = default_action_sets(None, VirtualGamepadBackend::XInput);
    assert!(!sets.is_empty());
    let inputs = &sets[0].inputs;
    let a_mapped = inputs.iter().any(|input| {
        input.source == InputSource::Button(GamepadButton::A)
            && input.activators.iter().any(|activator| {
                activator
                    .outputs
                    .contains(&OutputAction::GamepadButton(GamepadButton::A))
            })
    });
    assert!(a_mapped);
    assert!(!inputs
        .iter()
        .any(|input| matches!(input.source, InputSource::Button(button) if button.is_paddle())));
}

#[test]
fn test_default_action_sets_honor_device_buttons() {
    let device = DeviceInfo {
        path: PathBuf::from("/dev/input/event0"),
        name: "Test controller".to_string(),
        vendor: 0,
        product: 0,
        version: 0,
        has_evdev_gyro: false,
        supported_buttons: vec![GamepadButton::A, GamepadButton::Paddle1],
    };
    // Paddles only exist on DirectInput-class virtual devices.
    let sets = default_action_sets(Some(&device), VirtualGamepadBackend::DirectInput);
    assert!(sets[0]
        .inputs
        .iter()
        .any(|input| input.source == InputSource::Button(GamepadButton::Paddle1)));
}

#[test]
fn test_default_mapping_gives_axes_their_natural_modes() {
    assert_eq!(
        test_default_output(InputSource::Button(GamepadButton::B)),
        OutputAction::GamepadButton(GamepadButton::B)
    );
    let stick = super::super::input_profile_region_pages::default_mapping(
        InputSource::Axis(GamepadAxis::RightX),
        VirtualGamepadBackend::XInput,
    );
    assert!(matches!(
        stick.mode,
        Some(SourceMode::Joystick {
            output: StickOutput::Right,
            ..
        })
    ));
    assert!(stick.activators.is_empty());
    let trigger = super::super::input_profile_region_pages::default_mapping(
        InputSource::Axis(GamepadAxis::LeftTrigger),
        VirtualGamepadBackend::XInput,
    );
    assert!(matches!(trigger.mode, Some(SourceMode::Trigger { .. })));
}

#[test]
fn test_profile_path_for_save_prefers_current_path() {
    let existing = PathBuf::from("/tmp/profiles/mine.json");
    assert_eq!(
        profile_path_for_save("/tmp/profiles", Some(&existing), "renamed"),
        existing
    );
    let fresh = profile_path_for_save("/tmp/profiles", None, "New Layout");
    assert!(fresh.to_string_lossy().starts_with("/tmp/profiles"));
}
