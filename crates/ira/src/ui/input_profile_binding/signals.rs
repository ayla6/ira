use super::super::helpers::esc;
use super::super::input_profile_options::output_display_label;
use super::super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::assets::{set_source_asset, source_badge};
use super::categories::is_analog_source;
use super::types::{BindingRow, OutputChangeContext, SourceChangeContext};
use adw::prelude::*;
use ira_input::OutputAction;
use std::rc::Rc;

pub(super) fn connect_dirty(row: &BindingRow, on_dirty: &Rc<dyn Fn()>) {
    row.source.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.activation.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.activator.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.chord.connect_changed({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    for spin in [&row.dead_zone, &row.sensitivity, &row.exponent] {
        spin.connect_value_changed({
            let on_dirty = on_dirty.clone();
            move |_| on_dirty()
        });
    }
    row.invert.connect_toggled({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
}

pub(super) fn connect_source_changes(dropdown: &gtk4::DropDown, context: SourceChangeContext) {
    dropdown.connect_selected_notify(move |dropdown| {
        update_binding_summary(dropdown, &context.output.borrow(), &context.row);
        if let Some((source, _)) = context.source_options.get(dropdown.selected() as usize) {
            context.current_source.set(*source);
            context
                .fallback
                .set_text(&source_badge(*source, context.family));
            set_source_asset(&context.asset, &context.fallback, *source, context.family);
            let analog = is_analog_source(*source);
            context.dead_zone_row.set_visible(analog);
            context.sensitivity_row.set_visible(analog);
            context.exponent_row.set_visible(analog);
            context.invert_row.set_visible(analog);
        }
    });
}

pub(super) fn connect_output_changes(context: OutputChangeContext) {
    context.output_button.connect_clicked({
        let context = context.clone();
        move |button| {
            let Some(parent) = button.root().and_downcast::<gtk4::Window>() else {
                return;
            };
            let source_text = source_label(&context.source);
            let scope = OutputPickerScope::flat(context.backend);
            let current = context.output.borrow().clone();
            let handler = context.clone();
            show_output_picker(
                &parent,
                &crate::tr!("Select a command for {input}").replace("{input}", &source_text),
                &scope,
                Some(&current),
                move |action| {
                    *handler.output.borrow_mut() = action;
                    update_output_state(&handler);
                },
            );
        }
    });
}

fn source_label(source: &gtk4::DropDown) -> String {
    source
        .model()
        .and_then(|model| model.item(source.selected()))
        .and_then(|item| item.downcast::<gtk4::StringObject>().ok())
        .map(|item| item.string().to_string())
        .unwrap_or_else(|| crate::tr!("input").to_string())
}

fn update_output_state(context: &OutputChangeContext) {
    context
        .output_button
        .set_label(&output_display_label(&context.output.borrow()));
    update_binding_summary(
        &context.source,
        &context.output.borrow(),
        &context.row,
    );
    (context.on_dirty)();
}

pub(super) fn update_binding_summary(
    source: &gtk4::DropDown,
    output: &OutputAction,
    row: &adw::ExpanderRow,
) {
    let source_text = source
        .model()
        .and_then(|model| model.item(source.selected()))
        .and_then(|item| item.downcast::<gtk4::StringObject>().ok())
        .map(|item| item.string().to_string())
        .unwrap_or_else(|| "Input".to_string());
    let output_text = output_display_label(output);
    row.set_title(&esc(&source_text));
    if output_text == source_text {
        row.set_subtitle("");
    } else {
        row.set_subtitle(&crate::tr!("→ {output_text}").replace("{output_text}", &esc(&output_text)));
    }
}

pub(super) fn update_activation_controls(
    activation: &gtk4::DropDown,
    activator: &gtk4::DropDown,
    activator_row: &adw::ActionRow,
    chord: &adw::EntryRow,
) {
    let is_always = activation.selected() == 0;
    let is_chord = activation.selected() == 4;
    let show_activator = !is_always && !is_chord;
    activator.set_sensitive(show_activator);
    activator_row.set_visible(show_activator);
    chord.set_sensitive(is_chord);
    chord.set_visible(is_chord);
}
