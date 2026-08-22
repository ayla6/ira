//! Whole-controller gyro configuration card for the editor's Gyro page.
//!
//! Replaces the old per-axis gyro bindings: one switch, one activation rule,
//! one output, a sensitivity multiplier, and the smoothing toggle — mirroring
//! the Steam Input gyro panel instead of three hand-wired bindings.

use super::input_profile_options::source_options_for_device;
use adw::prelude::*;
use ira_input::{
    DeviceInfo, GamepadButton, GyroActivation, GyroConfig, GyroOrientation, GyroOutput,
    InputSource, VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct GyroWidgets {
    enable_row: adw::ActionRow,
    enable: gtk4::Switch,
    activation_row: adw::ActionRow,
    activation: gtk4::DropDown,
    button_row: adw::ActionRow,
    button: gtk4::DropDown,
    output_row: adw::ActionRow,
    output: gtk4::DropDown,
    orientation_row: adw::ActionRow,
    orientation: gtk4::DropDown,
    sensitivity_row: adw::ActionRow,
    sensitivity: gtk4::SpinButton,
    invert_x_row: adw::ActionRow,
    invert_x: gtk4::Switch,
    invert_y_row: adw::ActionRow,
    invert_y: gtk4::Switch,
    smoothing_row: adw::ActionRow,
    smoothing: gtk4::Switch,
}

pub(super) fn add_gyro_group(
    page: &gtk4::Box,
    gyro: &Rc<RefCell<GyroConfig>>,
    device: Option<&DeviceInfo>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Gyro"));
    group.set_description(Some(&crate::tr!(
        "Rotating the controller steers the output. Yaw and pitch are measured relative to gravity, so aiming stays consistent no matter how the controller is held."
    )));

    let button_options = activation_button_options(device, &gyro.borrow().activation);
    let (enable_row, enable) = switch_row(&crate::tr!("Enable gyro"), gyro.borrow().enabled);
    let (activation_row, activation) = dropdown_row(
        &crate::tr!("Activation"),
        &activation_labels(),
        activation_index(&gyro.borrow().activation),
    );
    let (button_row, button) = dropdown_row(
        &crate::tr!("Activation button"),
        &button_option_labels(&button_options),
        activation_button_index(&button_options, &gyro.borrow().activation),
    );
    let (output_row, output) =
        dropdown_row(&crate::tr!("Output"), &output_labels(), output_index(gyro.borrow().output));
    let (orientation_row, orientation) = dropdown_row(
        &crate::tr!("Orientation"),
        &orientation_labels(),
        orientation_index(gyro.borrow().orientation),
    );
    let (sensitivity_row, sensitivity) = sensitivity_row(gyro.borrow().sensitivity);
    let (invert_x_row, invert_x) =
        switch_row(&crate::tr!("Invert horizontal"), gyro.borrow().invert_x);
    let (invert_y_row, invert_y) = switch_row(&crate::tr!("Invert vertical"), gyro.borrow().invert_y);
    let (smoothing_row, smoothing) = switch_row(&crate::tr!("Smoothing"), gyro.borrow().smoothing);
    let widgets = GyroWidgets {
        enable_row,
        enable,
        activation_row,
        activation,
        button_row,
        button,
        output_row,
        output,
        orientation_row,
        orientation,
        sensitivity_row,
        sensitivity,
        invert_x_row,
        invert_x,
        invert_y_row,
        invert_y,
        smoothing_row,
        smoothing,
    };
    widgets
        .smoothing_row
        .set_subtitle(&crate::tr!("Damps jitter while aiming slowly; flicks stay untouched"));
    update_dependency_rows(&widgets, gyro.borrow().enabled);

    for row in [
        &widgets.enable_row,
        &widgets.activation_row,
        &widgets.button_row,
        &widgets.output_row,
        &widgets.orientation_row,
        &widgets.sensitivity_row,
        &widgets.invert_x_row,
        &widgets.invert_y_row,
        &widgets.smoothing_row,
    ] {
        group.add(row);
    }
    page.append(&group);
    connect_gyro_changes(&widgets, &button_options, gyro, on_dirty);
}

fn connect_gyro_changes(
    widgets: &GyroWidgets,
    button_options: &[(InputSource, String)],
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let widgets_for_enable = widgets.clone();
    let gyro_for_enable = gyro.clone();
    let on_dirty_for_enable = on_dirty.clone();
    widgets.enable.connect_active_notify(move |switch| {
        gyro_for_enable.borrow_mut().enabled = switch.is_active();
        update_dependency_rows(&widgets_for_enable, switch.is_active());
        on_dirty_for_enable();
    });

    let gyro_for_activation = gyro.clone();
    let on_dirty_for_activation = on_dirty.clone();
    let button_options_for_activation = button_options.to_vec();
    let widgets_for_activation = widgets.clone();
    widgets.activation.connect_selected_notify(move |dropdown| {
        let fallback_button = button_options_for_activation
            .get(widgets_for_activation.button.selected() as usize)
            .and_then(|(source, _)| match source {
                InputSource::Button(button) => Some(*button),
                _ => None,
            })
            .unwrap_or(GamepadButton::LeftTrigger);
        let mut gyro = gyro_for_activation.borrow_mut();
        gyro.activation = match dropdown.selected() {
            1 => GyroActivation::Hold(fallback_button),
            2 => GyroActivation::Toggle(fallback_button),
            _ => GyroActivation::Always,
        };
        drop(gyro);
        widgets_for_activation
            .button_row
            .set_visible(dropdown.selected() != 0);
        on_dirty_for_activation();
    });

    let gyro_for_button = gyro.clone();
    let on_dirty_for_button = on_dirty.clone();
    let activation_for_button = widgets.activation.clone();
    let button_options_for_button = button_options.to_vec();
    widgets.button.connect_selected_notify(move |dropdown| {
        let Some(InputSource::Button(button)) = button_options_for_button
            .get(dropdown.selected() as usize)
            .map(|(source, _)| *source)
        else {
            return;
        };
        let mut gyro = gyro_for_button.borrow_mut();
        gyro.activation = match (gyro.activation, activation_for_button.selected()) {
            (GyroActivation::Hold(_), 1) => GyroActivation::Hold(button),
            (GyroActivation::Toggle(_), 2) => GyroActivation::Toggle(button),
            (current, _) => current,
        };
        drop(gyro);
        on_dirty_for_button();
    });

    let gyro_for_output = gyro.clone();
    let on_dirty_for_output = on_dirty.clone();
    widgets.output.connect_selected_notify(move |dropdown| {
        gyro_for_output.borrow_mut().output = match dropdown.selected() {
            1 => GyroOutput::LeftStick,
            2 => GyroOutput::RightStick,
            _ => GyroOutput::Mouse,
        };
        on_dirty_for_output();
    });

    let gyro_for_orientation = gyro.clone();
    let on_dirty_for_orientation = on_dirty.clone();
    widgets.orientation.connect_selected_notify(move |dropdown| {
        gyro_for_orientation.borrow_mut().orientation = match dropdown.selected() {
            0 => GyroOrientation::Local,
            1 => GyroOrientation::Yaw,
            2 => GyroOrientation::Roll,
            3 => GyroOrientation::YawPlusRoll,
            4 => GyroOrientation::PlayerSpace,
            _ => GyroOrientation::WorldSpace,
        };
        on_dirty_for_orientation();
    });

    let gyro_for_sensitivity = gyro.clone();
    let on_dirty_for_sensitivity = on_dirty.clone();
    widgets
        .sensitivity
        .connect_value_changed(move |spin| {
            gyro_for_sensitivity.borrow_mut().sensitivity = spin.value() as f32;
            on_dirty_for_sensitivity();
        });

    let gyro_for_invert_x = gyro.clone();
    let on_dirty_for_invert_x = on_dirty.clone();
    widgets.invert_x.connect_active_notify(move |switch| {
        gyro_for_invert_x.borrow_mut().invert_x = switch.is_active();
        on_dirty_for_invert_x();
    });

    let gyro_for_invert_y = gyro.clone();
    let on_dirty_for_invert_y = on_dirty.clone();
    widgets.invert_y.connect_active_notify(move |switch| {
        gyro_for_invert_y.borrow_mut().invert_y = switch.is_active();
        on_dirty_for_invert_y();
    });

    let gyro_for_smoothing = gyro.clone();
    let on_dirty_for_smoothing = on_dirty.clone();
    widgets.smoothing.connect_active_notify(move |switch| {
        gyro_for_smoothing.borrow_mut().smoothing = switch.is_active();
        on_dirty_for_smoothing();
    });
}

fn update_dependency_rows(widgets: &GyroWidgets, enabled: bool) {
    widgets.activation_row.set_sensitive(enabled);
    widgets.output_row.set_sensitive(enabled);
    widgets.orientation_row.set_sensitive(enabled);
    widgets.sensitivity_row.set_sensitive(enabled);
    widgets.invert_x_row.set_sensitive(enabled);
    widgets.invert_y_row.set_sensitive(enabled);
    widgets.smoothing_row.set_sensitive(enabled);
    widgets
        .button_row
        .set_visible(enabled && widgets.activation.selected() != 0);
}

fn switch_row(title: &str, active: bool) -> (adw::ActionRow, gtk4::Switch) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    let switch = gtk4::Switch::new();
    switch.set_active(active);
    switch.set_valign(gtk4::Align::Center);
    row.add_suffix(&switch);
    (row, switch)
}

