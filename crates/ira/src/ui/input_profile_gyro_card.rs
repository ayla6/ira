//! Whole-controller gyro configuration card for the editor's Gyro page.
//!
//! One full-width libadwaita row per setting: enable switch, activation rule,
//! output, orientation, sensitivity multiplier, invert/smoothing toggles —
//! mirroring the Steam Input gyro panel.

use super::input_profile_options::source_options_for_device;
use super::input_profile_sheet_base::combo_row;
use super::input_profile_widgets::{
    format_number, option_picker_popover, picker_button, slider_row, switch_row, OptionChoice,
    SettingGroup, SliderSpec,
};
use adw::prelude::*;
use ira_input::{
    DeviceInfo, GamepadButton, GyroActivation, GyroConfig, GyroOrientation, GyroOutput,
    InputSource, VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct GyroWidgets {
    enable: adw::SwitchRow,
    activation: adw::ComboRow,
    button: adw::ComboRow,
    output: adw::ComboRow,
    orientation: gtk4::ListBoxRow,
    sensitivity: gtk4::ListBoxRow,
    invert_x: adw::SwitchRow,
    invert_y: adw::SwitchRow,
    smoothing: adw::SwitchRow,
}

pub(super) fn add_gyro_group(
    page: &gtk4::Box,
    gyro: &Rc<RefCell<GyroConfig>>,
    device: Option<&DeviceInfo>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let group = SettingGroup::new(
        Some(&crate::tr!("Gyro")),
        Some(&crate::tr!(
            "Rotating the controller steers the output. Yaw and pitch are measured relative to gravity, so aiming stays consistent no matter how the controller is held."
        )),
    );

    let button_options = activation_button_options(device, &gyro.borrow().activation);
    let activation = combo_row(
        &activation_labels(),
        activation_index(&gyro.borrow().activation),
    );
    activation.set_title(&crate::tr!("Activation"));
    let button = combo_row(
        &button_option_labels(&button_options),
        activation_button_index(&button_options, &gyro.borrow().activation),
    );
    button.set_title(&crate::tr!("Button"));
    let output = combo_row(&output_labels(), output_index(gyro.borrow().output));
    output.set_title(&crate::tr!("Output"));
    let widgets = GyroWidgets {
        enable: switch_row(&crate::tr!("Enable gyro"), None, gyro.borrow().enabled, {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            move |active| {
                gyro.borrow_mut().enabled = active;
                on_dirty();
            }
        }),
        activation,
        button,
        output,
        orientation: orientation_row(gyro, on_dirty, gyro.borrow().orientation),
        sensitivity: {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            let initial = f64::from(gyro.borrow().sensitivity);
            slider_row(
                &crate::tr!("Sensitivity"),
                Some(&crate::tr!("Multiplier applied to gyro motion")),
                &SliderSpec(0.05, 20.0, 0.05, initial),
                format_number,
                move |value| {
                    gyro.borrow_mut().sensitivity = value as f32;
                    on_dirty();
                },
            )
        },
        invert_x: switch_row(
            &crate::tr!("Invert horizontal"),
            None,
            gyro.borrow().invert_x,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                move |active| {
                    gyro.borrow_mut().invert_x = active;
                    on_dirty();
                }
            },
        ),
        invert_y: switch_row(
            &crate::tr!("Invert vertical"),
            None,
            gyro.borrow().invert_y,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                move |active| {
                    gyro.borrow_mut().invert_y = active;
                    on_dirty();
                }
            },
        ),
        smoothing: switch_row(
            &crate::tr!("Smoothing"),
            Some(&crate::tr!(
                "Damps jitter while aiming slowly; flicks stay untouched"
            )),
            gyro.borrow().smoothing,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                move |active| {
                    gyro.borrow_mut().smoothing = active;
                    on_dirty();
                }
            },
        ),
    };
    update_dependency_rows(&widgets, gyro.borrow().enabled);

    group.add(&widgets.enable);
    group.add(&widgets.activation);
    group.add(&widgets.button);
    group.add(&widgets.output);
    group.add(&widgets.orientation);
    group.add(&widgets.sensitivity);
    group.add(&widgets.invert_x);
    group.add(&widgets.invert_y);
    group.add(&widgets.smoothing);
    page.append(&group.root);
    connect_gyro_changes(&widgets, &button_options, gyro, on_dirty);
}

