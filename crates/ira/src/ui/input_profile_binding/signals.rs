use super::super::input_profile_options::{
    output_display_label, output_index, output_option, OutputOption,
};
use super::super::input_profile_output_capture::{
    show_keyboard_output_capture, show_mouse_output_capture,
};
use super::assets::{set_source_asset, source_badge};
use super::categories::{is_analog_source, uses_gyro_stick_output};
use super::types::{BindingRow, OutputChangeContext, SourceChangeContext};
use adw::prelude::*;
use ira_input::{InputSource, OutputAction};
use std::rc::Rc;

pub(super) fn connect_dirty(row: &BindingRow, on_dirty: &Rc<dyn Fn()>) {
    row.source.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.output.connect_selected_notify({
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
    row.recenter.connect_selected_notify({
        let on_dirty = on_dirty.clone();
        move |_| on_dirty()
    });
    row.gyro_mode.connect_selected_notify({
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
            context
                .recenter_row
                .set_visible(matches!(*source, InputSource::Gyro(_)));
            context
                .gyro_mode_row
                .set_visible(uses_gyro_stick_output(*source, &context.output.borrow()));
        }
    });
}

pub(super) fn connect_output_changes(dropdown: &gtk4::DropDown, context: OutputChangeContext) {
    dropdown.connect_selected_notify(move |dropdown| {
        let Some(option) = output_option(dropdown.selected(), context.backend) else {
            return;
        };
        match option {
            OutputOption::Action(action) => *context.output.borrow_mut() = action,
            capture_option => {
                dropdown.set_selected(output_index(&context.output.borrow(), context.backend));
                if let Some(parent) = dropdown.root().and_downcast::<gtk4::Window>() {
                    show_output_capture(capture_option, &parent, context.clone());
                }
                return;
            }
        }
        update_output_state(&context);
    });
}

fn show_output_capture(option: OutputOption, parent: &gtk4::Window, context: OutputChangeContext) {
    let complete: Rc<dyn Fn(OutputAction)> = Rc::new(move |action| {
        *context.output.borrow_mut() = action;
        update_output_state(&context);
    });
    match option {
        OutputOption::CaptureKeyboard => {
            show_keyboard_output_capture(parent, move |keycode| {
                complete(OutputAction::Keyboard { keycode });
            });
        }
        OutputOption::CaptureMouseButton => {
            show_mouse_output_capture(parent, move |button| {
                complete(OutputAction::MouseButton(button));
            });
        }
        OutputOption::Action(_) => {}
    }
}

fn update_output_state(context: &OutputChangeContext) {
    update_binding_summary(&context.source, &context.output.borrow(), &context.row);
    if let Some((source, _)) = context
        .source_options
        .get(context.source.selected() as usize)
    {
        context
            .gyro_mode_row
            .set_visible(uses_gyro_stick_output(*source, &context.output.borrow()));
    }
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
    row.set_title(&source_text);
    if output_text == source_text {
        row.set_subtitle("");
    } else {
        row.set_subtitle(&format!("→ {output_text}"));
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
