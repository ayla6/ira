//! End-to-end imports of real workshop configs copied from Steam.

use ira_input::{import_vdf, GamepadAxis, InputSource, OutputAction, SourceMode};

const FIXTURES: [&str; 3] = [
    "ps5_gyro_mouse.vdf",
    "gyro_button_mask.vdf",
    "mode_shift_trackpad.vdf",
];

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn test_workshop_fixtures_import_and_validate() {
    for name in FIXTURES {
        let text = std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        let (profile, _report) =
            import_vdf(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        profile.validate().unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!profile.action_sets.is_empty(), "{name}: no sets");
        assert!(
            !profile.name.is_empty(),
            "{name}: title should become the profile name"
        );
    }
}

#[test]
fn test_ps5_gyro_fixture_maps_trigger_axis_and_sticks() {
    let text = std::fs::read_to_string(fixture_path("ps5_gyro_mouse.vdf")).unwrap();
    let (profile, _) = import_vdf(&text).unwrap();
    let inputs = &profile.action_sets[0].inputs;
    assert!(inputs
        .iter()
        .any(|input| input.source == InputSource::Axis(GamepadAxis::LeftTrigger)));
    assert!(inputs.iter().any(|input| input.source
        == InputSource::Button(ira_input::GamepadButton::A)));
    // Gyro groups feed the gyro config rather than a mapping.
    assert!(profile.gyro.enabled);
}

#[test]
fn test_button_activators_keep_outputs() {
    let text = std::fs::read_to_string(fixture_path("ps5_gyro_mouse.vdf")).unwrap();
    let (profile, _) = import_vdf(&text).unwrap();
    let a = profile.action_sets[0]
        .inputs
        .iter()
        .find(|input| input.source == InputSource::Button(ira_input::GamepadButton::A))
        .expect("face button A must be imported");
    assert!(a.activators.iter().any(|activator| activator
        .outputs
        .contains(&OutputAction::GamepadButton(ira_input::GamepadButton::A))));
}

#[test]
fn test_stick_modes_survive_import() {
    let text = std::fs::read_to_string(fixture_path("ps5_gyro_mouse.vdf")).unwrap();
    let (profile, _) = import_vdf(&text).unwrap();
    let sticks = profile.action_sets[0]
        .inputs
        .iter()
        .filter(|input| matches!(input.source, InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::RightX)))
        .count();
    assert!(sticks >= 1);
    assert!(profile.action_sets[0].inputs.iter().any(|input| {
        matches!(input.mode, Some(SourceMode::Joystick { .. } | SourceMode::Mouse { .. }))
    }));
}
