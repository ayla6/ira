//! Analog behavior for one stick or trigger: the mode picker (joystick,
//! dpad, mouse, flick stick, trigger), its response rows (Steam's named
//! response-curve presets plus a custom-curve slider), and the writer
//! plumbing that routes edits either to the mapping's own mode or to one
//! of its mode shifts.

use super::input_profile_sheet_base::{
    combo_row, is_trigger_axis, with_mapping, Reopen, SheetBase,
};
use super::input_profile_widgets::{slider_row, slider_row_with_scale, OptionChoice, SliderSpec};
use adw::prelude::*;
use ira_input::{GamepadAxis, InputSource, JoystickSettings, SourceMode, StickOutput, StickProcessing};

/// Which `SourceMode` an edit targets: the mapping's own behavior or the
/// shifted behavior of one of its mode shifts.
#[derive(Clone, Copy)]
pub(crate) enum ModeTarget {
    Base,
    Shift(usize),
}

pub(crate) fn modes_for(source: InputSource) -> Vec<Option<SourceMode>> {
    // The d-pad group's behavior, Steam's dpad page, stored on the Up row
    // the way a stick's mode lives on its X axis. "None" rides the stick
    // Dpad mode: on button sources the engine runs it fully disabled.
    if matches!(source, InputSource::Button(button) if button.is_dpad()) {
        return vec![
            Some(SourceMode::Dpad { threshold: 0.5 }),
            None,
            Some(SourceMode::ButtonPad),
            Some(SourceMode::Joystick(JoystickSettings::new(StickOutput::Left))),
        ];
    }
    if is_trigger_axis(source) {
        vec![None, Some(SourceMode::Trigger { threshold: 0.5 })]
    } else {
        vec![
            None,
            Some(SourceMode::joystick(default_stick_output(source))),
            Some(SourceMode::Dpad { threshold: 0.5 }),
            Some(SourceMode::Mouse {
                sensitivity: 1.0,
                stick: StickProcessing::default(),
            }),
            Some(SourceMode::Flickstick {
                rotation_sensitivity: 1.0,
                flick_duration_ms: 100,
            }),
        ]
    }
}

pub(crate) fn mode_label(mode: &Option<SourceMode>, is_trigger: bool) -> String {
    match mode {
        None => crate::tr!("None"),
        Some(SourceMode::Joystick(_)) => crate::tr!("Joystick"),
        Some(SourceMode::Dpad { .. }) => crate::tr!("Directional Pad"),
        Some(SourceMode::ButtonPad) => crate::tr!("Button Pad"),
        Some(SourceMode::Mouse { .. }) => crate::tr!("Joystick Mouse"),
        Some(SourceMode::Flickstick { .. }) => crate::tr!("Flick Stick"),
        Some(SourceMode::Trigger { .. }) if is_trigger => crate::tr!("Trigger"),
        _ => crate::tr!("Other"),
    }
}

pub(crate) fn same_mode(left: &Option<SourceMode>, right: &SourceMode) -> bool {
    match left {
        Some(a) => std::mem::discriminant(a) == std::mem::discriminant(right),
        None => false,
    }
}

fn default_stick_output(source: InputSource) -> StickOutput {
    match source {
        InputSource::Axis(GamepadAxis::RightX | GamepadAxis::RightY) => StickOutput::Right,
        _ => StickOutput::Left,
    }
}

/// Steam's named response-curve presets. The runtime raises deflection to
/// the `curve` exponent, so values below 1.0 reach full output sooner
/// (aggressive) and values above 1.0 later (wide).
const CURVE_PRESET_VALUES: [f32; 5] = [1.0, 0.5, 1.5, 2.0, 3.0];
/// Index of the trailing "Custom Curve" entry, which has no fixed value.
pub(crate) const CURVE_CUSTOM_INDEX: usize = CURVE_PRESET_VALUES.len();

