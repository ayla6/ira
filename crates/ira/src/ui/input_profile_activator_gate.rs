//! Activation gating for the binding sheet: hold/toggle/disable-while
//! source pickers plus the analog gate editor (axis, condition, threshold).

use super::input_profile_editor_regions::{source_label, supported_button_sources};
use super::input_profile_options::{activation_index, activation_labels};
use super::input_profile_sheet_base::{combo_row, find_mapping, with_mapping, Reopen, SheetBase};
use adw::prelude::*;
use ira_input::{Activation, Activator, AnalogCondition, GamepadAxis, GamepadButton, InputSource};

pub(crate) fn activator_gate_controls(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    activator: &Activator,
    expander: &adw::ExpanderRow,
) {
    let gate = combo_row(
        &activation_labels(),
        activation_index(&activator.activation),
    );
    gate.set_title(&crate::tr!("Requires"));
    gate.set_subtitle(&crate::tr!("Only fire while this condition holds"));
    expander.add_row(&gate);
    let base_for_gate = base.clone();
    let reopen = reopen.clone();
    gate.connect_selected_notify(move |dropdown| {
        // Read the pre-change activation so switching gate flavor keeps the
        // previously chosen button or axis where one exists.
        let current = find_mapping(&base_for_gate)
            .and_then(|mapping| mapping.activators.get(index).cloned())
            .map(|activator| activator.activation)
            .unwrap_or(Activation::Always);
        with_mapping(&base_for_gate, |input| {
            if let Some(activator) = input.activators.get_mut(index) {
                activator.activation = gate_from_selection(dropdown.selected(), &current);
            }
        });
        (base_for_gate.on_changed)();
        reopen();
    });

    match &activator.activation {
        Activation::Analog {
            axis,
            condition,
            threshold,
        } => {
            add_analog_gate_rows(base, index, *axis, *condition, *threshold, expander);
        }
        Activation::Hold(_) | Activation::Toggle(_) | Activation::DisableWhile(_) => {
            expander.add_row(&gate_source_row(base, index, &activator.activation));
        }
        _ => {}
    }
}

fn gate_from_selection(selected: u32, current: &Activation) -> Activation {
    let button = InputSource::Button(default_gate_button(current));
    match selected {
        1 => Activation::Hold(button),
        2 => Activation::Toggle(button),
        3 => Activation::DisableWhile(button),
        5 => Activation::Analog {
            axis: default_gate_axis(current),
            condition: AnalogCondition::Active,
            threshold: 0.1,
        },
        _ => Activation::Always,
    }
}

fn default_gate_button(activation: &Activation) -> GamepadButton {
    if let Activation::Hold(InputSource::Button(button))
    | Activation::Toggle(InputSource::Button(button))
    | Activation::DisableWhile(InputSource::Button(button)) = activation
    {
        return *button;
    }
    GamepadButton::LeftTrigger
}

fn default_gate_axis(activation: &Activation) -> GamepadAxis {
    if let Activation::Analog { axis, .. } = activation {
        return *axis;
    }
    GamepadAxis::LeftTrigger
}

