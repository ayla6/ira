use super::super::input_profile_editor_save::{is_dirty, profile_path_for_save, EditorForm};
use super::{default_action_sets, test_default_output};
use ira_input::{
    DeviceInfo, GamepadAxis, GamepadButton, InputProfile, InputSource, OutputAction, SourceMode,
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
    let stick = super::super::input_profile_input_rows::default_mapping(InputSource::Axis(
        GamepadAxis::RightX,
    ));
    let Some(SourceMode::Joystick(settings)) = stick.mode.as_ref() else {
        panic!("expected a joystick mode");
    };
    assert_eq!(settings.output, StickOutput::Right);
    // Fresh sticks start on the controller-preference deadzone.
    assert_eq!(
        settings.processing.deadzone,
        ira_input::StickDeadzone::Controller
    );
    assert!(stick.activators.is_empty());
    let trigger = super::super::input_profile_input_rows::default_mapping(InputSource::Axis(
        GamepadAxis::LeftTrigger,
    ));
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

fn dirty_form(profile: InputProfile) -> (EditorForm, InputProfile) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let form = EditorForm {
        name: Rc::new(RefCell::new(profile.name.clone())),
        profile: Rc::new(RefCell::new(profile.clone())),
        calibration: Rc::new(RefCell::new(profile.controller_calibration)),
        gyro: Rc::new(RefCell::new(profile.gyro.clone())),
        compatible_game_ids: profile.compatible_game_ids.clone(),
        game_id: None,
        scopes_to_game: false,
    };
    (form, profile)
}

#[test]
fn test_is_dirty_new_layout_without_changes_is_saveable() {
    let (form, baseline) = dirty_form(InputProfile::default());
    // A never-saved layout must count as dirty even when identical to its
    // baseline, otherwise Save stays disabled and it can never be written.
    assert!(is_dirty(true, &form, &baseline));
    // The same form on an existing profile is clean.
    assert!(!is_dirty(false, &form, &baseline));
}

#[test]
fn test_is_dirty_flags_modified_form() {
    let (form, baseline) = dirty_form(InputProfile::default());
    assert!(!is_dirty(false, &form, &baseline));
    let renamed = "Renamed layout".to_string();
    form.profile.borrow_mut().name = renamed.clone();
    *form.name.borrow_mut() = renamed;
    assert!(is_dirty(false, &form, &baseline));
    assert!(is_dirty(true, &form, &baseline));
}

#[test]
fn test_build_profile_scopes_new_layouts_but_not_edits() {
    use crate::ui::input_profile_editor_save::build_profile;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn form_with(profile: &InputProfile, scopes: bool) -> EditorForm {
        EditorForm {
            name: Rc::new(RefCell::new(profile.name.clone())),
            profile: Rc::new(RefCell::new(profile.clone())),
            calibration: Rc::new(RefCell::new(profile.controller_calibration)),
            gyro: Rc::new(RefCell::new(profile.gyro.clone())),
            compatible_game_ids: profile.compatible_game_ids.clone(),
            game_id: Some(7),
            scopes_to_game: scopes,
        }
    }

    // A brand-new layout created from a game's page starts scoped to it.
    let fresh = InputProfile::default();
    let built = build_profile(&form_with(&fresh, true)).unwrap();
    assert_eq!(built.compatible_game_ids, vec![7]);

    // Editing a global layout through that same page must keep it global:
    // silently scoping it hid the profile from every other game's picker.
    let global = InputProfile::default();
    let rebuilt = build_profile(&form_with(&global, false)).unwrap();
    assert!(rebuilt.compatible_game_ids.is_empty());

    // An already-scoped layout keeps its scope across edits.
    let scoped = InputProfile {
        compatible_game_ids: vec![3],
        ..InputProfile::default()
    };
    let rebuilt = build_profile(&form_with(&scoped, false)).unwrap();
    assert_eq!(rebuilt.compatible_game_ids, vec![3]);
}
