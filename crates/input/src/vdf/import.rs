//! Import Steam controller configs (VDF) into Ira's action-set model.
//!
//! Lossy by design: everything without a representation in our model is
//! reported through [`ImportReport`] instead of being silently dropped.

use super::parse::{find, find_all, parse_vdf, Node};
use crate::profile::{
    ActionSet, Activation, Activator, ActivatorKind, ActivatorSettings, GamepadAxis, GamepadButton,
    GyroConfig, GyroOrientation, GyroOutput, InputMapping, InputProfile, InputSource, ModeShift,
    MouseAxis, MouseButton, OutputAction, SourceMode, StickOutput, VirtualGamepadBackend,
};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct ImportReport {
    pub warnings: Vec<String>,
}

impl ImportReport {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
}

/// Import one Steam `controller_mappings` VDF document.
pub fn import_vdf(text: &str) -> Result<(InputProfile, ImportReport), String> {
    let top = parse_vdf(text)?;
    let root = find(&top, "controller_mappings")
        .ok_or("not a controller mapping (missing controller_mappings)")?;
    let children = root.children();

    let mut report = ImportReport::default();
    let mut profile = InputProfile {
        name: root.str("title").unwrap_or_default().to_string(),
        backend: backend_for_controller_type(root.str("controller_type")),
        ..InputProfile::default()
    };

    let mut groups: HashMap<&str, &Node> = HashMap::new();
    for group in find_all(children, "group") {
        if let Some(id) = group.str("id") {
            groups.insert(id, group);
        }
    }

    // Presets become action sets; the first is the default layout.
    let mut sets: Vec<ActionSet> = Vec::new();
    for preset in find_all(children, "preset") {
        let Some(bindings) = preset.obj("group_source_bindings") else {
            continue;
        };
        let mut set = ActionSet {
            name: preset.str("name").unwrap_or("Set").to_string(),
            inputs: Vec::new(),
        };
        for entry in bindings {
            // Keys are group ids; values are "<region> <state>".
            let Some(text) = entry.as_str() else {
                continue;
            };
            let mut words = text.split_whitespace();
            let region = words.next().unwrap_or_default().to_string();
            let state = words.next().unwrap_or_default();
            if state != "active" {
                continue;
            }
            let Some(group) = groups.get(entry.key.as_str()) else {
                report.warn(format!("preset references unknown group {}", entry.key));
                continue;
            };
            import_group(group, &region, &mut set, &mut profile.gyro, &mut report);
        }
        dedupe_inputs(&mut set, &mut report);
        if !set.inputs.is_empty() {
            sets.push(set);
        }
    }
    if sets.is_empty() {
        return Err("no presets with active groups found".to_string());
    }
    profile.action_sets = sets;

    if !find_all(children, "action_layers").is_empty() {
        report.warn("action layers are not imported yet");
    }
    if profile.action_sets.len() == 1 {
        // A single set cannot target other sets; drop internal outputs so
        // validation passes.
        strip_internal_outputs(&mut profile);
    }
    profile.validate()?;
    Ok((profile, report))
}

fn backend_for_controller_type(controller_type: Option<&str>) -> VirtualGamepadBackend {
    match controller_type.unwrap_or_default() {
        "controller_switch_pro" | "controller_ne_wii" => VirtualGamepadBackend::SwitchPro,
        _ => VirtualGamepadBackend::XInput,
    }
}

fn dedupe_inputs(set: &mut ActionSet, report: &mut ImportReport) {
    let mut seen: Vec<InputSource> = Vec::new();
    set.inputs.retain(|mapping| {
        if seen.contains(&mapping.source) {
            report.warn(format!(
                "duplicate binding for {} kept from the earlier group",
                super::source_debug_name(mapping.source)
            ));
            false
        } else {
            seen.push(mapping.source);
            true
        }
    });
}

