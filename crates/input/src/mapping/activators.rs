//! Activator state machines.
//!
//! Each input's activators are evaluated as one group: the press pattern
//! observed on the input (click, double click, long press, ...) decides which
//! activator fires, mirroring Steam Input. Pattern decisions need time to
//! pass (double-press windows, long-press durations), so the group advances
//! from both `process()` events and `tick()` advances, both of which carry
//! microsecond timestamps.

use std::collections::HashMap;

use super::OutputEvent;
use crate::profile::{
    Activator, ActivatorKind, ActivatorSettings, InputMapping, InputSource, OutputAction,
};

/// Press-pattern state for one input, keyed by source.
#[derive(Default)]
pub(crate) struct ActivatorStates {
    inner: HashMap<InputSource, PatternState>,
}

impl ActivatorStates {
    pub(crate) fn clear(&mut self) {
        self.inner.clear();
    }

    pub(super) fn sources(&self) -> impl Iterator<Item = InputSource> + '_ {
        self.inner.keys().copied()
    }

    pub(super) fn entry(&mut self, source: InputSource) -> &mut PatternState {
        self.inner.entry(source).or_default()
    }

    pub(super) fn get_mut(&mut self, source: InputSource) -> Option<&mut PatternState> {
        self.inner.get_mut(&source)
    }
}

#[derive(Default)]
pub(crate) struct PatternState {
    pressed_at: Option<u64>,
    /// Set while the input is held and its long-press activator has fired.
    long_fired: bool,
    /// Release happened inside the double-press window; a second press
    /// before the deadline fires the double activator.
    double_deadline: Option<u64>,
    /// The plain click is withheld until the double-press window expires.
    click_pending: bool,
    /// Outputs whose "press" is currently held, per activator index; they
    /// must be released even when the profile swaps underneath.
    held: Vec<(usize, OutputAction)>,
    /// Activator toggle state, activator index → on/off.
    toggles: HashMap<usize, bool>,
}

impl PatternState {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn held_mut(&mut self) -> &mut Vec<(usize, OutputAction)> {
        &mut self.held
    }
}

/// Result of running one input's activators.
#[derive(Default)]
pub(crate) struct ActivatorOutcome {
    pub outputs: Vec<OutputEvent>,
    /// Engine-internal actions that fired this round and must be applied by
    /// the action-set engine.
    pub internal: Vec<OutputAction>,
}

pub(super) fn push_release_of(output: &OutputAction, outputs: &mut Vec<OutputEvent>) {
    match output {
        OutputAction::GamepadButton(button) => outputs.push(OutputEvent::GamepadButton {
            button: *button,
            pressed: false,
        }),
        OutputAction::Keyboard { keycode } => outputs.push(OutputEvent::Key {
            keycode: *keycode,
            pressed: false,
        }),
        OutputAction::MouseButton(button) => outputs.push(OutputEvent::MouseButton {
            button: *button,
            pressed: false,
        }),
        _ => {}
    }
}

/// Trait-abstracted evaluator so the engine can drive it without lending
/// itself out twice: the caller provides activation gating and the engine
/// applies internal actions as they fire.
pub(crate) struct ActivatorRunner<'a> {
    pub(crate) mapping: &'a InputMapping,
    /// Whether the input's activation gating currently passes.
    pub(crate) active: bool,
    outcome: ActivatorOutcome,
}

impl<'a> ActivatorRunner<'a> {
    pub(crate) fn new(mapping: &'a InputMapping, active: bool) -> Self {
        Self {
            mapping,
            active,
            outcome: ActivatorOutcome::default(),
        }
    }

    pub(crate) fn finish(self) -> ActivatorOutcome {
        self.outcome
    }

    /// Feed a value change for the input.
    pub(crate) fn value_change(&mut self, state: &mut PatternState, value: f32, now_us: u64) {
        let pressed = value > super::BUTTON_THRESHOLD;
        let was_pressed = state.pressed_at.is_some();
        if pressed && !was_pressed {
            self.on_press(state, now_us);
        } else if !pressed && was_pressed {
            self.on_release(state, now_us);
        }
        self.expire_deadlines(state, now_us);
    }

    /// Advance time-based patterns (double windows, long press, repeats).
    /// Gating that switched off mid-hold releases everything immediately.
    pub(crate) fn advance(&mut self, state: &mut PatternState, now_us: u64) {
        if !self.active && (state.pressed_at.is_some() || !state.held.is_empty()) {
            self.release_all(state);
            return;
        }
        self.fire_long_press(state, now_us);
        self.expire_deadlines(state, now_us);
        self.repeat_held(state, now_us);
    }

