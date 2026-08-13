use super::types::BindingRow;
use adw::prelude::*;
use ira_input::{
    Activation, AxisTransform, Binding, ChordMode, GyroMode, InputSource, RecenterMode,
};

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
        gyro_mode: match row.gyro_mode.selected() {
            1 => GyroMode::HoldLast,
            _ => GyroMode::Rate,
        },
        activation,
        transform: AxisTransform {
            dead_zone: row.dead_zone.value() as f32,
            sensitivity: row.sensitivity.value() as f32,
            exponent: row.exponent.value() as f32,
            invert: row.invert.is_active(),
        },
        recenter: match row.recenter.selected() {
            1 => RecenterMode::OnEnable,
            2 => RecenterMode::OnDisable,
            3 => RecenterMode::OnEnableOrDisable,
            _ => RecenterMode::Never,
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