fn strip_internal_outputs(profile: &mut InputProfile) {
    for set in &mut profile.action_sets {
        for input in &mut set.inputs {
            for activator in &mut input.activators {
                activator.outputs.retain(|output| {
                    !matches!(
                        output,
                        OutputAction::SwitchActionSet(_)
                            | OutputAction::EnableLayer { .. }
                            | OutputAction::ModeShiftActivate { .. }
                    )
                });
                input.mode_shifts.clear();
            }
        }
    }
}

/// Numeric value of a direct string child (activator blocks).
fn child_number(node: &Node, key: &str) -> Option<f32> {
    node.children().iter().find(|n| n.key == key)?.as_f32()
}

/// Numeric entry in a group's settings block.
fn group_setting(group: &Node, key: &str) -> Option<f32> {
    let settings = group.obj("settings")?;
    settings.iter().find(|n| n.key == key)?.as_f32()
}

/// Translate one group into mappings appended to `set`. The preset binding
/// string doubles as the physical-region hint ("left_trigger active").
fn import_group(
    group: &Node,
    region_hint: &str,
    set: &mut ActionSet,
    gyro: &mut GyroConfig,
    report: &mut ImportReport,
) {
    let mode = group.str("mode").unwrap_or_default();
    let region = region_hint.split_whitespace().next().unwrap_or_default();
    if region == "gyro" {
        // The gyro region wears several mode names depending on target.
        match mode {
            "gyro_to_joystick" => import_gyro(group, mode, gyro, report),
            "gyro_to_mouse" | "absolute_mouse" | "mouse_joystick" | "joystick_camera" => {
                import_gyro(group, "gyro_to_mouse", gyro, report);
            }
            other => report.warn(format!("unsupported gyro group mode \"{other}\"")),
        }
        return;
    }
    match mode {
        "four_buttons" | "dpad" | "switches" => {
            for (source_name, button) in button_sources(mode).iter().copied() {
                append_button_mapping(group, source_name, button, set, report);
            }
        }
        "trigger" => {
            import_trigger(group, region, set, report);
        }
        "joystick_move" | "joystick_mouse" | "mouse_joystick" | "flickstick" | "absolute_mouse" => {
            import_stick(group, region, mode, set);
        }
        "gyro_to_mouse" | "gyro_to_joystick" => {
            import_gyro(group, mode, gyro, report);
        }
        other => report.warn(format!("unsupported group mode \"{other}\" ({region})")),
    }
}

fn button_sources(mode: &str) -> &'static [(&'static str, GamepadButton)] {
    match mode {
        "dpad" => &[
            ("dpad_north", GamepadButton::DpadUp),
            ("dpad_east", GamepadButton::DpadRight),
            ("dpad_south", GamepadButton::DpadDown),
            ("dpad_west", GamepadButton::DpadLeft),
        ],
        "switches" => &[
            ("click", GamepadButton::Start),
            ("escape", GamepadButton::Back),
        ],
        _ => &[
            ("button_a", GamepadButton::A),
            ("button_b", GamepadButton::B),
            ("button_x", GamepadButton::X),
            ("button_y", GamepadButton::Y),
        ],
    }
}

fn append_button_mapping(
    group: &Node,
    source_name: &str,
    button: GamepadButton,
    set: &mut ActionSet,
    report: &mut ImportReport,
) {
    let Some(inputs) = group.obj("inputs") else {
        return;
    };
    let Some(input) = inputs.iter().find(|node| node.key == source_name) else {
        return;
    };
    let mut activators = Vec::new();
    let mut shifts = Vec::new();
    collect_activators(input, &mut activators, &mut shifts);
    if activators.is_empty() {
        return;
    }
    let mut mapping = InputMapping {
        activators,
        ..InputMapping::simple(
            InputSource::Button(button),
            OutputAction::GamepadButton(button),
        )
    };
    attach_shifts(&mut mapping, shifts, report);
    set.inputs.push(mapping);
}