    /// Release everything the input currently holds (gating turned off,
    /// profile swap, engine reset).
    pub(crate) fn release_all(&mut self, state: &mut PatternState) {
        let held = std::mem::take(&mut state.held);
        for (_, output) in held {
            push_release_of(&output, &mut self.outcome.outputs);
        }
        state.reset();
    }

    fn has_double(&self) -> bool {
        self.mapping
            .activators
            .iter()
            .any(|activator| matches!(activator.kind, ActivatorKind::DoublePress { .. }))
    }

    fn on_press(&mut self, state: &mut PatternState, now_us: u64) {
        state.pressed_at = Some(now_us);
        state.long_fired = false;
        if !self.active {
            state.double_deadline = None;
            state.click_pending = false;
            return;
        }
        if state.double_deadline.take().is_some() {
            // Second press inside the window: the double press wins and the
            // withheld click is dropped.
            state.click_pending = false;
            let mapping = self.mapping;
            for (index, activator) in double_activators(mapping) {
                self.fire(activator, state, index);
            }
            return;
        }
        for (index, activator) in self.mapping.activators.iter().enumerate() {
            match activator.kind {
                ActivatorKind::StartPress => self.fire(activator, state, index),
                ActivatorKind::FullPress if !self.has_double() => {
                    // With a double-press activator present the click must
                    // wait out the window; otherwise it fires on press.
                    self.fire(activator, state, index);
                }
                _ => {}
            }
        }
        if self.has_double() {
            state.click_pending = true;
        }
    }

    fn on_release(&mut self, state: &mut PatternState, now_us: u64) {
        let pressed_at = state.pressed_at.take().unwrap_or(now_us);
        let held_for = now_us.saturating_sub(pressed_at);
        self.release_kind(state, ActivatorKind::StartPress);
        if state.long_fired {
            self.release_kind(state, ActivatorKind::FullPress);
            self.release_kind(state, ActivatorKind::LongPress { duration_ms: 0 });
            state.long_fired = false;
            return;
        }
        if !self.active {
            state.click_pending = false;
            state.double_deadline = None;
            return;
        }
        if !self.has_double() {
            self.release_kind(state, ActivatorKind::FullPress);
        } else if state.click_pending && held_for < self.long_duration() {
            state.double_deadline = Some(now_us + self.double_window());
        }
        for (index, activator) in self.mapping.activators.iter().enumerate() {
            if matches!(activator.kind, ActivatorKind::Release) {
                self.fire(activator, state, index);
                self.release_activator(state, index);
            }
        }
    }

    /// A double-press window that expired without a second press promotes
    /// the withheld click to a full press.
    fn expire_deadlines(&mut self, state: &mut PatternState, now_us: u64) {
        if let Some(deadline) = state.double_deadline {
            if now_us >= deadline {
                state.double_deadline = None;
                if self.active && state.click_pending {
                    state.click_pending = false;
                    for (index, activator) in self.mapping.activators.iter().enumerate() {
                        if matches!(activator.kind, ActivatorKind::FullPress) {
                            self.fire(activator, state, index);
                            self.release_activator(state, index);
                        }
                    }
                }
            }
        }
    }

    fn fire_long_press(&mut self, state: &mut PatternState, now_us: u64) {
        let Some(pressed_at) = state.pressed_at else {
            return;
        };
        if state.long_fired || !self.active {
            return;
        }
        if now_us.saturating_sub(pressed_at) >= self.long_duration() {
            state.long_fired = true;
            for (index, activator) in self.mapping.activators.iter().enumerate() {
                if matches!(activator.kind, ActivatorKind::LongPress { .. }) {
                    self.fire(activator, state, index);
                }
            }
        }
    }

    fn repeat_held(&mut self, state: &mut PatternState, now_us: u64) {
        let Some(pressed_at) = state.pressed_at else {
            return;
        };
        for activator in &self.mapping.activators {
            let ActivatorSettings { repeat_rate_ms, .. } = activator.settings;
            let Some(rate_ms) = repeat_rate_ms else {
                continue;
            };
            if !matches!(activator.kind, ActivatorKind::FullPress) {
                continue;
            }
            let interval = u64::from(rate_ms) * 1_000;
            if now_us.saturating_sub(pressed_at) >= interval {
                // A repeat is a clean pulse; the hold from the original
                // press stays put and is released when the input releases.
                for output in &activator.outputs {
                    match output {
                        OutputAction::GamepadButton(button) => {
                            self.outcome.outputs.push(OutputEvent::GamepadButton {
                                button: *button,
                                pressed: true,
                            });
                            self.outcome.outputs.push(OutputEvent::GamepadButton {
                                button: *button,
                                pressed: false,
                            });
                        }
                        OutputAction::Keyboard { keycode } => {
                            self.outcome.outputs.push(OutputEvent::Key {
                                keycode: *keycode,
                                pressed: true,
                            });
                            self.outcome.outputs.push(OutputEvent::Key {
                                keycode: *keycode,
                                pressed: false,
                            });
                        }
                        OutputAction::MouseButton(button) => {
                            self.outcome.outputs.push(OutputEvent::MouseButton {
                                button: *button,
                                pressed: true,
                            });
                            self.outcome.outputs.push(OutputEvent::MouseButton {
                                button: *button,
                                pressed: false,
                            });
                        }
                        OutputAction::SwitchActionSet(_)
                        | OutputAction::EnableLayer { .. }
                        | OutputAction::ModeShiftActivate { .. } => {
                            self.outcome.internal.push(output.clone());
                        }
                        OutputAction::GamepadAxis(_) | OutputAction::MouseAxis(_) => {}
                    }
                }
                // Restart the hold clock so the next repeat waits a full
                // interval from here.
                state.pressed_at = Some(now_us);
                return;
            }
        }
    }

