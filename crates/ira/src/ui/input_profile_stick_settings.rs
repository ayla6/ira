//! Steam-style joystick settings: sensitivity (per-axis scale, inverts,
//! response curve, response axis style), output (target stick, axis
//! limiting, rotation), deadzone (source picker with per-controller
//! support), and the outer ring command. The Joystick and Joystick Mouse
//! behaviors share the same rows; the same rows also serve the flat layout
//! inside shift expanders.

use super::input_profile_sheet_base::{combo_row, Reopen, SheetBase};
use super::input_profile_source_modes::{
    curve_preset_index, curve_preset_row, curve_slider_row, mode_slider_row, mode_writer,
    ModeTarget, CURVE_CUSTOM_INDEX,
};
use super::input_profile_stick_indices::{
    axis_style_from_index, axis_style_index, deadzone_from_index, deadzone_source_index,
    format_degrees, output_axis_from_index, output_axis_index, output_from_index, output_index,
};
use super::input_profile_widgets::{
    format_percent, option_picker_popover, picker_button, switch_row, OptionChoice,
    SliderSpec,
};
use adw::prelude::*;
use ira_input::{ResponseAxisStyle, SourceMode, StickDeadzone, StickOutput, StickProcessing};

/// The stick sheet's behavior groups: Steam's full layout for the Joystick
/// and Joystick Mouse behaviors, the generic response rows for every other
/// behavior.
/// Flat, titled sections for one stick mode — the expander children on the
/// region page. Each entry is a section heading plus its rows.
pub(crate) fn stick_setting_sections(
    base: &SheetBase,
    mode: &SourceMode,
    reopen: &Reopen,
) -> Vec<(String, Vec<gtk4::ListBoxRow>)> {
    match mode {
        SourceMode::Joystick(settings) => vec![
            (
                crate::tr!("Sensitivity"),
                sensitivity_rows(base, ModeTarget::Base, reopen, &settings.processing),
            ),
            (
                crate::tr!("Output"),
                output_rows(
                    base,
                    ModeTarget::Base,
                    &settings.processing,
                    Some(settings.output),
                ),
            ),
            (
                crate::tr!("Deadzones"),
                deadzone_rows(base, ModeTarget::Base, reopen, &settings.processing),
            ),
            (
                crate::tr!("Outer Ring"),
                super::input_profile_stick_ring::outer_ring_rows(
                    base,
                    ModeTarget::Base,
                    reopen,
                    &settings.processing,
                ),
            ),
        ],
        SourceMode::Mouse { sensitivity, stick } => vec![
            (
                crate::tr!("General"),
                mouse_general_rows(base, ModeTarget::Base, reopen, *sensitivity, stick),
            ),
            (crate::tr!("Output"), output_rows(base, ModeTarget::Base, stick, None)),
            (crate::tr!("Deadzones"), deadzone_rows(base, ModeTarget::Base, reopen, stick)),
            (
                crate::tr!("Outer Ring"),
                super::input_profile_stick_ring::outer_ring_rows(
                    base,
                    ModeTarget::Base,
                    reopen,
                    stick,
                ),
            ),
        ],
        other => vec![(
            crate::tr!("Response"),
            super::input_profile_source_modes::mode_setting_rows(
                base,
                ModeTarget::Base,
                other,
                reopen,
            ),
        )],
    }
}

/// Flat rows for a shift expander, where a shifted stick mode is edited
/// inline instead of in groups.
pub(crate) fn joystick_rows(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    processing: &StickProcessing,
) -> Vec<gtk4::ListBoxRow> {
    sensitivity_rows(base, target, reopen, processing)
        .into_iter()
        .chain(output_rows(base, target, processing, None))
        .chain(deadzone_rows(base, target, reopen, processing))
        .chain(super::input_profile_stick_ring::outer_ring_rows(
            base, target, reopen, processing,
        ))
        .collect()
}

/// The processing settings a mode carries, whichever behavior they belong
/// to — the row writers are shared between Joystick and Joystick Mouse.
pub(super) fn processing_of(mode: &mut SourceMode) -> Option<&mut StickProcessing> {
    match mode {
        SourceMode::Joystick(settings) => Some(&mut settings.processing),
        SourceMode::Mouse { stick, .. } => Some(stick),
        _ => None,
    }
}

/// Joystick Mouse's General group: pointer speed plus the shared stick
/// response rows.
fn mouse_general_rows(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    sensitivity: f32,
    processing: &StickProcessing,
) -> Vec<gtk4::ListBoxRow> {
    let mut rows = vec![mode_slider_row(
        base,
        target,
        &crate::tr!("Mouse Sensitivity"),
        Some(&crate::tr!("How fast the pointer moves per stick motion")),
        &SliderSpec(0.05, 20.0, 0.05, f64::from(sensitivity)),
        format_percent,
        |mode, value| {
            if let SourceMode::Mouse { sensitivity, .. } = mode {
                *sensitivity = value as f32;
            }
        },
    )];
    rows.extend(sensitivity_rows(base, target, reopen, processing));
    rows
}

