//! Mode shifts: while the chosen trigger is held, the input uses a
//! different behavior. Axis sheets get a full expander per shift (shifted
//! mode picker plus its response rows); button sheets keep the plain
//! removal rows for shifts that arrive through VDF import.

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_editor_regions::source_label;
use super::input_profile_sheet_base::{
    combo_row, find_mapping, is_trigger_axis, with_mapping, Reopen, SheetBase,
};
use super::input_profile_source_modes::{
    mode_label, mode_setting_rows, modes_for, same_mode, ModeTarget,
};
use super::input_profile_widgets::SettingGroup;
use adw::prelude::*;
use ira_input::{InputSource, ModeShift};

pub(crate) fn shifts_group(base: &SheetBase, reopen: &Reopen) -> gtk4::Box {
    let group = SettingGroup::new(
        Some(&crate::tr!("Mode shifts")),
        Some(&crate::tr!(
            "While the shift button is held, this input uses the shifted behavior"
        )),
    );

    if let Some(mapping) = find_mapping(base) {
        for (shift_index, shift) in mapping.mode_shifts.iter().enumerate() {
            group.add(&shift_row(base, reopen, shift_index, shift));
        }
    }

    let sources: Vec<(InputSource, String)> =
        super::input_profile_editor_regions::supported_button_sources(base.device.as_ref())
            .into_iter()
            .map(|source| (source, source_label(source)))
            .collect();
    let labels: Vec<String> = sources.iter().map(|(_, label)| label.clone()).collect();
    let picker = combo_row(&labels, 0);
    picker.set_title(&crate::tr!("Shift while holding"));
    let add = gtk4::Button::with_label(&crate::tr!("Add shift"));
    add.add_css_class(CSS_FLAT);
    add.set_valign(gtk4::Align::Center);
    picker.add_suffix(&add);
    group.add(&picker);
    let base = base.clone();
    let reopen = reopen.clone();
    add.connect_clicked(move |_| {
        if let Some((trigger, _)) = sources.get(picker.selected() as usize) {
            let trigger = *trigger;
            with_mapping(&base, |input| {
                input.mode_shifts.push(ModeShift {
                    trigger,
                    mode: None,
                    activators: Vec::new(),
                });
            });
            (base.on_changed)();
            reopen();
        }
    });

    group.root
}

fn shift_row(
    base: &SheetBase,
    reopen: &Reopen,
    shift_index: usize,
    shift: &ModeShift,
) -> gtk4::Widget {
    if matches!(base.source, InputSource::Axis(_)) {
        shift_expander(base, reopen, shift_index, shift).upcast()
    } else {
        plain_shift_row(base, reopen, shift_index, shift).upcast()
    }
}

/// Axis inputs carry a mode, so their shifts can be edited in full: pick
/// the shifted behavior and tune its response right here.
fn shift_expander(
    base: &SheetBase,
    reopen: &Reopen,
    shift_index: usize,
    shift: &ModeShift,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::new();
    expander.set_title(&esc(&format!(
        "{} {}",
        crate::tr!("While holding"),
        source_label(shift.trigger)
    )));
    let is_trigger = is_trigger_axis(base.source);
    expander.set_subtitle(&esc(&match &shift.mode {
        Some(mode) => mode_label(&Some(mode.clone()), is_trigger),
        None => crate::tr!("Same behavior as base"),
    }));
    expander.add_suffix(&remove_shift_button(base, reopen, shift_index));

    let modes = modes_for(base.source);
    let selected = shift
        .mode
        .as_ref()
        .and_then(|mode| {
            modes
                .iter()
                .position(|candidate| same_mode(candidate, mode))
        })
        .unwrap_or(0);
    let mut labels: Vec<String> = vec![crate::tr!("Keep base behavior")];
    labels.extend(
        modes
            .iter()
            .skip(1)
            .map(|mode| mode_label(mode, is_trigger)),
    );
    let combo = combo_row(&labels, selected as u32);
    combo.set_title(&crate::tr!("Shifted behavior"));
    let base_for_combo = base.clone();
    let reopen_for_combo = reopen.clone();
    combo.connect_selected_notify(move |dropdown| {
        let mode = modes.get(dropdown.selected() as usize).cloned().flatten();
        with_mapping(&base_for_combo, |input| {
            if let Some(shift) = input.mode_shifts.get_mut(shift_index) {
                shift.mode = mode;
            }
        });
        (base_for_combo.on_changed)();
        reopen_for_combo();
    });
    expander.add_row(&combo);

    if let Some(mode) = &shift.mode {
        for row in mode_setting_rows(base, ModeTarget::Shift(shift_index), mode, reopen) {
            expander.add_row(&row);
        }
    }
    expander
}

/// Button inputs have no mode; imported shift-activator overrides only
/// need a face for inspection and removal.
fn plain_shift_row(
    base: &SheetBase,
    reopen: &Reopen,
    shift_index: usize,
    shift: &ModeShift,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&format!(
        "{} {}",
        crate::tr!("While holding"),
        source_label(shift.trigger)
    )));
    row.set_subtitle(&esc(&crate::tr!("Shifted activators")));
    row.add_suffix(&remove_shift_button(base, reopen, shift_index));
    row
}

fn remove_shift_button(base: &SheetBase, reopen: &Reopen, shift_index: usize) -> gtk4::Button {
    let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
    trash.add_css_class(CSS_FLAT);
    trash.add_css_class(CSS_SQUARE_BUTTON);
    trash.set_valign(gtk4::Align::Center);
    trash.set_tooltip_text(Some(&crate::tr!("Remove shift")));
    let base = base.clone();
    let reopen = reopen.clone();
    trash.connect_clicked(move |_| {
        with_mapping(&base, |input| {
            if shift_index < input.mode_shifts.len() {
                input.mode_shifts.remove(shift_index);
            }
        });
        (base.on_changed)();
        reopen();
    });
    trash
}
