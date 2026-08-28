//! Activator rows for the binding sheet: press patterns with timing
//! (picked from Steam's described-option popup), multi-output lists fed by
//! the tabbed command picker, and per-activator settings (toggle, repeat).

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::input_profile_activator_gate::activator_gate_controls;
use super::input_profile_editor_regions::{activator_kind_label, source_label};
use super::input_profile_options::output_display_label;
use super::input_profile_sheet_base::{is_trigger_axis, with_mapping, Reopen, SheetBase};
use super::input_profile_widgets::{
    option_picker_popover, picker_button, slider_row, OptionChoice,
    SliderSpec,
};
use adw::prelude::*;
use ira_input::{Activator, ActivatorKind, GamepadButton, InputMapping, OutputAction};

fn kind_index(kind: &ActivatorKind, soft_pull: bool) -> u32 {
    match kind {
        ActivatorKind::DoublePress { .. } => 1,
        ActivatorKind::LongPress { .. } => 2,
        ActivatorKind::StartPress => 3,
        ActivatorKind::Release => 4,
        // Off trigger sheets, where it cannot be picked, a soft pull falls
        // back to the Click entry rather than an out-of-range selection.
        ActivatorKind::SoftPress { .. } if soft_pull => 5,
        _ => 0,
    }
}

fn make_kind(index: u32, window_ms: u32, duration_ms: u32, threshold: f32) -> ActivatorKind {
    match index {
        1 => ActivatorKind::DoublePress { window_ms },
        2 => ActivatorKind::LongPress { duration_ms },
        3 => ActivatorKind::StartPress,
        4 => ActivatorKind::Release,
        5 => ActivatorKind::SoftPress { threshold },
        _ => ActivatorKind::FullPress,
    }
}

/// Press patterns with Steam-style descriptions; the trailing Soft Pull
/// entry only exists on trigger sheets.
fn kind_choices(soft_pull: bool) -> Vec<OptionChoice> {
    let mut choices = vec![
        OptionChoice {
            title: crate::tr!("Click"),
            description: Some(crate::tr!(
                "The regular activation: fires when the input is pressed"
            )),
        },
        OptionChoice {
            title: crate::tr!("Double press"),
            description: Some(crate::tr!(
                "Fires when the input is pressed twice in quick succession"
            )),
        },
        OptionChoice {
            title: crate::tr!("Long press"),
            description: Some(crate::tr!(
                "Fires when the input is held past the hold duration"
            )),
        },
        OptionChoice {
            title: crate::tr!("On press down"),
            description: Some(crate::tr!(
                "Fires the instant the input goes down, without waiting for the release"
            )),
        },
        OptionChoice {
            title: crate::tr!("On release"),
            description: Some(crate::tr!("Fires when the input is released")),
        },
    ];
    if soft_pull {
        choices.push(OptionChoice {
            title: crate::tr!("Soft pull"),
            description: Some(crate::tr!(
                "Fires when the trigger crosses the soft pull threshold, before the full pull"
            )),
        });
    }
    choices
}

/// The activator expanders plus the add row — the expander children of an
/// input row. Buttons and triggers carry these; plain analog modes do not.
pub(crate) fn activator_rows(
    base: &SheetBase,
    reopen: &Reopen,
    mapping: &InputMapping,
) -> Vec<gtk4::Widget> {
    let mut rows: Vec<gtk4::Widget> = mapping
        .activators
        .iter()
        .enumerate()
        .map(|(index, activator)| activator_expander(base, reopen, index, activator).upcast())
        .collect();

    // Full-width row rather than a floating button: a raw button as a
    // direct child of a preferences group looks wrong and trips GTK
    // widget assertions.
    let add_row = adw::ActionRow::new();
    let add = super::helpers::icon_label_button("list-add-symbolic", &crate::tr!("Add activator"));
    add_row.add_suffix(&add);
    add_row.set_activatable(true);
    rows.push(add_row.clone().upcast());
    {
        let base = base.clone();
        let reopen = reopen.clone();
        add_row.connect_activated(move |_| {
            with_mapping(&base, |input| {
                input
                    .activators
                    .push(Activator::full_press(vec![OutputAction::GamepadButton(
                        GamepadButton::A,
                    )]));
            });
            (base.on_changed)();
            reopen();
        });
    }
    rows
}