fn dropdown_row(title: &str, labels: &[String], selected: u32) -> (adw::ActionRow, gtk4::DropDown) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let dropdown = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&refs)),
        None::<&gtk4::Expression>,
    );
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk4::Align::Center);
    row.add_suffix(&dropdown);
    (row, dropdown)
}

fn sensitivity_row(value: f32) -> (adw::ActionRow, gtk4::SpinButton) {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Sensitivity"));
    row.set_subtitle(&crate::tr!("Multiplier applied to gyro motion"));
    let spin = gtk4::SpinButton::with_range(0.05, 20.0, 0.05);
    spin.set_digits(2);
    spin.set_value(value as f64);
    spin.set_valign(gtk4::Align::Center);
    row.add_suffix(&spin);
    (row, spin)
}

fn orientation_labels() -> Vec<String> {
    [
        crate::tr!("Passthrough"),
        crate::tr!("Yaw"),
        crate::tr!("Roll"),
        crate::tr!("Yaw + Roll"),
        crate::tr!("Player Space"),
        crate::tr!("World Space"),
    ]
    .into_iter()
    .collect()
}

fn orientation_index(orientation: GyroOrientation) -> u32 {
    match orientation {
        GyroOrientation::Local => 0,
        GyroOrientation::Yaw => 1,
        GyroOrientation::Roll => 2,
        GyroOrientation::YawPlusRoll => 3,
        GyroOrientation::PlayerSpace => 4,
        GyroOrientation::WorldSpace => 5,
    }
}