pub(crate) fn curve_presets() -> Vec<OptionChoice> {
    vec![
        OptionChoice {
            title: crate::tr!("Linear"),
            description: Some(crate::tr!(
                "A linear response curve maps the input directly to the output in a 1:1 fashion. At 50% deflection, 50% output will be sent."
            )),
        },
        OptionChoice {
            title: crate::tr!("Aggressive"),
            description: Some(crate::tr!(
                "An aggressive response curve gets to 100% output faster, leaving less of the slow range for fine control."
            )),
        },
        OptionChoice {
            title: crate::tr!("Relaxed"),
            description: Some(crate::tr!(
                "A relaxed response curve gets to 100% output slower, giving a little more slow range for fine control."
            )),
        },
        OptionChoice {
            title: crate::tr!("Wide"),
            description: Some(crate::tr!(
                "A wide response curve gets to 100% output much slower, with a broad low-output range before ramping up."
            )),
        },
        OptionChoice {
            title: crate::tr!("Extra Wide"),
            description: Some(crate::tr!(
                "An extra wide response curve provides an extremely large range of lower values, only reaching full output at the very edge."
            )),
        },
        OptionChoice {
            title: crate::tr!("Custom Curve"),
            description: Some(crate::tr!(
                "A custom curve can be defined using the slider below."
            )),
        },
    ]
}

/// Curve exponent for a preset index; `None` for Custom, which keeps the
/// current value and only reveals the slider.
pub(crate) fn curve_preset_value(index: usize) -> Option<f32> {
    CURVE_PRESET_VALUES.get(index).copied()
}

pub(crate) fn curve_preset_index(curve: f32) -> usize {
    for (index, value) in CURVE_PRESET_VALUES.iter().enumerate() {
        if (curve - value).abs() < 0.01 {
            return index;
        }
    }
    CURVE_CUSTOM_INDEX
}

/// The picker's selected index. The `curve_custom` flag decides between a
/// preset and Custom when the exponent is ambiguous — 1.0 is both Linear
/// and a legal custom value.
pub(crate) fn curve_combo_index(curve_custom: bool, curve: f32) -> usize {
    if curve_custom {
        CURVE_CUSTOM_INDEX
    } else {
        curve_preset_index(curve)
    }
}