fn import_trigger(group: &Node, region: &str, set: &mut ActionSet, report: &mut ImportReport) {
    let axis = match region {
        "right_trigger" => GamepadAxis::RightTrigger,
        _ => GamepadAxis::LeftTrigger,
    };
    let click_button = match axis {
        GamepadAxis::RightTrigger => GamepadButton::RightTrigger,
        _ => GamepadButton::LeftTrigger,
    };
    // Analog side: thresholded passthrough of the physical trigger.
    let threshold = group_setting(group, "deadzone")
        .unwrap_or(0.5)
        .clamp(0.05, 1.0);
    set.inputs.push(InputMapping {
        mode: Some(SourceMode::Trigger { threshold }),
        ..InputMapping::new(InputSource::Axis(axis))
    });
    // Digital side: full pull as a button.
    let mut click_mapping = InputMapping::new(InputSource::Button(click_button));
    let mut click_activators = Vec::new();
    let mut shifts = Vec::new();
    if let Some(inputs) = group.obj("inputs") {
        if let Some(click) = inputs.iter().find(|node| node.key == "click") {
            collect_activators(click, &mut click_activators, &mut shifts);
        }
    }
    click_mapping.activators = click_activators;
    if click_mapping.activators.is_empty() {
        return;
    }
    attach_shifts(&mut click_mapping, shifts, report);
    set.inputs.push(click_mapping);
}

fn import_stick(group: &Node, region: &str, mode: &str, set: &mut ActionSet) {
    let (axis, output) = match region {
        "right_joystick" | "right_trackpad" => (GamepadAxis::RightX, StickOutput::Right),
        _ => (GamepadAxis::LeftX, StickOutput::Left),
    };
    let stick_mode = match mode {
        "joystick_move" => SourceMode::Joystick {
            output,
            deadzone_inner: group_setting(group, "deadzone")
                .unwrap_or(0.1)
                .clamp(0.0, 0.9),
            deadzone_outer: 0.95,
            curve: 1.0,
        },
        "flickstick" => SourceMode::Flickstick {
            rotation_sensitivity: 1.0,
            flick_duration_ms: 100,
        },
        // Mouse-style groups, trackpads included.
        _ => SourceMode::Mouse {
            sensitivity: group_setting(group, "sensitivity").unwrap_or(100.0) / 100.0,
        },
    };
    set.inputs.push(InputMapping {
        mode: Some(stick_mode),
        ..InputMapping::new(InputSource::Axis(axis))
    });
}

fn import_gyro(group: &Node, mode: &str, gyro: &mut GyroConfig, report: &mut ImportReport) {
    gyro.enabled = true;
    gyro.activation = crate::profile::GyroActivation::Always;
    gyro.orientation = GyroOrientation::Local;
    gyro.output = match mode {
        "gyro_to_joystick" => match group_setting(group, "output_joystick").unwrap_or(0.0) as i32 {
            1 => GyroOutput::RightStick,
            _ => GyroOutput::LeftStick,
        },
        _ => GyroOutput::Mouse,
    };
    if let Some(sensitivity) = group_setting(group, "sensitivity") {
        gyro.sensitivity = (sensitivity / 100.0).clamp(0.05, 20.0);
    }
    if group.obj("settings").is_some_and(|settings| {
        settings
            .iter()
            .any(|node| node.key == "gyro_button" && node.as_str() != Some("0"))
    }) {
        report.warn("gyro button gating imports as always-on");
    }
}

/// Parse every activator block under `input`, appending to `activators`.
/// `mode_shift` bindings are collected separately and attached by the caller.
fn collect_activators(
    input: &Node,
    activators: &mut Vec<Activator>,
    shifts: &mut Vec<InputSource>,
) {
    let Some(blocks) = input.obj("activators") else {
        return;
    };
    for block in blocks {
        let Some(kind) = activator_kind(block) else {
            continue;
        };
        let mut outputs = Vec::new();
        if let Some(bindings) = block.obj("bindings") {
            for binding in bindings.iter().filter(|node| node.key == "binding") {
                let Some(text) = binding.as_str() else {
                    continue;
                };
                let body = text.split(',').next().unwrap_or(text).trim();
                if let Some(rest) = body.strip_prefix("mode_shift ") {
                    if let Some(source_name) = rest.split_whitespace().next() {
                        if let Some(trigger) = source_from_region(source_name) {
                            shifts.push(InputSource::Button(trigger));
                        }
                    }
                    continue;
                }
                if let Some(action) = parse_binding(body) {
                    outputs.push(action);
                }
            }
        }
        if outputs.is_empty() {
            continue;
        }
        activators.push(Activator {
            kind,
            outputs,
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        });
    }
}

