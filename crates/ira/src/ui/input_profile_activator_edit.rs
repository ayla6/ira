//! Activator rows for the binding sheet: press patterns with timing,
//! multi-output lists fed by the tabbed command picker, and per-activator
//! settings (toggle, repeat).

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::input_profile_activator_gate::activator_gate_controls;
use super::input_profile_activator_sheet::{
    combo_row, spin_row, with_mapping, Reopen, SheetBase,
};
use super::input_profile_editor_regions::{activator_kind_label, source_label};
use super::input_profile_options::output_display_label;
use adw::prelude::*;
use ira_input::{Activator, ActivatorKind, GamepadButton, InputMapping, OutputAction};
use std::rc::Rc;

fn kind_index(kind: &ActivatorKind) -> u32 {
    match kind {
        ActivatorKind::DoublePress { .. } => 1,
        ActivatorKind::LongPress { .. } => 2,
        ActivatorKind::StartPress => 3,
        ActivatorKind::Release => 4,
        _ => 0,
    }
}

fn make_kind(index: u32, window_ms: u32, duration_ms: u32) -> ActivatorKind {
    match index {
        1 => ActivatorKind::DoublePress { window_ms },
        2 => ActivatorKind::LongPress { duration_ms },
        3 => ActivatorKind::StartPress,
        4 => ActivatorKind::Release,
        _ => ActivatorKind::FullPress,
    }
}

fn kind_labels() -> Vec<String> {
    [
        crate::tr!("Click"),
        crate::tr!("Double press"),
        crate::tr!("Long press"),
        crate::tr!("On press down"),
        crate::tr!("On release"),
    ]
    .into_iter()
    .collect()
}

pub(crate) fn activators_group(
    base: &SheetBase,
    reopen: &Reopen,
    mapping: &InputMapping,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Activators"));
    group.set_description(Some(&crate::tr!(
        "Different actions by how the input is pressed"
    )));

    for (index, activator) in mapping.activators.iter().enumerate() {
        group.add(&activator_expander(base, reopen, index, activator));
    }

    // Full-width row rather than a floating button: a raw button as a
    // direct child of a preferences group looks wrong and trips GTK
    // widget assertions.
    let add_row = adw::ActionRow::new();
    let add =
        super::helpers::icon_label_button("list-add-symbolic", &crate::tr!("Add activator"));
    add_row.add_suffix(&add);
    add_row.set_activatable(true);
    group.add(&add_row);
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
    group
}

fn activator_expander(
    base: &SheetBase,
    reopen: &Reopen,
    index: usize,
    activator: &Activator,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::new();
    expander.set_title(&activator_kind_label(&activator.kind));
    let outputs_summary: Vec<String> = activator
        .outputs
        .iter()
        .map(output_display_label)
        .collect();
    expander.set_subtitle(&esc(&outputs_summary.join(", ")));
    activator_header_controls(base, reopen, index, &expander);
    activator_kind_controls(base, reopen, index, activator, &expander);
    activator_output_rows(base, reopen, index, activator, &expander);
    activator_gate_controls(base, reopen, index, activator, &expander);
    activator_setting_controls(base, index, activator, &expander);
    expander
}

fn activator_header_controls(base: &SheetBase, reopen: &Reopen, index: usize, expander: &adw::ExpanderRow) {
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
    let timing_window = match &activator.kind {
        ActivatorKind::DoublePress { window_ms } => *window_ms,
        _ => 320,
    };
    let timing_duration = match &activator.kind {
        ActivatorKind::LongPress { duration_ms } => *duration_ms,
        _ => 600,
    };
    let kind = combo_row(&kind_labels(), kind_index(&activator.kind));
    kind.set_title(&crate::tr!("Press pattern"));
    expander.add_row(&kind);
    let base_for_kind = base.clone();
    let reopen = reopen.clone();
    kind.connect_selected_notify(move |dropdown| {
        with_mapping(&base_for_kind, |input| {
            if let Some(activator) = input.activators.get_mut(index) {
                activator.kind = make_kind(dropdown.selected(), timing_window, timing_duration);
            }
        });
        (base_for_kind.on_changed)();
        reopen();
    });

    if let ActivatorKind::DoublePress { window_ms } = &activator.kind {
        let base = base.clone();
        expander.add_row(&spin_row(
            &crate::tr!("Double-press window"),
            100.0,
            1000.0,
            10.0,
            f64::from(*window_ms),
            Rc::new(move |value| {
                with_mapping(&base, |input| {
                    if let Some(ActivatorKind::DoublePress { window_ms }) = input
                        .activators
                        .get_mut(index)
                        .map(|activator| &mut activator.kind)
                    {
                        *window_ms = value as u32;
                    }
                });
                (base.on_changed)();
            }),
        ));
    }
    if let ActivatorKind::LongPress { duration_ms } = &activator.kind {
        let base = base.clone();
        expander.add_row(&spin_row(
            &crate::tr!("Hold duration"),
            200.0,
            2000.0,
            25.0,
            f64::from(*duration_ms),
            Rc::new(move |value| {
                with_mapping(&base, |input| {
                    if let Some(ActivatorKind::LongPress { duration_ms }) = input
                        .activators
                        .get_mut(index)
                        .map(|activator| &mut activator.kind)
                    {
                        *duration_ms = value as u32;
                    }
                });
                (base.on_changed)();
            }),
        ));
    }
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
        let repeat_ms = activator.settings.repeat_rate_ms.map(f64::from).unwrap_or(0.0);
        let base_for_repeat = base.clone();
        expander.add_row(&spin_row(
            &crate::tr!("Repeat every"),
            0.0,
            1000.0,
            50.0,
            repeat_ms,
            Rc::new(move |value| {
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
            }),
        ));
    }
}
