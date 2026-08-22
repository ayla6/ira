//! Action set engine: resolves which mapping applies to a source given the
//! active set, toggled/held layers, and held mode-shift triggers, and applies
//! engine-internal actions (set switches, layer toggles).

use super::activators::{push_release_of, ActivatorRunner};
use super::{MappingEngine, OutputEvent, BUTTON_THRESHOLD};
use crate::profile::{ChordMode, InputMapping, ModeShift, OutputAction};

impl MappingEngine {
    /// The mapping that currently applies to `source`: active layers
    /// override their parent set (child wins), mode shifts override both
    /// while their trigger button is held.
    pub(crate) fn resolve_mapping(&self, source: InputSource) -> Option<&InputMapping> {
        let set = self.profile.action_sets.get(self.active_set)?;
        for layer_index in self.active_layer_indexes() {
            if let Some(layer) = self
                .profile
                .action_layers
                .get(layer_index)
                .filter(|layer| layer.parent_set == set.name)
            {
                if let Some(input) = layer.inputs.iter().find(|input| input.source == source) {
                    return Some(input);
                }
            }
        }
        set.inputs.iter().find(|input| input.source == source)
    }

    /// The mode-shift currently overriding `input`, if its trigger is held.
    pub(crate) fn active_shift<'a>(&self, input: &'a InputMapping) -> Option<&'a ModeShift> {
        input
            .mode_shifts
            .iter()
            .find(|shift| self.source_value(shift.trigger) > BUTTON_THRESHOLD)
    }

    /// Toggled layers plus hold-mode layers whose firing button is down.
    pub(crate) fn active_layer_indexes(&self) -> Vec<usize> {
        let mut layers = self.toggled_layers.clone();
        for (index, _) in self.profile.action_layers.iter().enumerate() {
            if layers.contains(&index) {
                continue;
            }
            let held = self
                .profile
                .action_sets
                .iter()
                .flat_map(|set| set.inputs.iter())
                .any(|input| {
                    self.source_value(input.source) > BUTTON_THRESHOLD
                        && input.activators.iter().any(|activator| {
                            activator.outputs.iter().any(|output| {
                                matches!(
                                    output,
                                    OutputAction::EnableLayer {
                                        layer: target,
                                        mode: ChordMode::Hold,
                                    } if target == &index
                                )
                            })
                        })
                });
            if held {
                layers.push(index);
            }
        }
        layers
    }

    /// Apply engine-internal actions fired by activators.
    pub(crate) fn apply_internal_actions(&mut self, actions: &[OutputAction]) {
        for action in actions {
            match action {
                OutputAction::SwitchActionSet(index) => self.switch_action_set(*index),
                OutputAction::EnableLayer { layer, mode } => {
                    if *mode == ChordMode::Toggle && self.toggled_layers.contains(layer) {
                        self.toggled_layers.retain(|candidate| candidate != layer);
                        self.release_all_activator_states();
                    } else if *mode == ChordMode::Toggle {
                        self.toggled_layers.push(*layer);
                        self.release_all_activator_states();
                    }
                    // Hold layers evaluate from button state in
                    // active_layer_indexes; deactivation releases below.
                }
                // Mode shifts evaluate directly from held trigger buttons in
                // resolve_mapping; nothing to arm here.
                OutputAction::ModeShiftActivate { .. } => {}
                _ => {}
            }
        }
    }

    fn switch_action_set(&mut self, index: usize) {
        if index >= self.profile.action_sets.len() || index == self.active_set {
            return;
        }
        self.active_set = index;
        self.toggled_layers.clear();
        self.release_all_activator_states();
    }

    /// Release every held activator output (set switch, layer change);
    /// released events land in `pending_releases`.
    pub(crate) fn release_all_activator_states(&mut self) {
        let sources: Vec<_> = self.activator_states.sources().collect();
        for source in sources {
            if let Some(state) = self.activator_states.get_mut(source) {
                for (_, output) in std::mem::take(state.held_mut()) {
                    push_release_of(&output, &mut self.pending_releases);
                }
                state.reset();
            }
        }
    }

    /// Drain outputs released by set/layer changes; callers merge them into
    /// the event stream.
    pub(crate) fn take_pending_releases(&mut self) -> Vec<OutputEvent> {
        std::mem::take(&mut self.pending_releases)
    }

    /// Run activators for one resolved input after a value change.
    pub(crate) fn run_activators(
        &mut self,
        source: InputSource,
        value: f32,
        now_us: u64,
    ) -> Vec<OutputEvent> {
        let Some(mapping) = self.resolve_mapping(source).cloned() else {
            return Vec::new();
        };
        let effective = effective_mapping(&mapping, self.active_shift(&mapping).cloned());
        let active = effective
            .activators
            .iter()
            .all(|activator| self.activation_active(&activator.activation));
        let mut runner = ActivatorRunner::new(&effective, active);
        runner.value_change(self.activator_states.entry(source), value, now_us);
        let outcome = runner.finish();
        self.apply_internal_actions(&outcome.internal);
        outcome.outputs
    }

    /// Advance time-based activator patterns for every input in the active
    /// set plus its layers; called from tick().
    pub(crate) fn advance_set_activators(&mut self, now_us: u64) -> Vec<OutputEvent> {
        let mut outputs = Vec::new();
        let mut mappings: Vec<InputMapping> = Vec::new();
        if let Some(set) = self.profile.action_sets.get(self.active_set) {
            mappings.extend(set.inputs.iter().cloned());
        }
        for index in self.active_layer_indexes() {
            if let Some(layer) = self.profile.action_layers.get(index) {
                mappings.extend(layer.inputs.iter().cloned());
            }
        }
        for mapping in mappings {
            if mapping.activators.is_empty() {
                continue;
            }
            let effective = effective_mapping(&mapping, self.active_shift(&mapping).cloned());
            let active = effective
                .activators
                .iter()
                .all(|activator| self.activation_active(&activator.activation));
            let mut runner = ActivatorRunner::new(&effective, active);
            runner.advance(self.activator_states.entry(effective.source), now_us);
            let outcome = runner.finish();
            self.apply_internal_actions(&outcome.internal);
            outputs.extend(outcome.outputs);
        }
        outputs
    }
}

