//! End-to-end imports of real workshop configs copied from Steam.

use ira_input::{import_vdf, GamepadAxis, InputSource, OutputAction, SourceMode};

const FIXTURES: [&str; 3] = [
    "ps5_gyro_mouse.vdf",
    "gyro_button_mask.vdf",
    "mode_shift_trackpad.vdf",
];

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn test_workshop_fixtures_import_and_validate() {
    for name in FIXTURES {
        let text = std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        let (profile, _report) =
            import_vdf(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        profile
            .validate()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
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
    assert!(inputs
        .iter()
        .any(|input| input.source == InputSource::Button(ira_input::GamepadButton::A)));
    // Gyro groups feed the gyro config rather than a mapping.
    assert!(profile.gyro.enabled);
}

#[test]
fn test_gyro_dampening_imports_from_steam_settings() {
    let text = r#"
"controller_mappings"
{
	"title"		"Dampening layout"
	"group"
	{
		"id"		"0"
		"mode"		"four_buttons"
		"inputs"
		{
			"button_a"
			{
				"activators"
				{
					"Full_Press"
					{
						"bindings"
						{
							"binding"		"xinput_button A"
						}
					}
				}
			}
		}
	}
	"group"
	{
		"id"		"19"
		"mode"		"gyro_to_mouse"
		"inputs"
		{
		}
		"settings"
		{
			"sensitivity"				"200"
			"mouse_dampening"			"right_trigger_soft_pull"
			"mouse_dampening_amount"		"60"
		}
	}
	"preset"
	{
		"id"		"0"
		"name"		"Default"
		"group_source_bindings"
		{
			"0"		"button_diamond active"
			"19"		"gyro active"
		}
	}
	"settings"
	{
	}
}
"#;
    let (profile, _) = import_vdf(text).unwrap();
    assert!(profile.gyro.enabled);
    assert_eq!(
        profile.gyro.trigger_dampening,
        ira_input::TriggerDampening::RightTriggerSoftPull
    );
    assert!((profile.gyro.dampening_amount - 0.6).abs() < 1.0e-6);
    assert_eq!(profile.gyro.sensitivity, 2.0);
    profile.validate().unwrap();
}

#[test]
fn test_joystick_move_imports_deadzone_settings() {
    // A deadzone setting becomes a Custom deadzone; raw counts (Steam's
    // usual encoding) scale down to a fraction, fractions pass through.
    let text = r#"
"controller_mappings"
{
	"title"		"Stick layout"
	"group"
	{
		"id"		"0"
		"mode"		"joystick_move"
		"inputs"
		{
		}
		"settings"
		{
			"deadzone"		"8192"
		}
	}
	"group"
	{
		"id"		"1"
		"mode"		"joystick_move"
		"inputs"
		{
		}
		"settings"
		{
		}
	}
	"preset"
	{
		"id"		"0"
		"name"		"Default"
		"group_source_bindings"
		{
			"0"		"left_joystick active"
			"1"		"right_joystick active"
		}
	}
}
"#;
    let (profile, _) = import_vdf(text).unwrap();
    let left = profile.action_sets[0]
        .inputs
        .iter()
        .find(|input| input.source == InputSource::Axis(GamepadAxis::LeftX))
        .expect("left stick must be imported");
    let Some(SourceMode::Joystick(settings)) = left.mode.as_ref() else {
        panic!("expected a joystick mode");
    };
    assert_eq!(
        settings.processing.deadzone,
        ira_input::StickDeadzone::Custom
    );
    assert!((settings.processing.deadzone_inner - 8192.0 / 32767.0).abs() < 1.0e-4);

    let right = profile.action_sets[0]
        .inputs
        .iter()
        .find(|input| input.source == InputSource::Axis(GamepadAxis::RightX))
        .expect("right stick must be imported");
    let Some(SourceMode::Joystick(settings)) = right.mode.as_ref() else {
        panic!("expected a joystick mode");
    };
    // No deadzone setting: raw input, Steam's default.
    assert_eq!(settings.processing.deadzone, ira_input::StickDeadzone::None);
    profile.validate().unwrap();
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
    assert!(a.activators.iter().any(|activator| {
        activator
            .outputs
            .contains(&OutputAction::GamepadButton(ira_input::GamepadButton::A))
    }));
}

#[test]
fn test_stick_modes_survive_import() {
    let text = std::fs::read_to_string(fixture_path("ps5_gyro_mouse.vdf")).unwrap();
    let (profile, _) = import_vdf(&text).unwrap();
    let sticks = profile.action_sets[0]
        .inputs
        .iter()
        .filter(|input| {
            matches!(
                input.source,
                InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::RightX)
            )
        })
        .count();
    assert!(sticks >= 1);
    assert!(profile.action_sets[0].inputs.iter().any(|input| {
        matches!(
            input.mode,
            Some(SourceMode::Joystick { .. } | SourceMode::Mouse { .. })
        )
    }));
}