/// Response rows for a mode, shared by the base behavior group and by the
/// mode-shift expanders (which target the shift's mode instead).
pub(crate) fn mode_setting_rows(
    base: &SheetBase,
    target: ModeTarget,
    mode: &SourceMode,
    reopen: &Reopen,
) -> Vec<gtk4::ListBoxRow> {
    let mut rows = Vec::new();
    match mode {
        SourceMode::Joystick(settings) => {
            rows.extend(super::input_profile_stick_settings::joystick_rows(
                base,
                target,
                reopen,
                &settings.processing,
            ));
        }
        SourceMode::Mouse { sensitivity, .. } => {
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Sensitivity"),
                Some(&crate::tr!("How fast the pointer moves per stick motion")),
                &SliderSpec(0.05, 20.0, 0.05, f64::from(*sensitivity)),
                |mode, value| {
                    if let SourceMode::Mouse { sensitivity, .. } = mode {
                        *sensitivity = value as f32;
                    }
                },
            ));
        }
        SourceMode::Dpad { threshold } => {
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Activation threshold"),
                Some(&crate::tr!(
                    "How far the stick must move before a direction registers"
                )),
                &SliderSpec(0.2, 0.95, 0.05, f64::from(*threshold)),
                |mode, value| {
                    if let SourceMode::Dpad { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
        }
        SourceMode::ButtonPad => {
            // Four independent buttons: no group settings.
        }
        SourceMode::Trigger { threshold } => {
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Full pull threshold"),
                Some(&crate::tr!(
                    "How far the trigger must be pulled for the full-pull activator"
                )),
                &SliderSpec(0.1, 1.0, 0.05, f64::from(*threshold)),
                |mode, value| {
                    if let SourceMode::Trigger { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
        }
        SourceMode::Flickstick {
            rotation_sensitivity,
            flick_duration_ms,
        } => {
            // Steam's shared angle calibration: pixels per full 360° sweep
            // at 1x. Lives on the shared live Gyro config — the same copy
            // the Gyro page's Dots Per 360° row edits — so read it back
            // from there too.
            let initial_dots = base.gyro.borrow().dots_per_360;
            let calibration_base = base.clone();
            rows.push(slider_row_with_scale(
                &crate::tr!("Flick Stick ° to Mouse Pixels (Dots Per 360°)"),
                Some(&crate::tr!(
                    "One full 360° sweep of the stick turns the camera this many pixels of mouse movement at 1x sweep sensitivity. Shared with the Gyro's Dots Per 360°."
                )),
                &SliderSpec(500.0, 30_000.0, 5.0, f64::from(initial_dots)),
                move |value| {
                    calibration_base.gyro.borrow_mut().dots_per_360 = value as f32;
                    (calibration_base.on_adjusted)();
                })
            .0);
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Flick Stick ° Sweep Sensitivity"),
                Some(&crate::tr!(
                    "How far a flick turns per degree of stick rotation"
                )),
                &SliderSpec(0.1, 10.0, 0.1, f64::from(*rotation_sensitivity)),
                |mode, value| {
                    if let SourceMode::Flickstick {
                        rotation_sensitivity,
                        ..
                    } = mode
                    {
                        *rotation_sensitivity = value as f32;
                    }
                },
            ));
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Flick duration"),
                Some(&crate::tr!("How long the turn input of a flick lasts")),
                &SliderSpec(40.0, 400.0, 10.0, f64::from(*flick_duration_ms)),
                |mode, value| {
                    if let SourceMode::Flickstick {
                        flick_duration_ms, ..
                    } = mode
                    {
                        *flick_duration_ms = value as u32;
                    }
                },
            ));
        }
    }
    rows
}

/// The Response curve combo: Steam's named presets plus Custom, which keeps
/// the current exponent and reveals the slider underneath. Writes go through
/// `processing_of` so both behaviors sharing these rows (Joystick and
/// Joystick Mouse) pick the change up; the row's subtitle carries the
/// selected preset's description.
pub(crate) fn curve_preset_row(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    processing: &StickProcessing,
) -> adw::ComboRow {
    let presets = curve_presets();
    let current = curve_combo_index(processing.curve_custom, processing.curve);
    let combo = combo_row(
        &presets
            .iter()
            .map(|choice| choice.title.clone())
            .collect::<Vec<_>>(),
        current as u32,
    );
    combo.set_title(&crate::tr!("Response curve"));
    let description = presets[current].description.clone().unwrap_or_default();
    combo.set_subtitle(description.as_str());
    let was_custom = current == CURVE_CUSTOM_INDEX;
    let base = std::rc::Rc::new(base.clone());
    let reopen = reopen.clone();
    combo.connect_selected_notify(move |combo| {
        let index = combo.selected() as usize;
        let write = mode_writer(&base, target);
        write(&mut |mode| {
            if let Some(processing) = super::input_profile_stick_settings::processing_of(mode) {
                match curve_preset_value(index) {
                    Some(value) => {
                        processing.curve = value;
                        processing.curve_custom = false;
                    }
                    // Custom keeps the current exponent; the flag alone
                    // distinguishes it from the matching preset.
                    None => processing.curve_custom = true,
                }
            }
        });
        if let Some(choice) = presets.get(index) {
            let description = choice.description.clone().unwrap_or_default();
            combo.set_subtitle(description.as_str());
        }
        (base.on_changed)();
        // Entering or leaving Custom is the only pick that changes the row
        // set — it reveals or removes the slider underneath.
        if was_custom != curve_preset_value(index).is_none() {
            reopen();
        }
    });
    combo
}

