//! Analog behavior for one stick or trigger: the mode picker (joystick,
//! dpad, mouse, flick stick, trigger), its response rows (Steam's named
//! response-curve presets plus a custom-curve slider), and the writer
//! plumbing that routes edits either to the mapping's own mode or to one
//! of its mode shifts.

use super::input_profile_sheet_base::{
    combo_row, find_mapping, is_trigger_axis, with_mapping, Reopen, SheetBase,
};
use super::input_profile_widgets::{
    slider_row_with_scale,
    format_ms, format_number, format_percent, option_picker_popover, picker_button, slider_row,
    OptionChoice, SettingGroup, SliderSpec,
};
use adw::prelude::*;
use ira_input::{GamepadAxis, InputSource, SourceMode, StickOutput, StickProcessing};

/// Which `SourceMode` an edit targets: the mapping's own behavior or the
/// shifted behavior of one of its mode shifts.
#[derive(Clone, Copy)]
pub(crate) enum ModeTarget {
    Base,
    Shift(usize),
}

pub(crate) fn modes_for(source: InputSource) -> Vec<Option<SourceMode>> {
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
        Some(SourceMode::Mouse { .. }) => crate::tr!("Joystick Mouse"),
        Some(SourceMode::Flickstick { .. }) => crate::tr!("Flick Stick"),
        Some(SourceMode::Trigger { .. }) if is_trigger => crate::tr!("Trigger"),
        _ => crate::tr!("Other"),
    }
}

/// Steam-style explanation shown under the behavior dropdown for the
/// currently selected mode.
pub(crate) fn mode_description(mode: &Option<SourceMode>, is_trigger: bool) -> String {
    match mode {
        None => crate::tr!(
            "The input is unused — the controller ignores it entirely"
        )
        .to_string(),
        Some(SourceMode::Joystick(_)) => crate::tr!(
            "Moves a joystick for camera or movement, with a tunable response curve"
        )
        .to_string(),
        Some(SourceMode::Dpad { .. }) => crate::tr!(
            "Emulates a d-pad in the direction the stick is pushed"
        )
        .to_string(),
        Some(SourceMode::Mouse { .. }) => crate::tr!(
            "Moves the mouse cursor — good for menus and mouse-look camera"
        )
        .to_string(),
        Some(SourceMode::Flickstick { .. }) => crate::tr!(
            "Flicks the view in the stick's direction, then turns while held — pair with gyro aiming"
        )
        .to_string(),
        Some(SourceMode::Trigger { .. }) if is_trigger => crate::tr!(
            "Analog trigger with an adjustable pull threshold"
        )
        .to_string(),
        _ => String::new(),
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
                "A custom curve can be defined using the slider below the preset picker."
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

pub(crate) fn behavior_group(
    base: &SheetBase,
    reopen: &Reopen,
    modes: Vec<Option<SourceMode>>,
) -> gtk4::Box {
    let group = SettingGroup::new(
        Some(&crate::tr!("Behavior")),
        Some(&crate::tr!("What this stick or trigger does")),
    );

    let is_trigger = is_trigger_axis(base.source);
    let current = find_mapping(base).and_then(|mapping| mapping.mode);
    let selected = current
        .as_ref()
        .and_then(|mode| {
            modes
                .iter()
                .position(|candidate| same_mode(candidate, mode))
        })
        .unwrap_or(0);
    let labels: Vec<String> = modes
        .iter()
        .map(|mode| mode_label(mode, is_trigger))
        .collect();

    let dropdown = combo_row(&labels, selected as u32);
    dropdown.set_title(&crate::tr!("Behavior"));
    dropdown.set_subtitle(&mode_description(&current, is_trigger));
    group.add(&dropdown);

    let base_for_change = base.clone();
    let reopen_for_change = reopen.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        let mode = modes.get(dropdown.selected() as usize).cloned().flatten();
        dropdown.set_subtitle(&mode_description(&mode, is_trigger));
        with_mapping(&base_for_change, |input| {
            input.mode = mode;
        });
        (base_for_change.on_changed)();
        reopen_for_change();
    });

    group.root
}

