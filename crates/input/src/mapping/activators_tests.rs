use super::*;
use crate::profile::{Activation, ActivatorSettings};

fn mapping_with(activators: Vec<Activator>) -> InputMapping {
    InputMapping {
        source: InputSource::Button(crate::GamepadButton::A),
        mode: None,
        mode_shifts: Vec::new(),
        activators,
    }
}

fn button_output(pressed: bool) -> OutputEvent {
    OutputEvent::GamepadButton {
        button: crate::GamepadButton::B,
        pressed,
    }
}

#[test]
fn test_plain_click_fires_full_press_on_press_and_release() {
    let mapping = mapping_with(vec![Activator::full_press(vec![
        OutputAction::GamepadButton(crate::GamepadButton::B),
    ])]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    let outcome = runner.finish();
    assert_eq!(outcome.outputs, vec![button_output(true)]);

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.0, 50_000);
    assert_eq!(runner.finish().outputs, vec![button_output(false)]);
}

#[test]
fn test_double_press_fires_double_and_cancels_click() {
    let mapping = mapping_with(vec![
        Activator::full_press(vec![OutputAction::GamepadButton(crate::GamepadButton::B)]),
        Activator {
            kind: ActivatorKind::DoublePress { window_ms: 300 },
            outputs: vec![OutputAction::Keyboard { keycode: 30 }],
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        },
    ]);
    let mut states = ActivatorStates::default();
    // Press and release quickly; nothing fires yet.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.value_change(states.entry(mapping.source), 0.0, 30_000);
    assert!(runner.finish().outputs.is_empty());

    // Second press inside the window fires the double activator.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 100_000);
    let outcome = runner.finish();
    assert_eq!(
        outcome.outputs,
        vec![OutputEvent::Key {
            keycode: 30,
            pressed: true
        }]
    );
}