use crate::profile::InputSource;

/// Mode shifts replace the mapping's mode and activators while held.
fn effective_mapping(
    mapping: &InputMapping,
    shift: Option<ModeShift>,
) -> std::borrow::Cow<'_, InputMapping> {
    match shift {
        None => std::borrow::Cow::Borrowed(mapping),
        Some(shift) => std::borrow::Cow::Owned(InputMapping {
            source: mapping.source,
            mode: shift.mode.clone().or_else(|| mapping.mode.clone()),
            mode_shifts: Vec::new(),
            activators: if shift.activators.is_empty() {
                mapping.activators.clone()
            } else {
                shift.activators
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{InputEvent, MappingEngine, OutputEvent};
    use crate::profile::{
        Activator, ActivatorKind, ActivatorSettings, ActionSet, ActionSetLayer, Activation,
        Binding, GamepadAxis, GamepadButton, GyroConfig, InputMapping, InputProfile, InputSource,
        OutputAction,
    };

    fn profile_with_sets(sets: Vec<ActionSet>, layers: Vec<ActionSetLayer>) -> InputProfile {
        InputProfile {
            action_sets: sets,
            action_layers: layers,
            ..InputProfile::default()
        }
    }

    fn press(source: InputSource, value: f32, at_us: u64) -> InputEvent {
        InputEvent {
            source,
            value,
            timestamp_us: at_us,
        }
    }

    #[test]
    fn test_set_path_maps_button_through_activators() {
        let profile = profile_with_sets(
            vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadButton(GamepadButton::B),
                )],
            }],
            Vec::new(),
        );
        let mut engine = MappingEngine::new(profile).unwrap();
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 2_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: false
            }]
        );
    }

    #[test]
    fn test_activator_switches_action_set() {
        let make_set = |name: &str, output: GamepadButton| ActionSet {
            name: name.to_string(),
            inputs: vec![InputMapping::simple(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(output),
            )],
        };
        let switcher = InputMapping {
            source: InputSource::Button(GamepadButton::X),
            mode: None,
            mode_shifts: Vec::new(),
            activators: vec![Activator::full_press(vec![OutputAction::SwitchActionSet(1)])],
        };
        let mut first = make_set("On foot", GamepadButton::B);
        first.inputs.push(switcher);
        let profile = profile_with_sets(vec![first, make_set("In car", GamepadButton::Y)], Vec::new());
        let mut engine = MappingEngine::new(profile).unwrap();

        // Set 0: A -> B.
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 2_000));
        // Switch to set 1: A -> Y now. The held B from set 0 would be
        // released bluntly; A was already released.
        engine.process(press(InputSource::Button(GamepadButton::X), 1.0, 3_000));
        engine.process(press(InputSource::Button(GamepadButton::X), 0.0, 4_000));
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 5_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::Y,
                pressed: true
            }]
        );
    }

    #[test]
    fn test_layer_overrides_parent_mapping() {
        let base = ActionSet {
            name: "Default".to_string(),
            inputs: vec![InputMapping::simple(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::B),
            )],
        };
        let layer = ActionSetLayer {
            name: "Menus".to_string(),
            parent_set: "Default".to_string(),
            inputs: vec![InputMapping::simple(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::X),
            )],
        };
        let mut toggler = InputMapping::new(InputSource::Button(GamepadButton::Guide));
        toggler.activators.push(Activator {
            kind: ActivatorKind::FullPress,
            outputs: vec![OutputAction::EnableLayer {
                layer: 0,
                mode: ChordMode::Hold,
            }],
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        });
        let mut base = base;
        base.inputs.push(toggler);
        let mut engine = MappingEngine::new(profile_with_sets(vec![base], vec![layer])).unwrap();

        // Without the layer: A -> B.
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 2_000));
        // Hold the layer button: A -> X.
        engine.process(press(InputSource::Button(GamepadButton::Guide), 1.0, 3_000));
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 4_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::X,
                pressed: true
            }]
        );
        // Release the layer button: back to B.
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 5_000));
        engine.process(press(InputSource::Button(GamepadButton::Guide), 0.0, 6_000));
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 7_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
    }

    #[test]
    fn test_mode_shift_overrides_while_trigger_held() {
        let shifted = crate::profile::ModeShift {
            trigger: InputSource::Button(GamepadButton::LeftTrigger),
            mode: None,
            activators: vec![Activator::full_press(vec![
                OutputAction::GamepadButton(GamepadButton::Y),
            ])],
        };
        let mut input = InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::B),
        );
        input.mode_shifts.push(shifted);
        let profile = profile_with_sets(
            vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![input],
            }],
            Vec::new(),
        );
        let mut engine = MappingEngine::new(profile).unwrap();

        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 2_000));
        engine.process(press(InputSource::Button(GamepadButton::LeftTrigger), 1.0, 3_000));
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 4_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::Y,
                pressed: true
            }]
        );
    }

    #[test]
    fn test_bindings_path_unchanged_when_no_sets() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::B),
            )],
            gyro: GyroConfig::default(),
            ..InputProfile::default()
        };
        let mut engine = MappingEngine::new(profile).unwrap();
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000)),
            vec![OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }]
        );
    }

    #[test]
    fn test_set_input_with_double_press_via_tick_expiry() {
        let mut input = InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::B),
        );
        input.activators.push(Activator {
            kind: ActivatorKind::DoublePress { window_ms: 250 },
            outputs: vec![OutputAction::Keyboard { keycode: 30 }],
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        });
        let profile = profile_with_sets(
            vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![input],
            }],
            Vec::new(),
        );
        let mut engine = MappingEngine::new(profile).unwrap();
        engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 1_000));
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 20_000));
        // The withheld click promotes once the window expires in tick().
        let events = engine.tick(400_000);
        assert!(events.iter().any(|event| matches!(
            event,
            OutputEvent::GamepadButton {
                button: GamepadButton::B,
                pressed: true
            }
        )));
        let _ = GamepadAxis::LeftX;
    }

    #[test]
    fn test_analog_activation_gates_on_axis_state() {
        let mut input = InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::Keyboard { keycode: 32 },
        );
        input.activators[0].activation = Activation::Analog {
            axis: GamepadAxis::LeftTrigger,
            condition: crate::profile::AnalogCondition::Active,
            threshold: 0.3,
        };
        let profile = profile_with_sets(
            vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![input],
            }],
            Vec::new(),
        );
        let mut engine = MappingEngine::new(profile).unwrap();

        // Gate closed (trigger at rest): the press produces nothing.
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 2_000)),
            Vec::<OutputEvent>::new()
        );
        engine.process(press(InputSource::Button(GamepadButton::A), 0.0, 3_000));
        // Trigger held: gate open, the same press fires.
        engine.process(press(InputSource::Axis(GamepadAxis::LeftTrigger), 0.8, 4_000));
        assert_eq!(
            engine.process(press(InputSource::Button(GamepadButton::A), 1.0, 5_000)),
            vec![OutputEvent::Key {
                keycode: 32,
                pressed: true
            }]
        );
        // Gate flips off mid-hold: the next tick releases the held key.
        engine.process(press(InputSource::Axis(GamepadAxis::LeftTrigger), 0.0, 6_000));
        assert_eq!(
            engine.tick(7_000),
            vec![OutputEvent::Key {
                keycode: 32,
                pressed: false
            }]
        );
    }

    #[test]
    fn test_analog_activation_conditions() {
        let mut engine = MappingEngine::new(InputProfile::default()).unwrap();
        let at_rest = Activation::Analog {
            axis: GamepadAxis::RightTrigger,
            condition: crate::profile::AnalogCondition::AtRest,
            threshold: 0.1,
        };
        let maxed = Activation::Analog {
            axis: GamepadAxis::RightTrigger,
            condition: crate::profile::AnalogCondition::MaxedOut,
            threshold: 0.1,
        };
        engine.process(press(InputSource::Axis(GamepadAxis::RightTrigger), 0.95, 1_000));
        assert!(!engine.activation_active(&at_rest));
        assert!(engine.activation_active(&maxed));
        engine.process(press(InputSource::Axis(GamepadAxis::RightTrigger), 0.0, 2_000));
        assert!(engine.activation_active(&at_rest));
        assert!(!engine.activation_active(&maxed));
    }
}