/// Append the axis / condition / threshold rows for an analog gate.
fn add_analog_gate_rows(
    base: &SheetBase,
    index: usize,
    axis: GamepadAxis,
    condition: AnalogCondition,
    threshold: f32,
    expander: &adw::ExpanderRow,
) {
    let axis_labels: Vec<String> = [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ]
    .iter()
    .map(|axis| super::input_profile_options::axis_label(*axis))
    .collect();
    let axis_row = combo_row(&axis_labels, axis_index(axis) as u32);
    axis_row.set_title(&crate::tr!("Axis"));

    let condition_labels = [
        crate::tr!("at rest"),
        crate::tr!("not at zero"),
        crate::tr!("maxed out"),
    ];
    let condition_row = combo_row(
        condition_labels.as_ref(),
        match condition {
            AnalogCondition::AtRest => 0,
            AnalogCondition::Active => 1,
            AnalogCondition::MaxedOut => 2,
        },
    );
    condition_row.set_title(&crate::tr!("Condition"));

    let threshold_spin = gtk4::SpinButton::with_range(0.01, 0.5, 0.01);
    threshold_spin.set_digits(2);
    threshold_spin.set_value(f64::from(threshold));
    threshold_spin.set_valign(gtk4::Align::Center);
    let threshold_row = adw::ActionRow::new();
    threshold_row.set_title(&crate::tr!("Threshold"));
    threshold_row.add_suffix(&threshold_spin);

    let base = base.clone();
    let apply =
        move |base: &SheetBase, axis: GamepadAxis, condition: AnalogCondition, threshold: f32| {
            with_mapping(base, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    if let Activation::Analog {
                        axis: gate_axis,
                        condition: gate_condition,
                        threshold: gate_threshold,
                    } = &mut activator.activation
                    {
                        *gate_axis = axis;
                        *gate_condition = condition;
                        *gate_threshold = threshold;
                    }
                }
            });
            (base.on_changed)();
        };

    let base_for_axis = base.clone();
    let condition_for_axis = condition;
    axis_row.connect_selected_notify(move |dropdown| {
        let axis = [
            GamepadAxis::LeftX,
            GamepadAxis::LeftY,
            GamepadAxis::RightX,
            GamepadAxis::RightY,
            GamepadAxis::LeftTrigger,
            GamepadAxis::RightTrigger,
        ][dropdown.selected() as usize];
        apply(&base_for_axis, axis, condition_for_axis, threshold);
    });

    let base_for_condition = base.clone();
    let axis_for_condition = axis;
    condition_row.connect_selected_notify(move |dropdown| {
        let condition = match dropdown.selected() {
            0 => AnalogCondition::AtRest,
            2 => AnalogCondition::MaxedOut,
            _ => AnalogCondition::Active,
        };
        apply(
            &base_for_condition,
            axis_for_condition,
            condition,
            threshold,
        );
    });

    let base_for_threshold = base.clone();
    threshold_spin.connect_value_changed(move |spin| {
        with_mapping(&base_for_threshold, |input| {
            if let Some(activator) = input.activators.get_mut(index) {
                if let Activation::Analog { threshold, .. } = &mut activator.activation {
                    *threshold = spin.value() as f32;
                }
            }
        });
        (base_for_threshold.on_changed)();
    });

    expander.add_row(&axis_row);
    expander.add_row(&condition_row);
    expander.add_row(&threshold_row);
}

fn axis_index(axis: GamepadAxis) -> usize {
    match axis {
        GamepadAxis::LeftX => 0,
        GamepadAxis::LeftY => 1,
        GamepadAxis::RightX => 2,
        GamepadAxis::RightY => 3,
        GamepadAxis::LeftTrigger => 4,
        GamepadAxis::RightTrigger => 5,
    }
}

/// "While holding <button>" selector for hold/toggle/disable gates.
fn gate_source_row(base: &SheetBase, index: usize, activation: &Activation) -> adw::ComboRow {
    let gate_sources: Vec<(InputSource, String)> = supported_button_sources(base.device.as_ref())
        .into_iter()
        .map(|source| (source, source_label(source)))
        .collect();
    let gate_labels: Vec<String> = gate_sources
        .iter()
        .map(|(_, label)| label.clone())
        .collect();
    let selected = gate_sources
        .iter()
        .position(|(source, _)| {
            matches!(
                activation,
                Activation::Hold(current)
                    | Activation::Toggle(current)
                    | Activation::DisableWhile(current)
                    if current == source
            )
        })
        .unwrap_or(0);
    let row = combo_row(&gate_labels, selected as u32);
    row.set_title(&crate::tr!("While holding"));
    let base = base.clone();
    row.connect_selected_notify(move |dropdown| {
        if let Some((gate_input, _)) = gate_sources.get(dropdown.selected() as usize) {
            let gate_input = *gate_input;
            with_mapping(&base, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    match activator.activation {
                        Activation::Hold(_) => activator.activation = Activation::Hold(gate_input),
                        Activation::Toggle(_) => {
                            activator.activation = Activation::Toggle(gate_input)
                        }
                        Activation::DisableWhile(_) => {
                            activator.activation = Activation::DisableWhile(gate_input)
                        }
                        _ => {}
                    }
                }
            });
            (base.on_changed)();
        }
    });
    row
}