fn activator_expander(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    activator: &Activator,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::new();
    expander.set_title(&activator_kind_label(&activator.kind));
    let outputs_summary: Vec<String> = activator.outputs.iter().map(output_display_label).collect();
    expander.set_subtitle(&esc(&outputs_summary.join(", ")));
    activator_header_controls(base, reopen, index, &expander);
    activator_kind_controls(base, reopen, index, activator, &expander);
    activator_output_rows(base, reopen, index, activator, &expander);
    activator_gate_controls(base, reopen, index, activator, &expander);
    activator_setting_controls(base, index, activator, &expander);
    expander
}

fn activator_header_controls(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    expander: &adw::ExpanderRow,
) {
    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(CSS_FLAT);
    remove.add_css_class(CSS_SQUARE_BUTTON);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove activator")));
    expander.add_suffix(&remove);
    let base = base.clone();
    let reopen = reopen.clone();
    remove.connect_clicked(move |_| {
        with_mapping(&base, |input| {
            if index < input.activators.len() {
                input.activators.remove(index);
            }
        });
        (base.on_changed)();
        reopen();
    });
}

fn activator_kind_controls(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    activator: &Activator,
    expander: &adw::ExpanderRow,
) {
    let soft_pull = is_trigger_axis(base.source);
    let timing_window = match &activator.kind {
        ActivatorKind::DoublePress { window_ms } => *window_ms,
        _ => 320,
    };
    let timing_duration = match &activator.kind {
        ActivatorKind::LongPress { duration_ms } => *duration_ms,
        _ => 600,
    };
    let soft_threshold = match &activator.kind {
        ActivatorKind::SoftPress { threshold } => *threshold,
        _ => 0.5,
    };

    let choices = kind_choices(soft_pull);
    let selected = kind_index(&activator.kind, soft_pull) as usize;
    let current_label = choices
        .get(selected)
        .map(|choice| choice.title.clone())
        .unwrap_or_else(|| crate::tr!("Click"));
    let kind_row = adw::ActionRow::new();
    kind_row.set_title(&crate::tr!("Press pattern"));
    kind_row.set_subtitle(&crate::tr!("What kind of press activates this"));
    let base_for_kind = base.clone();
    let reopen_for_kind = reopen.clone();
    let picker = option_picker_popover(&choices, selected, move |picked| {
        with_mapping(&base_for_kind, |input| {
            if let Some(activator) = input.activators.get_mut(index) {
                activator.kind = make_kind(
                    picked as u32,
                    timing_window,
                    timing_duration,
                    soft_threshold,
                );
            }
        });
        (base_for_kind.on_changed)();
        reopen_for_kind();
    });
    kind_row.add_suffix(&picker_button(&current_label, &picker));
    expander.add_row(&kind_row);

    if let ActivatorKind::DoublePress { window_ms } = &activator.kind {
        let base = base.clone();
        expander.add_row(&timing_slider(
            &crate::tr!("Double-press window"),
            Some(&crate::tr!("Time allowed between the two presses")),
            &SliderSpec(100.0, 1000.0, 10.0, f64::from(*window_ms)),
            move |value| write_kind_value(&base, index, value, set_double_press_window),
        ));
    }
    if let ActivatorKind::LongPress { duration_ms } = &activator.kind {
        let base = base.clone();
        expander.add_row(&timing_slider(
            &crate::tr!("Hold duration"),
            Some(&crate::tr!(
                "How long the input must be held before it fires"
            )),
            &SliderSpec(200.0, 2000.0, 25.0, f64::from(*duration_ms)),
            move |value| write_kind_value(&base, index, value, set_long_press_duration),
        ));
    }
    if let ActivatorKind::SoftPress { threshold } = &activator.kind {
        let base = base.clone();
        expander.add_row(&slider_row(
            &crate::tr!("Soft pull threshold"),
            Some(&crate::tr!("Trigger travel that fires the soft pull")),
            &SliderSpec(0.05, 0.95, 0.05, f64::from(*threshold)),
            move |value| write_kind_value(&base, index, value, set_soft_pull_threshold)));
    }
}

fn timing_slider(
    title: &str,
    subtitle: Option<&str>,
    spec: &SliderSpec,
    on_change: impl Fn(f64) + 'static,
) -> gtk4::ListBoxRow {
    slider_row(title, subtitle, spec, on_change)
}

fn set_double_press_window(kind: &mut ActivatorKind, value: f64) {
    if let ActivatorKind::DoublePress { window_ms } = kind {
        *window_ms = value as u32;
    }
}

fn set_long_press_duration(kind: &mut ActivatorKind, value: f64) {
    if let ActivatorKind::LongPress { duration_ms } = kind {
        *duration_ms = value as u32;
    }
}

