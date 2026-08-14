use super::super::input_profile_editor_pages::section_order;
use super::super::input_profile_editor_sections::{
    default_profile_bindings, section_behavior_bindings, stick_to_dpad_bindings,
};
use super::{build_profile, profile_path_for_save};
use ira_input::{
    AxisDirection, DeviceInfo, GamepadAxis, GamepadButton, GyroCalibration, InputSource,
    OutputAction, VirtualGamepadBackend,
};
use std::path::PathBuf;

#[test]
fn test_default_profile_bindings_without_device_omit_paddles() {
    let bindings = default_profile_bindings(None, VirtualGamepadBackend::XInput);
    assert!(!bindings.is_empty());
    assert!(!bindings.iter().any(
        |binding| matches!(binding.source, InputSource::Button(button) if button.is_paddle())
    ));
}

#[test]
fn test_default_profile_bindings_include_only_supported_buttons() {
    let device = DeviceInfo {
        path: PathBuf::from("/dev/input/event0"),
        name: "Test controller".to_string(),
        vendor: 0,
        product: 0,
        version: 0,
        has_evdev_gyro: false,
        supported_buttons: vec![GamepadButton::A, GamepadButton::Paddle1],
    };
    let bindings = default_profile_bindings(Some(&device), VirtualGamepadBackend::XInput);
    assert!(bindings
        .iter()
        .any(|binding| binding.source == InputSource::Button(GamepadButton::A)));
    assert!(!bindings
        .iter()
        .any(|binding| { binding.source == InputSource::Button(GamepadButton::Paddle1) }));
    assert!(!bindings
        .iter()
        .any(|binding| binding.source == InputSource::Button(GamepadButton::B)));
}

#[test]
fn test_stick_to_dpad_preset_assigns_each_direction() {
    let bindings = stick_to_dpad_bindings(GamepadAxis::LeftX, GamepadAxis::LeftY);
    assert_eq!(
        bindings[0].source,
        InputSource::AxisDirection {
            axis: GamepadAxis::LeftX,
            direction: AxisDirection::Negative,
        }
    );
    assert_eq!(
        bindings[1].output,
        OutputAction::GamepadButton(GamepadButton::DpadRight)
    );
    assert_eq!(
        bindings[2].output,
        OutputAction::GamepadButton(GamepadButton::DpadUp)
    );
    assert_eq!(
        bindings[3].output,
        OutputAction::GamepadButton(GamepadButton::DpadDown)
    );
}

#[test]
fn test_buttons_sections_follow_steam_order() {
    assert!(section_order("Face Buttons") < section_order("Bumpers"));
    assert!(section_order("Bumpers") < section_order("Extended Buttons"));
    assert!(section_order("Extended Buttons") < section_order("Menu Buttons"));
    assert!(section_order("Menu Buttons") < section_order("Stick Clicks"));
}

#[test]
fn test_stick_behavior_replaces_with_directional_bindings() {
    let bindings = section_behavior_bindings("Left Stick", 3, None, VirtualGamepadBackend::XInput);
    assert_eq!(bindings.len(), 4);
    assert!(bindings.iter().all(|binding| matches!(
        binding.output,
        OutputAction::GamepadButton(
            GamepadButton::DpadUp
                | GamepadButton::DpadDown
                | GamepadButton::DpadLeft
                | GamepadButton::DpadRight
        )
    )));
    let default = section_behavior_bindings("Left Stick", 2, None, VirtualGamepadBackend::XInput);
    assert_eq!(default.len(), 2);
    assert_ne!(default, bindings);
}

#[test]
fn test_custom_section_sorts_before_standard_sections() {
    assert!(section_order("Custom") < section_order("Stick Clicks"));
}

#[test]
fn test_profile_save_keeps_existing_path() {
    let tmp = tempfile::tempdir().unwrap();
    let old_path = tmp.path().join("old-name.json");
    let saved_path =
        profile_path_for_save(tmp.path().to_str().unwrap(), Some(&old_path), "New name");
    assert_eq!(saved_path, old_path);
}

#[test]
fn test_build_profile_uses_updated_calibration() {
    let calibration = GyroCalibration {
        x: 1.0,
        y: -2.0,
        z: 3.0,
    };
    let profile = build_profile(
        "Test",
        &[],
        calibration,
        &[],
        None,
        VirtualGamepadBackend::XInput,
    )
    .unwrap();
    assert_eq!(profile.gyro_calibration, calibration);
}