fn connect_gyro_changes(
    widgets: &GyroWidgets,
    button_options: &[(InputSource, String)],
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    // The enable switch writes the config through its own construction
    // closure; this second connection just refreshes dependent rows.
    let widgets_for_enable = widgets.clone();
    widgets.enable.connect_active_notify(move |row| {
        update_dependency_rows(&widgets_for_enable, row.is_active());
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

    connect_activation_changes(widgets, button_options, gyro, on_dirty);
}

fn connect_activation_changes(
    widgets: &GyroWidgets,
    button_options: &[(InputSource, String)],
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let gyro_for_activation = gyro.clone();
    let on_dirty_for_activation = on_dirty.clone();
    let button_options_for_activation = button_options.to_vec();
    let widgets_for_activation = widgets.clone();
    widgets.activation.connect_selected_notify(move |dropdown| {
        apply_activation_selection(
            &widgets_for_activation,
            dropdown.selected(),
            &button_options_for_activation,
            &gyro_for_activation,
            &on_dirty_for_activation,
        );
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
}

fn apply_activation_selection(
    widgets: &GyroWidgets,
    selected: u32,
    button_options: &[(InputSource, String)],
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    let fallback_button = button_options
        .get(widgets.button.selected() as usize)
        .and_then(|(source, _)| match source {
            InputSource::Button(button) => Some(*button),
            _ => None,
        })
        .unwrap_or(GamepadButton::LeftTrigger);
    gyro.borrow_mut().activation = match selected {
        1 => GyroActivation::Hold(fallback_button),
        2 => GyroActivation::Toggle(fallback_button),
        _ => GyroActivation::Always,
    };
    widgets.button.set_visible(selected != 0);
    on_dirty();
}

fn update_dependency_rows(widgets: &GyroWidgets, enabled: bool) {
    widgets.activation.set_sensitive(enabled);
    widgets.button.set_sensitive(enabled);
    widgets.output.set_sensitive(enabled);
    widgets.orientation.set_sensitive(enabled);
    widgets.sensitivity.set_sensitive(enabled);
    widgets.invert_x.set_sensitive(enabled);
    widgets.invert_y.set_sensitive(enabled);
    widgets.smoothing.set_sensitive(enabled);
    widgets
        .button
        .set_visible(enabled && widgets.activation.selected() != 0);
}

/// Steam's described orientation popup: every preset explains what it does
/// to horizontal and vertical output.
fn orientation_choices() -> Vec<(GyroOrientation, OptionChoice)> {
    vec![
        (
            GyroOrientation::Local,
            OptionChoice {
                title: crate::tr!("Passthrough"),
                description: Some(crate::tr!(
                    "Raw controller axes with no gravity math: reported yaw drives horizontal output and reported pitch drives vertical output exactly as the sensor delivers them."
                )),
            },
        ),
        (
            GyroOrientation::Yaw,
            OptionChoice {
                title: crate::tr!("Yaw"),
                description: Some(crate::tr!(
                    "Turn the controller around its own vertical axis for horizontal output. Tilt the controller up and down around its own lateral axis for vertical output. (Local Space Preset)"
                )),
            },
        ),
        (
            GyroOrientation::Roll,
            OptionChoice {
                title: crate::tr!("Roll"),
                description: Some(crate::tr!(
                    "Lean the controller around its forward axis for horizontal output. Tilt the controller up and down around its own lateral axis for vertical output. (Local Space Preset)"
                )),
            },
        ),
        (
            GyroOrientation::YawPlusRoll,
            OptionChoice {
                title: crate::tr!("Yaw + Roll"),
                description: Some(crate::tr!(
                    "Yaw + Roll adds the Lean and Turn together for horizontal output. Tilting the controller up and down around its own lateral axis still moves the output up and down. (Local Space Preset)"
                )),
            },
        ),
        (
            GyroOrientation::PlayerSpace,
            OptionChoice {
                title: crate::tr!("Player Space"),
                description: Some(crate::tr!(
                    "Player Space uses Yaw + Roll around the gravity axis for horizontal output, and Local Pitch for vertical output."
                )),
            },
        ),
        (
            GyroOrientation::WorldSpace,
            OptionChoice {
                title: crate::tr!("World Space"),
                description: Some(crate::tr!(
                    "World Space uses all rotation around the gravity axis for horizontal output, and World Pitch for vertical output, but does not move vertically when the controller is tilted on its side."
                )),
            },
        ),
        (
            GyroOrientation::LaserPointer,
            OptionChoice {
                title: crate::tr!("Laser Pointer"),
                description: Some(crate::tr!(
                    "Acts similar to a laser pointer. Great for cursor control on a stand-alone controller."
                )),
            },
        ),
    ]
}

fn orientation_row(
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
    orientation: GyroOrientation,
) -> gtk4::ListBoxRow {
    let choices = orientation_choices();
    let current = choices
        .iter()
        .position(|(candidate, _)| *candidate == orientation)
        .unwrap_or(0);
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Orientation"));
    row.set_subtitle(&crate::tr!(
        "How the controller's rotation maps to horizontal and vertical output"
    ));
    let button = picker_button(&choices[current].1.title, &gtk4::Popover::new());
    let options: Vec<OptionChoice> = choices.iter().map(|entry| entry.1.clone()).collect();
    let picker = option_picker_popover(&options, current, {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        let button = button.clone();
        move |index| {
            if let Some((orientation, choice)) = choices.get(index) {
                gyro.borrow_mut().orientation = *orientation;
                button.set_label(&choice.title);
                on_dirty();
            }
        }
    });
    button.set_popover(Some(&picker));
    row.add_suffix(&button);
    row.upcast()
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
    let mut options: Vec<(InputSource, String)> =
        source_options_for_device(device, VirtualGamepadBackend::DirectInput)
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