fn sensitivity_rows(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    processing: &StickProcessing,
) -> Vec<gtk4::ListBoxRow> {
    let mut rows = vec![
        mode_slider_row(
            base,
            target,
            &crate::tr!("Horizontal Scale"),
            Some(&crate::tr!("Scales the stick's horizontal output")),
            &SliderSpec(0.0, 2.0, 0.05, f64::from(processing.sensitivity_x)),
            format_percent,
            |mode, value| {
                if let Some(processing) = processing_of(mode) {
                    processing.sensitivity_x = value as f32;
                }
            },
        ),
        mode_slider_row(
            base,
            target,
            &crate::tr!("Vertical Scale"),
            Some(&crate::tr!("Scales the stick's vertical output")),
            &SliderSpec(0.0, 2.0, 0.05, f64::from(processing.sensitivity_y)),
            format_percent,
            |mode, value| {
                if let Some(processing) = processing_of(mode) {
                    processing.sensitivity_y = value as f32;
                }
            },
        ),
        mode_switch_row(
            base,
            target,
            &crate::tr!("Invert Horizontal Axis"),
            None,
            processing.invert_x,
            |mode, enabled| {
                if let Some(processing) = processing_of(mode) {
                    processing.invert_x = enabled;
                }
            },
        )
        .upcast(),
        mode_switch_row(
            base,
            target,
            &crate::tr!("Invert Vertical Axis"),
            None,
            processing.invert_y,
            |mode, enabled| {
                if let Some(processing) = processing_of(mode) {
                    processing.invert_y = enabled;
                }
            },
        )
        .upcast(),
        curve_preset_row(base, target, reopen, processing.curve).upcast(),
        response_axis_style_row(base, target, processing.response_axis_style).upcast(),
    ];
    if curve_preset_index(processing.curve) == CURVE_CUSTOM_INDEX {
        rows.push(curve_slider_row(base, target, processing.curve));
    }
    rows
}

/// The Response Axis Style combo: curve each axis on its own, or the
/// deflection's distance from the deadzone.
fn response_axis_style_row(
    base: &SheetBase,
    target: ModeTarget,
    style: ResponseAxisStyle,
) -> adw::ComboRow {
    mode_combo_row(
        base,
        target,
        &crate::tr!("Response Axis Style"),
        Some(&crate::tr!(
            "Apply the response curve Per Axis, or based on the Distance from the Deadzone"
        )),
        &[crate::tr!("Distance from Deadzone"), crate::tr!("Per Axis")],
        axis_style_index(style),
        |mode, index| {
            if let Some(processing) = processing_of(mode) {
                processing.response_axis_style = axis_style_from_index(index);
            }
        },
    )
}

/// Output rows; `target_stick` is `Some` for the Joystick behavior, whose
/// deflection can drive either output stick.
fn output_rows(
    base: &SheetBase,
    target: ModeTarget,
    processing: &StickProcessing,
    target_stick: Option<StickOutput>,
) -> Vec<gtk4::ListBoxRow> {
    let mut rows = Vec::new();
    if let Some(output) = target_stick {
        rows.push(
            mode_combo_row(
                base,
                target,
                &crate::tr!("Output Joystick"),
                Some(&crate::tr!("Which stick the input drives")),
                &[crate::tr!("Left Joystick"), crate::tr!("Right Joystick")],
                output_index(output),
                |mode, index| {
                    if let SourceMode::Joystick(settings) = mode {
                        settings.output = output_from_index(index);
                    }
                },
            )
            .upcast(),
        );
    }
    rows.push(
        mode_combo_row(
            base,
            target,
            &crate::tr!("Output Axis"),
            Some(&crate::tr!(
                "Output can be limited to a single axis if desired"
            )),
            &[
                crate::tr!("Both Horizontal & Vertical"),
                crate::tr!("Horizontal Only"),
                crate::tr!("Vertical Only"),
            ],
            output_axis_index(processing.output_axis),
            |mode, index| {
                if let Some(processing) = processing_of(mode) {
                    processing.output_axis = output_axis_from_index(index);
                }
            },
        )
        .upcast(),
    );
    rows.push(mode_slider_row(
        base,
        target,
        &crate::tr!("Rotate Output"),
        Some(&crate::tr!(
            "Rotates the output, such that pushing the physical stick \"North\" results in an output of \"East\", when set to 90°"
        )),
        &SliderSpec(0.0, 360.0, 5.0, f64::from(processing.rotation)),
        format_degrees,
        |mode, value| {
            if let Some(processing) = processing_of(mode) {
                processing.rotation = value as f32;
            }
        },
    ));
    rows
}