#[test]
fn test_slow_double_press_promotes_click_after_window() {
    let mapping = mapping_with(vec![
        Activator::full_press(vec![OutputAction::GamepadButton(crate::GamepadButton::B)]),
        Activator {
            kind: ActivatorKind::DoublePress { window_ms: 300 },
            outputs: vec![OutputAction::Keyboard { keycode: 30 }],
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        },
    ]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.value_change(states.entry(mapping.source), 0.0, 30_000);
    runner.finish();

    // Window expires with no second press: the click fires as a pulse.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.advance(states.entry(mapping.source), 500_000);
    let outcome = runner.finish();
    assert_eq!(
        outcome.outputs,
        vec![button_output(true), button_output(false)]
    );
}

#[test]
fn test_long_press_fires_while_held() {
    let mapping = mapping_with(vec![
        Activator::full_press(vec![OutputAction::GamepadButton(crate::GamepadButton::B)]),
        Activator {
            kind: ActivatorKind::LongPress { duration_ms: 200 },
            outputs: vec![OutputAction::Keyboard { keycode: 42 }],
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        },
    ]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.finish();

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.advance(states.entry(mapping.source), 250_000);
    let outcome = runner.finish();
    assert_eq!(
        outcome.outputs,
        vec![OutputEvent::Key {
            keycode: 42,
            pressed: true
        }]
    );

    // Release ends both the full press and the long press.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.0, 260_000);
    let mut outcome = runner.finish();
    outcome.outputs.sort_by_key(|event| match event {
        OutputEvent::GamepadButton { pressed, .. } => u8::from(!*pressed),
        _ => 1,
    });
    assert_eq!(outcome.outputs.len(), 2);
}

#[test]
fn test_toggle_activator_presses_and_releases_on_reactivation() {
    let mapping = mapping_with(vec![Activator {
        kind: ActivatorKind::FullPress,
        outputs: vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
        activation: Activation::Always,
        settings: ActivatorSettings {
            repeat_rate_ms: None,
            toggle: true,
            interruptable: true,
        },
    }]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.value_change(states.entry(mapping.source), 0.0, 50_000);
    assert_eq!(runner.finish().outputs, vec![button_output(true)]);

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 100_000);
    runner.value_change(states.entry(mapping.source), 0.0, 150_000);
    assert_eq!(runner.finish().outputs, vec![button_output(false)]);
}

#[test]
fn test_release_activator_fires_pulse_on_release() {
    let mapping = mapping_with(vec![Activator {
        kind: ActivatorKind::Release,
        outputs: vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
        activation: Activation::Always,
        settings: ActivatorSettings::default(),
    }]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.finish();

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.0, 50_000);
    assert_eq!(
        runner.finish().outputs,
        vec![button_output(true), button_output(false)]
    );
}

#[test]
fn test_inactive_input_releases_held_outputs() {
    let mapping = mapping_with(vec![Activator::full_press(vec![
        OutputAction::GamepadButton(crate::GamepadButton::B),
    ])]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    runner.finish();

    // Gating turns off while held: the press must be released.
    let mut runner = ActivatorRunner::new(&mapping, false);
    runner.advance(states.entry(mapping.source), 10_000);
    assert_eq!(runner.finish().outputs, vec![button_output(false)]);
}

#[test]
fn test_repeat_rate_refires_while_held() {
    let mapping = mapping_with(vec![Activator {
        kind: ActivatorKind::FullPress,
        outputs: vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
        activation: Activation::Always,
        settings: ActivatorSettings {
            repeat_rate_ms: Some(100),
            toggle: false,
            interruptable: true,
        },
    }]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    assert_eq!(runner.finish().outputs, vec![button_output(true)]);

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.advance(states.entry(mapping.source), 150_000);
    let outcome = runner.finish();
    assert_eq!(
        outcome.outputs,
        vec![button_output(true), button_output(false)]
    );
}

#[test]
fn test_internal_actions_are_reported_not_emitted() {
    let mapping = mapping_with(vec![Activator::full_press(vec![
        OutputAction::SwitchActionSet(1),
    ])]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 1_000);
    let outcome = runner.finish();
    assert!(outcome.outputs.is_empty());
    assert_eq!(outcome.internal, vec![OutputAction::SwitchActionSet(1)]);
}

fn soft_activator(threshold: f32, outputs: Vec<OutputAction>) -> Activator {
    Activator {
        kind: ActivatorKind::SoftPress { threshold },
        outputs,
        activation: Activation::Always,
        settings: ActivatorSettings::default(),
    }
}

#[test]
fn test_soft_press_fires_and_releases_at_threshold() {
    let mapping = mapping_with(vec![soft_activator(
        0.4,
        vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
    )]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.3, 1_000);
    assert!(runner.finish().outputs.is_empty());

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.45, 2_000);
    assert_eq!(runner.finish().outputs, vec![button_output(true)]);

    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.35, 3_000);
    assert_eq!(runner.finish().outputs, vec![button_output(false)]);
}

#[test]
fn test_dual_stage_soft_pull_precedes_full_press() {
    let mapping = mapping_with(vec![
        soft_activator(
            0.4,
            vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
        ),
        Activator::full_press(vec![OutputAction::Keyboard { keycode: 30 }]),
    ]);
    let mut states = ActivatorStates::default();
    // Into the soft stage, still below the digitalized click.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.45, 1_000);
    assert_eq!(runner.finish().outputs, vec![button_output(true)]);

    // Through to full pull: the click fires alongside the held soft pull.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 2_000);
    assert_eq!(
        runner.finish().outputs,
        vec![OutputEvent::Key {
            keycode: 30,
            pressed: true
        }]
    );

    // Releasing everything lets go of both stages.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.0, 3_000);
    assert_eq!(
        runner.finish().outputs,
        vec![
            button_output(false),
            OutputEvent::Key {
                keycode: 30,
                pressed: false
            }
        ]
    );
}

#[test]
fn test_deep_soft_pull_pushes_full_press_threshold_up() {
    let mapping = mapping_with(vec![
        soft_activator(
            0.8,
            vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
        ),
        Activator::full_press(vec![OutputAction::Keyboard { keycode: 30 }]),
    ]);
    let mut states = ActivatorStates::default();
    // Past the plain button threshold but still short of the digitalized
    // full pull (0.8 + margin): neither stage may fire.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.6, 1_000);
    assert!(runner.finish().outputs.is_empty());

    // Full travel fires soft first, then the click.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 1.0, 2_000);
    assert_eq!(
        runner.finish().outputs,
        vec![
            button_output(true),
            OutputEvent::Key {
                keycode: 30,
                pressed: true
            }
        ]
    );
}

#[test]
fn test_soft_press_toggle_persists_below_threshold() {
    let mut activator = soft_activator(
        0.4,
        vec![OutputAction::GamepadButton(crate::GamepadButton::B)],
    );
    activator.settings.toggle = true;
    let mapping = mapping_with(vec![activator]);
    let mut states = ActivatorStates::default();
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.45, 1_000);
    assert_eq!(runner.finish().outputs, vec![button_output(true)]);

    // Falling below the threshold leaves the toggle latched on.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.2, 2_000);
    assert!(runner.finish().outputs.is_empty());

    // Crossing again flips the toggle off.
    let mut runner = ActivatorRunner::new(&mapping, true);
    runner.value_change(states.entry(mapping.source), 0.45, 3_000);
    assert_eq!(runner.finish().outputs, vec![button_output(false)]);
}
