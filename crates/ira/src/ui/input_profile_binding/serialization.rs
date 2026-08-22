use super::types::BindingRow;
use adw::prelude::*;
use ira_input::{Activation, AnalogCondition, AxisTransform, Binding, ChordMode, GamepadAxis, InputSource};

pub(crate) fn binding_from_row(row: &BindingRow) -> Result<Binding, String> {
    let source = row
        .source_options
        .get(row.source.selected() as usize)
        .map(|(source, _)| *source)
        .ok_or_else(|| "Invalid binding source".to_string())?;
    let output = row.output_action.borrow().clone();
    let activation = activation_from_row(row)?;
    Ok(Binding {
        source,
        output,
        activation,
        transform: AxisTransform {
            dead_zone: row.dead_zone.value() as f32,
            sensitivity: row.sensitivity.value() as f32,
            exponent: row.exponent.value() as f32,
            invert: row.invert.is_active(),
        },
    })
}

fn activation_from_row(row: &BindingRow) -> Result<Activation, String> {
    let source = row
        .activator_options
        .get(row.activator.selected() as usize)
        .map(|(source, _)| *source)
        .ok_or_else(|| "Invalid activator source".to_string())?;
    Ok(match row.activation.selected() {
        1 => Activation::Hold(source),
        2 => Activation::Toggle(source),
        3 => Activation::DisableWhile(source),
        4 => Activation::Chord {
            sources: parse_chord(&row.chord.text(), &row.activator_options)?,
            mode: ChordMode::Hold,
        },
        5 => Activation::Analog {
            axis: match source {
                InputSource::Axis(axis) => axis,
                _ => GamepadAxis::LeftTrigger,
            },
            condition: AnalogCondition::Active,
            threshold: 0.1,
        },
        _ => Activation::Always,
    })
}

fn parse_chord(text: &str, options: &[(InputSource, String)]) -> Result<Vec<InputSource>, String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            options
                .iter()
                .find(|(_, label)| label.eq_ignore_ascii_case(part))
                .map(|(source, _)| *source)
                .ok_or_else(|| format!("Unknown chord source: {part}"))
        })
        .collect()
}