fn activation_labels() -> Vec<String> {
    [
        crate::tr!("Always on"),
        crate::tr!("While button held"),
        crate::tr!("Button toggle"),
    ]
    .into_iter()
    .collect()
}

fn output_labels() -> Vec<String> {
    [
        crate::tr!("Mouse"),
        crate::tr!("Left stick"),
        crate::tr!("Right stick"),
    ]
    .into_iter()
    .collect()
}

fn activation_index(activation: &GyroActivation) -> u32 {
    match activation {
        GyroActivation::Always => 0,
        GyroActivation::Hold(_) => 1,
        GyroActivation::Toggle(_) => 2,
    }
}

fn output_index(output: GyroOutput) -> u32 {
    match output {
        GyroOutput::Mouse => 0,
        GyroOutput::LeftStick => 1,
        GyroOutput::RightStick => 2,
    }
}

fn activation_button_options(
    device: Option<&DeviceInfo>,
    activation: &GyroActivation,
) -> Vec<(InputSource, String)> {
    let mut options: Vec<(InputSource, String)> = source_options_for_device(
        device,
        VirtualGamepadBackend::DirectInput,
    )
    .into_iter()
    .filter(|(source, _)| matches!(source, InputSource::Button(_)))
    .collect();
    if let Some(button) = activation.button() {
        let source = InputSource::Button(button);
        if !options.iter().any(|(candidate, _)| *candidate == source) {
            let label = source_options_for_device(None, VirtualGamepadBackend::DirectInput)
                .into_iter()
                .find(|(candidate, _)| *candidate == source)
                .map(|(_, label)| label)
                .unwrap_or_else(|| format!("{button:?}"));
            options.push((source, format!("{label} (unavailable)")));
        }
    }
    options
}

fn button_option_labels(options: &[(InputSource, String)]) -> Vec<String> {
    options.iter().map(|(_, label)| label.clone()).collect()
}

fn activation_button_index(options: &[(InputSource, String)], activation: &GyroActivation) -> u32 {
    let wanted = InputSource::Button(activation.button().unwrap_or(GamepadButton::LeftTrigger));
    options
        .iter()
        .position(|(source, _)| *source == wanted)
        .unwrap_or(0) as u32
}
