//! Analog behavior for one stick or trigger: the mode picker (joystick,
//! dpad, mouse, flick stick, trigger), its response rows, and the writer
//! plumbing that routes edits either to the mapping's own mode or to one
//! of its mode shifts.

use super::input_profile_sheet_base::{
    combo_row, find_mapping, is_trigger_axis, spin_row, with_mapping, Reopen, SheetBase,
    SpinChange,
};
use adw::prelude::*;
use ira_input::{GamepadAxis, InputSource, SourceMode, StickOutput};
use std::rc::Rc;

/// Which `SourceMode` an edit targets: the mapping's own behavior or the
/// shifted behavior of one of its mode shifts.
#[derive(Clone, Copy)]
pub(crate) enum ModeTarget {
    Base,
    Shift(usize),
}

/// min / max / step / value for one mode spin row.
struct SpinSpec(f64, f64, f64, f64);

pub(crate) fn modes_for(source: InputSource) -> Vec<Option<SourceMode>> {
    if is_trigger_axis(source) {
        vec![None, Some(SourceMode::Trigger { threshold: 0.5 })]
    } else {
        vec![
            None,
            Some(SourceMode::Joystick {
                output: default_stick_output(source),
                deadzone_inner: 0.1,
                deadzone_outer: 0.95,
                curve: 1.0,
            }),
            Some(SourceMode::Dpad { threshold: 0.5 }),
            Some(SourceMode::Mouse { sensitivity: 1.0 }),
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
        Some(SourceMode::Joystick { .. }) => crate::tr!("Joystick"),
        Some(SourceMode::Dpad { .. }) => crate::tr!("Directional Pad"),
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

pub(crate) fn behavior_group(
    base: &SheetBase,
    reopen: &Reopen,
    modes: Vec<Option<SourceMode>>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Behavior"));
    group.set_description(Some(&crate::tr!("What this stick or trigger does")));

    let is_trigger = is_trigger_axis(base.source);
    let current = find_mapping(base).and_then(|mapping| mapping.mode);
    let selected = current
        .as_ref()
        .and_then(|mode| modes.iter().position(|candidate| same_mode(candidate, mode)))
        .unwrap_or(0);
    let labels: Vec<String> = modes.iter().map(|mode| mode_label(mode, is_trigger)).collect();

    let dropdown = combo_row(&labels, selected as u32);
    dropdown.set_title(&crate::tr!("Behavior"));
    dropdown.set_subtitle(&crate::tr!("What this stick or trigger does"));
    group.add(&dropdown);

    let base_for_change = base.clone();
    let reopen_for_change = reopen.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        let mode = modes
            .get(dropdown.selected() as usize)
            .cloned()
            .flatten();
        with_mapping(&base_for_change, |input| {
            input.mode = mode;
        });
        (base_for_change.on_changed)();
        reopen_for_change();
    });

    group
}

pub(crate) fn mode_settings_group(base: &SheetBase, mode: &SourceMode) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Response"));
    for row in mode_setting_rows(base, ModeTarget::Base, mode) {
        group.add(&row);
    }
    group
}

/// Response rows for a mode, shared by the base behavior group and by the
/// mode-shift expanders (which target the shift's mode instead).
pub(crate) fn mode_setting_rows(
    base: &SheetBase,
    target: ModeTarget,
    mode: &SourceMode,
) -> Vec<adw::SpinRow> {
    let mut rows = Vec::new();
    match mode {
        SourceMode::Joystick {
            deadzone_inner,
            deadzone_outer,
            curve,
            ..
        } => {
            rows.extend(joystick_rows(base, target, *deadzone_inner, *deadzone_outer, *curve));
        }
        SourceMode::Mouse { sensitivity } => {
            rows.push(mode_spin_row(
                base,
                target,
                &crate::tr!("Sensitivity"),
                SpinSpec(0.05, 20.0, 0.05, f64::from(*sensitivity)),
                |mode, value| {
                    if let SourceMode::Mouse { sensitivity } = mode {
                        *sensitivity = value as f32;
                    }
                },
            ));
        }
        SourceMode::Dpad { threshold } => {
            rows.push(mode_spin_row(
                base,
                target,
                &crate::tr!("Activation threshold"),
                SpinSpec(0.2, 0.95, 0.05, f64::from(*threshold)),
                |mode, value| {
                    if let SourceMode::Dpad { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
        }
        SourceMode::Trigger { threshold } => {
            rows.push(mode_spin_row(
                base,
                target,
                &crate::tr!("Full pull threshold"),
                SpinSpec(0.1, 1.0, 0.05, f64::from(*threshold)),
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
            rows.push(mode_spin_row(
                base,
                target,
                &crate::tr!("Rotation sensitivity"),
                SpinSpec(0.1, 10.0, 0.1, f64::from(*rotation_sensitivity)),
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
            rows.push(mode_spin_row(
                base,
                target,
                &crate::tr!("Flick duration (ms)"),
                SpinSpec(40.0, 400.0, 10.0, f64::from(*flick_duration_ms)),
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

fn joystick_rows(
    base: &SheetBase,
    target: ModeTarget,
    inner: f32,
    outer: f32,
    curve: f32,
) -> Vec<adw::SpinRow> {
    vec![
        mode_spin_row(
            base,
            target,
            &crate::tr!("Inner dead zone"),
            SpinSpec(0.0, 0.9, 0.01, f64::from(inner)),
            |mode, value| {
                if let SourceMode::Joystick { deadzone_inner, .. } = mode {
                    *deadzone_inner = value as f32;
                }
            },
        ),
        mode_spin_row(
            base,
            target,
            &crate::tr!("Outer dead zone"),
            SpinSpec(0.1, 1.0, 0.01, f64::from(outer)),
            |mode, value| {
                if let SourceMode::Joystick {
                    deadzone_outer, ..
                } = mode
                {
                    *deadzone_outer = value as f32;
                }
            },
        ),
        mode_spin_row(
            base,
            target,
            &crate::tr!("Response curve"),
            SpinSpec(0.2, 3.0, 0.05, f64::from(curve)),
            |mode, value| {
                if let SourceMode::Joystick { curve, .. } = mode {
                    *curve = value as f32;
                }
            },
        ),
    ]
}

/// Returns a closure that mutates the targeted mode in place, if any.
fn mode_writer(base: &SheetBase, target: ModeTarget) -> impl Fn(&mut dyn FnMut(&mut SourceMode)) {
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

/// One spin row bound to a field of the targeted SourceMode.
fn mode_spin_row(
    base: &SheetBase,
    target: ModeTarget,
    title: &str,
    spec: SpinSpec,
    mutate: fn(&mut SourceMode, f64),
) -> adw::SpinRow {
    let SpinSpec(min, max, step, value) = spec;
    let base = base.clone();
    let on_change: SpinChange = Rc::new(move |value| {
        let write = mode_writer(&base, target);
        write(&mut |mode| mutate(mode, value));
        (base.on_changed)();
    });
    spin_row(title, min, max, step, value, on_change)
}
