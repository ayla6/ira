//! Action set / layer management on [`InputProfile`]: renames and removals
//! that keep the rest of the profile consistent. Outputs reference sets and
//! layers positionally (`SwitchActionSet(n)`, `EnableLayer { layer }`) and
//! layers parent by name, so every structural edit has to cascade — removing
//! a set shifts later set indices, removes its layers (shifting later layer
//! indices), and any surviving binding that pointed at something removed
//! would fail validation otherwise.

use super::{ActionSet, InputMapping, InputProfile, OutputAction};

impl InputProfile {
    /// Whether the session should expose the physical controller itself over
    /// uhid instead of a virtual uinput pad, so the game talks to native
    /// hardware (and its sensors) through the real driver. The explicit
    /// switch and the gyro's "Native motion sensors" output both opt in.
    pub fn wants_native_controller(&self) -> bool {
        self.native_motion
            || (self.gyro.enabled && self.gyro.output == super::GyroOutput::NativeMotion)
    }

    /// Renames a set, keeping its layers parented to it (layers reference
    /// their parent by name).
    pub fn rename_action_set(&mut self, index: usize, name: String) {
        let Some(old) = self.action_sets.get_mut(index) else {
            return;
        };
        let old_name = std::mem::replace(&mut old.name, name.clone());
        if old_name == name {
            return;
        }
        for layer in &mut self.action_layers {
            if layer.parent_set == old_name {
                layer.parent_set = name.clone();
            }
        }
    }

    /// Removes a set and everything bound to it: its layers,
    /// [`OutputAction::SwitchActionSet`] outputs pointing at it, cursor
    /// switching preferences naming it, and shifts later indices in
    /// surviving outputs. The first set is the default and cannot go away.
    pub fn remove_action_set(&mut self, index: usize) {
        if index == 0 || index >= self.action_sets.len() {
            return;
        }
        let name = self.action_sets.remove(index).name;
        let mut original_index = 0;
        let mut removed_layers = Vec::new();
        self.action_layers.retain(|layer| {
            let remove = layer.parent_set == name;
            if remove {
                removed_layers.push(original_index);
            } else {
                original_index += 1;
            }
            !remove
        });
        let removed_layers = removed_layers;
        self.for_every_output(&mut |output| match output {
            OutputAction::SwitchActionSet(target) => {
                if *target == index {
                    return false;
                }
                if *target > index {
                    *target -= 1;
                }
                true
            }
            OutputAction::EnableLayer { layer, .. } => {
                if removed_layers.contains(layer) {
                    return false;
                }
                let shift_down = removed_layers
                    .iter()
                    .filter(|removed| **removed < *layer)
                    .count();
                *layer -= shift_down;
                true
            }
            _ => true,
        });
        for target in [
            &mut self.action_set_when_cursor_shown,
            &mut self.action_set_when_cursor_hidden,
        ] {
            match *target {
                Some(removed) if removed == index => *target = None,
                Some(later) if later > index => *target = Some(later - 1),
                _ => {}
            }
        }
    }

    /// Removes one layer and shifts later indices in
    /// [`OutputAction::EnableLayer`] outputs.
    pub fn remove_action_layer(&mut self, index: usize) {
        if index >= self.action_layers.len() {
            return;
        }
        self.action_layers.remove(index);
        self.for_every_output(&mut |output| match output {
            OutputAction::EnableLayer { layer, .. } => {
                if *layer == index {
                    return false;
                }
                if *layer > index {
                    *layer -= 1;
                }
                true
            }
            _ => true,
        });
    }

    /// Whether `name` is free for a new action set.
    pub fn is_free_set_name(&self, name: &str) -> bool {
        self.action_sets.iter().all(|set| set.name != name)
    }

    /// Whether `name` is free for a new action layer.
    pub fn is_free_layer_name(&self, name: &str) -> bool {
        self.action_layers.iter().all(|layer| layer.name != name)
    }

    /// Visits every output in the profile — activators and mode shifts of
    /// every mapping in every set and layer. Returning `false` drops the
    /// output; emptied activators and then their mappings are removed so the
    /// profile keeps validating.
    fn for_every_output(&mut self, visit: &mut dyn FnMut(&mut OutputAction) -> bool) {
        let input_lists = self
            .action_sets
            .iter_mut()
            .map(|set: &mut ActionSet| &mut set.inputs)
            .chain(self.action_layers.iter_mut().map(|layer| &mut layer.inputs));
        for inputs in input_lists {
            for mapping_index in (0..inputs.len()).rev() {
                if !fix_mapping_outputs(&mut inputs[mapping_index], visit) {
                    inputs.remove(mapping_index);
                }
            }
        }
    }
}

/// Applies `visit` to every output of one mapping. Returns whether the
/// mapping still has a reason to exist: a mode, or at least one activator.
fn fix_mapping_outputs(
    mapping: &mut InputMapping,
    visit: &mut dyn FnMut(&mut OutputAction) -> bool,
) -> bool {
    for activator in &mut mapping.activators {
        activator.outputs.retain_mut(|output| visit(output));
    }
    // Validation demands one or more outputs per activator; an activator
    // whose only output pointed at the removed set is gone instead.
    mapping
        .activators
        .retain(|activator| !activator.outputs.is_empty());
    for shift in &mut mapping.mode_shifts {
        shift
            .activators
            .retain_mut(|activator: &mut super::Activator| {
                activator.outputs.retain_mut(|output| visit(output));
                !activator.outputs.is_empty()
            });
    }
    mapping.mode.is_some() || !mapping.activators.is_empty()
}

#[cfg(test)]
mod tests {
    use super::super::{ActionSet, ActionSetLayer, ChordMode, GamepadButton, InputSource};
    use super::*;