fn deadzone_rows(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    processing: &StickProcessing,
) -> Vec<gtk4::ListBoxRow> {
    let mut rows = vec![deadzone_source_row(base, target, reopen, processing.deadzone).upcast()];
    if processing.deadzone == StickDeadzone::Custom {
        rows.push(mode_slider_row(
            base,
            target,
            &crate::tr!("Inner Deadzone"),
            Some(&crate::tr!(
                "Push the stick as far as this threshold before input is sent"
            )),
            &SliderSpec(0.0, 0.9, 0.01, f64::from(processing.deadzone_inner)),
            format_percent,
            |mode, value| {
                if let Some(processing) = processing_of(mode) {
                    processing.deadzone_inner = value as f32;
                }
            },
        ));
        rows.push(mode_slider_row(
            base,
            target,
            &crate::tr!("Outer Deadzone"),
            Some(&crate::tr!(
                "Push the stick this far to get the maximum output"
            )),
            &SliderSpec(0.1, 1.0, 0.01, f64::from(processing.deadzone_outer)),
            format_percent,
            |mode, value| {
                if let Some(processing) = processing_of(mode) {
                    processing.deadzone_outer = value as f32;
                }
            },
        ));
    }
    rows
}

/// The Deadzone Source row: Steam's described option popup over No Deadzone
/// / Controller Preference / Custom.
fn deadzone_source_row(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    deadzone: StickDeadzone,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Deadzone Source"));
    let choices = vec![
        OptionChoice {
            title: crate::tr!("No Deadzone"),
            description: Some(crate::tr!("The raw input of the joystick will be sent.")),
        },
        OptionChoice {
            title: crate::tr!("Controller Preference"),
            description: Some(crate::tr!(
                "The deadzone value comes from this specific controller's calibration, which you can set in the controller settings."
            )),
        },
        OptionChoice {
            title: crate::tr!("Custom"),
            description: Some(crate::tr!(
                "Set up a custom deadzone for this profile's configuration."
            )),
        },
    ];
    let current = deadzone_source_index(deadzone);
    let current_label = choices[current].title.clone();
    let base_for_pick = base.clone();
    let reopen_for_pick = reopen.clone();
    let picker = option_picker_popover(&choices, current, move |index| {
        let write = mode_writer(&base_for_pick, target);
        write(&mut |mode| {
            if let Some(processing) = processing_of(mode) {
                processing.deadzone = deadzone_from_index(index);
            }
        });
        (base_for_pick.on_changed)();
        // The rebuild reveals or removes the inner/outer sliders.
        reopen_for_pick();
    });
    row.add_suffix(&picker_button(&current_label, &picker));
    row
}

pub(super) fn write_processing(
    base: &SheetBase,
    target: ModeTarget,
    mutate: impl Fn(&mut StickProcessing),
) {
    let write = mode_writer(base, target);
    write(&mut |mode| {
        if let Some(processing) = processing_of(mode) {
            mutate(processing);
        }
    });
    (base.on_changed)();
}

/// One switch row bound to a bool field of the targeted SourceMode.
pub(super) fn mode_switch_row(
    base: &SheetBase,
    target: ModeTarget,
    title: &str,
    subtitle: Option<&str>,
    active: bool,
    mutate: fn(&mut SourceMode, bool),
) -> adw::SwitchRow {
    let base = std::rc::Rc::new(base.clone());
    switch_row(title, subtitle, active, move |enabled| {
        let write = mode_writer(&base, target);
        write(&mut |mode| mutate(mode, enabled));
        (base.on_changed)();
    })
}

/// One combo row bound to an enum field of the targeted SourceMode.
fn mode_combo_row(
    base: &SheetBase,
    target: ModeTarget,
    title: &str,
    subtitle: Option<&str>,
    labels: &[String],
    selected: usize,
    mutate: fn(&mut SourceMode, usize),
) -> adw::ComboRow {
    let combo = combo_row(labels, selected as u32);
    combo.set_title(title);
    if let Some(subtitle) = subtitle {
        combo.set_subtitle(subtitle);
    }
    let base = std::rc::Rc::new(base.clone());
    combo.connect_selected_notify(move |combo| {
        let write = mode_writer(&base, target);
        let index = combo.selected() as usize;
        write(&mut |mode| mutate(mode, index));
        (base.on_changed)();
    });
    combo
}