    /// Fire an activator's outputs, honoring toggle semantics.
    fn fire(&mut self, activator: &Activator, state: &mut PatternState, index: usize) {
        if activator.settings.toggle {
            let enabled = state.toggles.entry(index).or_insert(false);
            *enabled = !*enabled;
            if *enabled {
                self.hold_outputs(activator, state, index);
            } else {
                self.release_activator(state, index);
            }
            return;
        }
        self.hold_outputs(activator, state, index);
    }

    /// Hold semantics per output type: buttons/keys/mouse buttons stay
    /// pressed until released; engine-internal actions fire once now.
    fn hold_outputs(&mut self, activator: &Activator, state: &mut PatternState, index: usize) {
        for output in &activator.outputs {
            if state
                .held
                .iter()
                .any(|(held_index, held)| *held_index == index && held == output)
            {
                continue;
            }
            match output {
                OutputAction::GamepadButton(button) => {
                    self.outcome.outputs.push(OutputEvent::GamepadButton {
                        button: *button,
                        pressed: true,
                    });
                    state.held.push((index, output.clone()));
                }
                OutputAction::Keyboard { keycode } => {
                    self.outcome.outputs.push(OutputEvent::Key {
                        keycode: *keycode,
                        pressed: true,
                    });
                    state.held.push((index, output.clone()));
                }
                OutputAction::MouseButton(button) => {
                    self.outcome.outputs.push(OutputEvent::MouseButton {
                        button: *button,
                        pressed: true,
                    });
                    state.held.push((index, output.clone()));
                }
                OutputAction::SwitchActionSet(_)
                | OutputAction::EnableLayer { .. }
                | OutputAction::ModeShiftActivate { .. } => {
                    self.outcome.internal.push(output.clone());
                }
                OutputAction::GamepadAxis(_) | OutputAction::MouseAxis(_) => {}
            }
        }
    }

    /// Release one activator's held outputs (idempotent).
    fn release_activator(&mut self, state: &mut PatternState, index: usize) {
        let held = std::mem::take(&mut state.held);
        for (held_index, output) in held {
            if held_index == index {
                push_release_of(&output, &mut self.outcome.outputs);
            } else {
                state.held.push((held_index, output));
            }
        }
    }

    fn release_kind(&mut self, state: &mut PatternState, kind: ActivatorKind) {
        for (index, activator) in self.mapping.activators.iter().enumerate() {
            if std::mem::discriminant(&activator.kind) == std::mem::discriminant(&kind)
                // Toggled-on activators persist until toggled off again.
                && !activator.settings.toggle
            {
                self.release_activator(state, index);
            }
        }
    }

    fn double_window(&self) -> u64 {
        self.mapping
            .activators
            .iter()
            .find_map(|activator| match activator.kind {
                ActivatorKind::DoublePress { window_ms } => Some(u64::from(window_ms) * 1_000),
                _ => None,
            })
            .unwrap_or(320_000)
    }

    fn long_duration(&self) -> u64 {
        self.mapping
            .activators
            .iter()
            .find_map(|activator| match activator.kind {
                ActivatorKind::LongPress { duration_ms } => Some(u64::from(duration_ms) * 1_000),
                _ => None,
            })
            .unwrap_or(600_000)
    }
}

fn double_activators(mapping: &InputMapping) -> impl Iterator<Item = (usize, &Activator)> {
    mapping.activators.iter().enumerate().filter(|(_, activator)| {
        matches!(activator.kind, ActivatorKind::DoublePress { .. })
    })
}

#[cfg(test)]
mod tests {
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
        assert_eq!(outcome.outputs, vec![button_output(true), button_output(false)]);
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
}