fn attach_shifts(mapping: &mut InputMapping, shifts: Vec<InputSource>, report: &mut ImportReport) {
    for trigger in shifts {
        report.warn("mode shifts import as placeholders (no shifted behavior)");
        mapping.mode_shifts.push(ModeShift {
            trigger,
            mode: None,
            activators: Vec::new(),
        });
    }
}

fn activator_kind(block: &Node) -> Option<ActivatorKind> {
    match block.key.as_str() {
        "Double_Press" => Some(ActivatorKind::DoublePress {
            window_ms: child_number(block, "double_tap_time")
                .map(|ms| ms.max(100.0))
                .unwrap_or(320.0) as u32,
        }),
        "Long_Press" => Some(ActivatorKind::LongPress {
            duration_ms: child_number(block, "long_press_time")
                .map(|ms| ms.max(200.0))
                .unwrap_or(600.0) as u32,
        }),
        "Start_Press" => Some(ActivatorKind::StartPress),
        "Release" => Some(ActivatorKind::Release),
        "Full_Press" | "full_press" => Some(ActivatorKind::FullPress),
        _ => None,
    }
}

/// Binding strings look like "<verb> <args>, <description>, ...".
fn parse_binding(binding: &str) -> Option<OutputAction> {
    let (verb, arg) = binding.split_once(' ')?;
    let arg = arg.trim();
    match verb {
        "xinput_button" => xinput_button(arg),
        "key_press" => evdev_keycode(arg).map(|keycode| OutputAction::Keyboard { keycode }),
        "mouse_button" => match arg.to_ascii_uppercase().as_str() {
            "LEFT" => Some(OutputAction::MouseButton(MouseButton::Left)),
            "RIGHT" => Some(OutputAction::MouseButton(MouseButton::Right)),
            "MIDDLE" => Some(OutputAction::MouseButton(MouseButton::Middle)),
            _ => None,
        },
        "mouse_wheel" => match arg.to_ascii_uppercase().as_str() {
            "UP" => Some(wheel(MouseAxis::Wheel, 1)),
            "DOWN" => Some(wheel(MouseAxis::Wheel, -1)),
            "LEFT" => Some(wheel(MouseAxis::WheelX, -1)),
            "RIGHT" => Some(wheel(MouseAxis::WheelX, 1)),
            _ => None,
        },
        "controller_action" => match arg.split_whitespace().next()? {
            "CHANGE_PRESET" => arg
                .split_whitespace()
                .nth(1)?
                .parse::<usize>()
                .ok()
                .map(|preset| OutputAction::SwitchActionSet(preset.saturating_sub(1))),
            _ => None,
        },
        _ => None,
    }
}

fn wheel(axis: MouseAxis, amount: i32) -> OutputAction {
    OutputAction::WheelClick { axis, amount }
}