pub(crate) fn curve_slider_row(
    base: &SheetBase,
    target: ModeTarget,
    curve: f32,
) -> gtk4::ListBoxRow {
    let base_for_change = base.clone();
    slider_row(
        &crate::tr!("Custom curve"),
        None,
        &SliderSpec(0.2, 3.0, 0.05, f64::from(curve)),
        move |value| {
            let write = mode_writer(&base_for_change, target);
            write(&mut |mode| {
                if let Some(processing) = super::input_profile_stick_settings::processing_of(mode) {
                    processing.curve = value as f32;
                    // Dragging the slider is the Custom pick in progress.
                    processing.curve_custom = true;
                }
            });
            (base_for_change.on_adjusted)();
        })
}

/// Returns a closure that mutates the targeted mode in place, if any.
pub(crate) fn mode_writer(
    base: &SheetBase,
    target: ModeTarget,
) -> impl Fn(&mut dyn FnMut(&mut SourceMode)) {
    let base = base.clone();
    move |mutate: &mut dyn FnMut(&mut SourceMode)| {
        with_mapping(&base, |input| {
            let mode_slot = match target {
                ModeTarget::Base => input.mode.as_mut(),
                ModeTarget::Shift(index) => input
                    .mode_shifts
                    .get_mut(index)
                    .and_then(|shift| shift.mode.as_mut()),
            };
            if let Some(current) = mode_slot {
                mutate(current);
            }
        });
    }
}

/// One slider row bound to a field of the targeted SourceMode.
/// One slider row bound to a numeric field of the targeted SourceMode.
/// Slider drags fire continuously; they route through `on_adjusted` so the
/// page is never rebuilt under the pointer mid-gesture.
pub(crate) fn mode_slider_row(
    base: &SheetBase,
    target: ModeTarget,
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    mutate: fn(&mut SourceMode, f64),
) -> gtk4::ListBoxRow {
    let base = base.clone();
    slider_row(title, subtitle, spec, move |value| {
        let write = mode_writer(&base, target);
        write(&mut |mode| mutate(mode, value));
        (base.on_adjusted)();
    })
}

#[cfg(test)]
mod tests {
    use super::{curve_combo_index, curve_preset_index, curve_preset_value, CURVE_CUSTOM_INDEX};

    #[test]
    fn test_curve_preset_round_trip_for_named_values() {
        for (index, value) in [
            super::CURVE_PRESET_VALUES[0],
            super::CURVE_PRESET_VALUES[1],
            super::CURVE_PRESET_VALUES[2],
            super::CURVE_PRESET_VALUES[3],
            super::CURVE_PRESET_VALUES[4],
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(curve_preset_index(value), index);
            assert_eq!(curve_preset_value(index), Some(value));
        }
    }

    #[test]
    fn test_curve_preset_index_custom_for_other_values() {
        assert_eq!(curve_preset_index(0.7), CURVE_CUSTOM_INDEX);
        assert_eq!(curve_preset_index(2.5), CURVE_CUSTOM_INDEX);
        assert_eq!(curve_preset_value(CURVE_CUSTOM_INDEX), None);
    }

    #[test]
    fn test_curve_preset_index_tolerates_rounding() {
        // Values saved as f32 can drift in the last decimals.
        assert_eq!(curve_preset_index(0.49999), 1);
        assert_eq!(curve_preset_index(1.004), 0);
    }

    #[test]
    fn test_curve_combo_index_flag_decides_at_preset_values() {
        // A preset value is only Custom when the flag says so.
        assert_eq!(curve_combo_index(false, 1.0), 0);
        assert_eq!(curve_combo_index(true, 1.0), CURVE_CUSTOM_INDEX);
    }

    #[test]
    fn test_curve_combo_index_unknown_value_reads_as_custom() {
        assert_eq!(curve_combo_index(false, 0.7), CURVE_CUSTOM_INDEX);
        assert_eq!(curve_combo_index(false, 0.5), 1);
    }
}