pub(crate) fn mode_settings_group(
    base: &SheetBase,
    mode: &SourceMode,
    reopen: &Reopen,
) -> gtk4::Box {
    let group = SettingGroup::new(Some(&crate::tr!("Response")), None);
    for row in mode_setting_rows(base, ModeTarget::Base, mode, reopen) {
        group.add(&row);
    }
    group.root
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
                format_number,
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
                format_percent,
                |mode, value| {
                    if let SourceMode::Dpad { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
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
                format_percent,
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
            // at 1x. Lives on the profile so the gyro edits the same value.
            let calibration_base = base.clone();
            rows.push(slider_row_with_scale(
                &crate::tr!("Flick Stick ° to Mouse Pixels (Dots Per 360°)"),
                Some(&crate::tr!(
                    "One full 360° sweep of the stick turns the camera this many pixels of mouse movement at 1x sweep sensitivity. Shared with the Gyro's Dots Per 360°."
                )),
                &SliderSpec(
                    500.0,
                    30_000.0,
                    5.0,
                    f64::from(base.profile.borrow().gyro.dots_per_360),
                ),
                |value| format!("{value:.0}px"),
                move |value| {
                    calibration_base.profile.borrow_mut().gyro.dots_per_360 = value as f32;
                    (calibration_base.on_changed)();
                },
            )
            .0);
            rows.push(mode_slider_row(
                base,
                target,
                &crate::tr!("Flick Stick ° Sweep Sensitivity"),
                Some(&crate::tr!(
                    "How far a flick turns per degree of stick rotation"
                )),
                &SliderSpec(0.1, 10.0, 0.1, f64::from(*rotation_sensitivity)),
                |value| format!("{value:.1}x"),
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
                format_ms,
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

/// The preset picker row; its MenuButton carries the current preset's name.
pub(crate) fn curve_preset_row(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    curve: f32,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Response curve"));
    row.set_subtitle(&crate::tr!("How quickly the stick reaches full output"));
    let presets = curve_presets();
    let current = curve_preset_index(curve);
    let current_label = presets
        .get(current)
        .map(|choice| choice.title.clone())
        .unwrap_or_else(|| crate::tr!("Custom Curve"));
    let base_for_pick = base.clone();
    let reopen_for_pick = reopen.clone();
    let picker = option_picker_popover(&presets, current, move |index| {
        if let Some(value) = curve_preset_value(index) {
            let write = mode_writer(&base_for_pick, target);
            write(&mut |mode| {
                if let SourceMode::Joystick(settings) = mode {
                    settings.processing.curve = value;
                }
            });
            (base_for_pick.on_changed)();
        }
        // Custom keeps the current exponent; the rebuild reveals the slider.
        reopen_for_pick();
    });
    row.add_suffix(&picker_button(&current_label, &picker));
    row
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
        format_number,
        move |value| {
            let write = mode_writer(&base_for_change, target);
            write(&mut |mode| {
                if let SourceMode::Joystick(settings) = mode {
                    settings.processing.curve = value as f32;
                }
            });
            (base_for_change.on_changed)();
        },
    )
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
pub(crate) fn mode_slider_row(
    base: &SheetBase,
    target: ModeTarget,
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    format: impl Fn(f64) -> String + 'static,
    mutate: fn(&mut SourceMode, f64),
) -> gtk4::ListBoxRow {
    let base = base.clone();
    slider_row(title, subtitle, spec, format, move |value| {
        let write = mode_writer(&base, target);
        write(&mut |mode| mutate(mode, value));
        (base.on_changed)();
    })
}

#[cfg(test)]
mod tests {
    use super::{curve_preset_index, curve_preset_value, CURVE_CUSTOM_INDEX};

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
}