fn set_soft_pull_threshold(kind: &mut ActivatorKind, value: f64) {
    if let ActivatorKind::SoftPress { threshold } = kind {
        *threshold = value as f32;
    }
}

/// Writes one activator's kind-specific field from a slider and reports
/// the change.
fn write_kind_value(base: &SheetBase, index: usize, value: f64, set: fn(&mut ActivatorKind, f64)) {
    with_mapping(base, |input| {
        if let Some(activator) = input.activators.get_mut(index) {
            set(&mut activator.kind, value);
        }
    });
    (base.on_changed)();
}

fn activator_output_rows(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    activator: &Activator,
    expander: &adw::ExpanderRow,
) {
    for (output_at, output) in activator.outputs.iter().enumerate() {
        let output_row = adw::ActionRow::new();
        output_row.set_title(&esc(&output_display_label(output)));
        let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
        trash.add_css_class(CSS_FLAT);
        trash.add_css_class(CSS_SQUARE_BUTTON);
        trash.set_valign(gtk4::Align::Center);
        output_row.add_suffix(&trash);
        let base = base.clone();
        let reopen = reopen.clone();
        trash.connect_clicked(move |_| {
            with_mapping(&base, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    if output_at < activator.outputs.len() {
                        activator.outputs.remove(output_at);
                    }
                }
            });
            (base.on_changed)();
            reopen();
        });
        expander.add_row(&output_row);
    }

    // Steam-style picker for new outputs, including keyboard, wheel clicks
    // and action-set commands.
    let add_output = gtk4::Button::with_label(&crate::tr!("Add output…"));
    add_output.set_valign(gtk4::Align::Center);
    let add_row = adw::ActionRow::new();
    add_row.set_title(&crate::tr!("Add output"));
    add_row.add_suffix(&add_output);
    expander.add_row(&add_row);
    let base = base.clone();
    let reopen = reopen.clone();
    add_output.connect_clicked(move |button| {
        let Some(parent) = button.root().and_downcast::<gtk4::Window>() else {
            return;
        };
        let scope = picker_scope(&base);
        let base = base.clone();
        let reopen = reopen.clone();
        show_output_picker(
            &parent,
            &source_label(base.source),
            &scope,
            None,
            move |action| {
                with_mapping(&base, |input| {
                    if let Some(activator) = input.activators.get_mut(index) {
                        activator.outputs.push(action.clone());
                    }
                });
                (base.on_changed)();
                reopen();
            },
        );
    });
}

/// The picker sees the profile's real action set and layer names so its
/// Action Sets tab can target them.
fn picker_scope(base: &SheetBase) -> OutputPickerScope {
    let borrow = base.profile.borrow();
    OutputPickerScope {
        backend: base.backend,
        set_names: borrow
            .action_sets
            .iter()
            .map(|set| set.name.clone())
            .collect(),
        layer_names: borrow
            .action_layers
            .iter()
            .map(|layer| layer.name.clone())
            .collect(),
    }
}

fn activator_setting_controls(
    base: &SheetBase,
    index: usize,
    activator: &Activator,
    expander: &adw::ExpanderRow,
) {
    let toggle = gtk4::Switch::new();
    toggle.set_active(activator.settings.toggle);
    toggle.set_valign(gtk4::Align::Center);
    let toggle_row = adw::ActionRow::new();
    toggle_row.set_title(&crate::tr!("Toggle instead of hold"));
    toggle_row.add_suffix(&toggle);
    expander.add_row(&toggle_row);
    let base_for_toggle = base.clone();
    toggle.connect_active_notify(move |switch| {
        with_mapping(&base_for_toggle, |input| {
            if let Some(activator) = input.activators.get_mut(index) {
                activator.settings.toggle = switch.is_active();
            }
        });
        (base_for_toggle.on_changed)();
    });

    if matches!(activator.kind, ActivatorKind::FullPress) {
        let repeat_ms = activator
            .settings
            .repeat_rate_ms
            .map(f64::from)
            .unwrap_or(0.0);
        let base_for_repeat = base.clone();
        expander.add_row(&slider_row(
            &crate::tr!("Repeat every"),
            Some(&crate::tr!("Re-fires the outputs while the input is held")),
            &SliderSpec(0.0, 1000.0, 50.0, repeat_ms),
            move |value| {
                with_mapping(&base_for_repeat, |input| {
                    if let Some(activator) = input.activators.get_mut(index) {
                        activator.settings.repeat_rate_ms = if value >= 50.0 {
                            Some(value as u32)
                        } else {
                            None
                        };
                    }
                });
                (base_for_repeat.on_changed)();
            }));
    }
}