    fn set(name: &str, inputs: Vec<InputMapping>) -> ActionSet {
        ActionSet {
            name: name.to_string(),
            inputs,
        }
    }

    fn layer(name: &str, parent: &str) -> ActionSetLayer {
        ActionSetLayer {
            name: name.to_string(),
            parent_set: parent.to_string(),
            inputs: Vec::new(),
        }
    }

    fn switch(target: usize) -> InputMapping {
        InputMapping::simple(
            InputSource::Button(GamepadButton::A),
            OutputAction::SwitchActionSet(target),
        )
    }

    fn enable_layer(target: usize) -> InputMapping {
        InputMapping::simple(
            InputSource::Button(GamepadButton::B),
            OutputAction::EnableLayer {
                layer: target,
                mode: ChordMode::Toggle,
            },
        )
    }

    fn layer_targets(profile: &InputProfile) -> Vec<usize> {
        profile
            .all_activator_outputs()
            .filter_map(|output| match output {
                OutputAction::EnableLayer { layer, .. } => Some(*layer),
                _ => None,
            })
            .collect()
    }

    fn set_targets(profile: &InputProfile) -> Vec<usize> {
        profile
            .all_activator_outputs()
            .filter_map(|output| match output {
                OutputAction::SwitchActionSet(target) => Some(*target),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_remove_action_set_removes_its_layers_and_fixes_indices() {
        let mut profile = InputProfile {
            action_sets: vec![
                set("Default", vec![enable_layer(1), switch(2)]),
                set("Menus", vec![switch(2)]),
                set("Driving", vec![switch(1)]),
            ],
            action_layers: vec![
                layer("Menus layer", "Menus"),
                layer("Driving layer", "Driving"),
            ],
            ..InputProfile::default()
        };
        profile.remove_action_set(1);
        assert_eq!(
            profile
                .action_sets
                .iter()
                .map(|set| set.name.as_str())
                .collect::<Vec<_>>(),
            ["Default", "Driving"]
        );
        // The Menus layer went with its set; the Driving layer is now index 0.
        assert_eq!(
            profile
                .action_layers
                .iter()
                .map(|layer| layer.name.as_str())
                .collect::<Vec<_>>(),
            ["Driving layer"]
        );
        assert_eq!(layer_targets(&profile), [0]);
        // switch(2) -> 1, switch(1) -> gone (pointed at the removed set).
        assert_eq!(set_targets(&profile), [1]);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_remove_action_set_clears_cursor_preferences() {
        let mut profile = InputProfile {
            action_sets: vec![set("Default", Vec::new()), set("Mouse", Vec::new())],
            action_set_when_cursor_shown: Some(1),
            ..InputProfile::default()
        };
        profile.remove_action_set(1);
        assert_eq!(profile.action_set_when_cursor_shown, None);
    }

    #[test]
    fn test_remove_action_set_keeps_default_untouched() {
        let mut profile = InputProfile {
            action_sets: vec![set("Default", Vec::new()), set("Second", Vec::new())],
            ..InputProfile::default()
        };
        profile.remove_action_set(0);
        assert_eq!(profile.action_sets.len(), 2);
        profile.remove_action_set(9);
        assert_eq!(profile.action_sets.len(), 2);
    }

    #[test]
    fn test_remove_action_layer_shifts_enable_layer_targets() {
        let mut profile = InputProfile {
            action_sets: vec![set(
                "Default",
                vec![enable_layer(0), enable_layer(1), enable_layer(2)],
            )],
            action_layers: vec![
                layer("One", "Default"),
                layer("Two", "Default"),
                layer("Three", "Default"),
            ],
            ..InputProfile::default()
        };
        profile.remove_action_layer(0);
        assert_eq!(layer_targets(&profile), [0, 1]);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_rename_action_set_reparents_layers() {
        let mut profile = InputProfile {
            action_sets: vec![set("Default", Vec::new()), set("Wheel", Vec::new())],
            action_layers: vec![layer("Boost", "Wheel"), layer("Other", "Default")],
            ..InputProfile::default()
        };
        profile.rename_action_set(1, "Driving".to_string());
        assert_eq!(profile.action_layers[0].parent_set, "Driving");
        assert_eq!(profile.action_layers[1].parent_set, "Default");
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_free_name_checks_against_existing_entries() {
        let profile = InputProfile {
            action_sets: vec![set("Default", Vec::new()), set("Set 2", Vec::new())],
            action_layers: vec![layer("Layer 1", "Default")],
            ..InputProfile::default()
        };
        assert!(!profile.is_free_set_name("Set 2"));
        assert!(profile.is_free_set_name("Set 3"));
        assert!(!profile.is_free_layer_name("Layer 1"));
        assert!(profile.is_free_layer_name("Layer 2"));
    }

    #[test]
    fn test_native_controller_gates_on_switch_and_gyro_output() {
        let mut profile = InputProfile::default();
        assert!(!profile.wants_native_controller());
        profile.native_motion = true;
        assert!(profile.wants_native_controller());
        profile.native_motion = false;
        profile.gyro.enabled = true;
        profile.gyro.output = super::super::GyroOutput::NativeMotion;
        assert!(profile.wants_native_controller());
        profile.gyro.enabled = false;
        assert!(!profile.wants_native_controller());
    }

    #[test]
    fn test_validate_rejects_duplicate_set_names_and_bad_cursor_targets() {
        let duplicated = InputProfile {
            action_sets: vec![set("Same", Vec::new()), set("Same", Vec::new())],
            ..InputProfile::default()
        };
        assert!(duplicated.validate().is_err());
        let dangling_cursor = InputProfile {
            action_sets: vec![set("Default", Vec::new())],
            action_set_when_cursor_hidden: Some(3),
            ..InputProfile::default()
        };
        assert!(dangling_cursor.validate().is_err());
    }
}