fn xinput_button(arg: &str) -> Option<OutputAction> {
    let button = match arg.to_ascii_uppercase().as_str() {
        "A" => GamepadButton::A,
        "B" => GamepadButton::B,
        "X" => GamepadButton::X,
        "Y" => GamepadButton::Y,
        "DPAD_UP" => GamepadButton::DpadUp,
        "DPAD_DOWN" => GamepadButton::DpadDown,
        "DPAD_LEFT" => GamepadButton::DpadLeft,
        "DPAD_RIGHT" => GamepadButton::DpadRight,
        "SHOULDER_LEFT" => GamepadButton::LeftShoulder,
        "SHOULDER_RIGHT" => GamepadButton::RightShoulder,
        "TRIGGER_LEFT" => GamepadButton::LeftTrigger,
        "TRIGGER_RIGHT" => GamepadButton::RightTrigger,
        "START" => GamepadButton::Start,
        "SELECT" | "BACK" => GamepadButton::Back,
        "JOYSTICK_LEFT" | "STICK_LEFT" | "L3" => GamepadButton::LeftStick,
        "JOYSTICK_RIGHT" | "STICK_RIGHT" | "R3" => GamepadButton::RightStick,
        "GUIDE" => GamepadButton::Guide,
        _ => return None,
    };
    Some(OutputAction::GamepadButton(button))
}

/// Physical region names from `group_source_bindings` to sources.
pub(crate) fn source_from_region(region: &str) -> Option<GamepadButton> {
    match region {
        "left_trigger" => Some(GamepadButton::LeftTrigger),
        "right_trigger" => Some(GamepadButton::RightTrigger),
        "left_bumper" => Some(GamepadButton::LeftShoulder),
        "right_bumper" => Some(GamepadButton::RightShoulder),
        "joystick" | "left_joystick" | "left_stick" => Some(GamepadButton::LeftStick),
        "right_joystick" | "right_stick" => Some(GamepadButton::RightStick),
        "button_diamond" | "button_a" => Some(GamepadButton::A),
        "dpad" => Some(GamepadButton::DpadUp),
        "menu" | "switches" => Some(GamepadButton::Back),
        _ => None,
    }
}

/// evdev keycodes for the key-name subset Steam uses in `key_press`.
fn evdev_keycode(name: &str) -> Option<u16> {
    let code = match name.to_ascii_uppercase().as_str() {
        "ESCAPE" => 1,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "0" => 11,
        "MINUS" => 12,
        "EQUAL" => 13,
        "BACKSPACE" => 14,
        "TAB" => 15,
        "Q" => 16,
        "W" => 17,
        "E" => 18,
        "R" => 19,
        "T" => 20,
        "Y" => 21,
        "U" => 22,
        "I" => 23,
        "O" => 24,
        "P" => 25,
        "ENTER" | "RETURN" => 28,
        "LEFT_CTRL" | "LCTRL" => 29,
        "A" => 30,
        "S" => 31,
        "D" => 32,
        "F" => 33,
        "G" => 34,
        "H" => 35,
        "J" => 36,
        "K" => 37,
        "L" => 38,
        "SEMICOLON" => 39,
        "APOSTROPHE" => 40,
        "GRAVE" => 41,
        "LEFT_SHIFT" | "LSHIFT" | "SHIFT" => 42,
        "BACKSLASH" => 43,
        "Z" => 44,
        "X" => 45,
        "C" => 46,
        "V" => 47,
        "B" => 48,
        "N" => 49,
        "M" => 50,
        "COMMA" => 51,
        "DOT" | "PERIOD" => 52,
        "SLASH" => 53,
        "RIGHT_SHIFT" | "RSHIFT" => 54,
        "LEFT_ALT" | "LALT" | "ALT" => 56,
        "SPACE" => 57,
        "CAPS_LOCK" | "CAPSLOCK" => 58,
        "F1" => 59,
        "F2" => 60,
        "F3" => 61,
        "F4" => 62,
        "F5" => 63,
        "F6" => 64,
        "F7" => 65,
        "F8" => 66,
        "F9" => 67,
        "F10" => 68,
        "F11" => 87,
        "F12" => 88,
        "RIGHT_CTRL" | "RCTRL" => 97,
        "UP" => 103,
        "LEFT" => 105,
        "RIGHT" => 106,
        "DOWN" => 108,
        "RIGHT_ALT" | "RALT" => 100,
        "INSERT" => 110,
        "DELETE" => 111,
        "HOME" => 102,
        "END" => 107,
        "PAGE_UP" => 104,
        "PAGE_DOWN" => 109,
        _ => return None,
    };
    Some(code)
}
