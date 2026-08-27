//! Whole-controller gyro configuration card for the editor's Gyro page.
//!
//! One full-width libadwaita row per setting: enable switch, activation rule,
//! output, orientation, sensitivity multiplier, invert/smoothing toggles —
//! mirroring the Steam Input gyro panel.

use super::input_profile_options::source_options_for_device;
use super::input_profile_sheet_base::combo_row;
use super::input_profile_widgets::{
    format_percent, slider_entry_row, slider_row_with_scale, switch_row, OptionChoice,
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
    orientation: adw::ComboRow,
    sensitivity: gtk4::ListBoxRow,
    invert_x: adw::SwitchRow,
    invert_y: adw::SwitchRow,
    smoothing: adw::SwitchRow,
    /// Steam's "Dots Per 360°" — pixels per full physical turn at 1x,
    /// shared with the flick stick.
    dots_per_360: gtk4::ListBoxRow,
    /// Clockwise rotation of the 2D gyro output (Steam's "Rotate Output").
    rotate_output: gtk4::ListBoxRow,
    /// Gyro-to-stick shaping rows, sensitive only when the output is a
    /// stick (Steam's "Gyro To Joystick" page).
    stick_max_output: gtk4::ListBoxRow,
    stick_response_style: adw::ComboRow,
    stick_power_curve: gtk4::ListBoxRow,
    stick_lock_edges: adw::SwitchRow,
    stick_deadzone: gtk4::ListBoxRow,
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
    activation.set_title(&crate::tr!("Choose Gyro Button(s)"));
    activation.set_subtitle(&activation_description(activation_index(&gyro.borrow().activation)));
    let button = combo_row(
        &button_option_labels(&button_options),
        activation_button_index(&button_options, &gyro.borrow().activation),
    );
    button.set_title(&crate::tr!("Button"));
    let output = combo_row(&output_labels(), output_index(gyro.borrow().output));
    output.set_title(&crate::tr!("Output"));
    output.set_subtitle(
        crate::tr!(
            "Native motion sensors pass the controller's real sensor on to the game \
             (Switch Pro, DualShock 4 and DualSense with a motion sensor)"
        )
        .as_str(),
    );
    let stick = gyro.borrow().stick;
    let initial_dots = f64::from(gyro.borrow().dots_per_360);
    let initial_rotate = f64::from(gyro.borrow().rotate_output);
    let dots_per_360 = {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        slider_entry_row(
            &crate::tr!("Gyro Angles to Mouse Pixels (Dots Per 360°)"),
            Some(&crate::tr!(
                "One full 360° turn of the gyro moves the mouse this many pixels at 1x sensitivity. Shared with the Flick Stick so both calibrate against the same in-game angle."
            )),
            &SliderSpec(500.0, 30_000.0, 5.0, initial_dots),
            move |value| {
                gyro.borrow_mut().dots_per_360 = value as f32;
                on_dirty();
            },
        )
    };
    let rotate_output = {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        slider_row_with_scale(
            &crate::tr!("Rotate Output"),
            Some(&crate::tr!(
                "Adjust the 2D output of the gyroscope clockwise/counter clockwise"
            )),
            &SliderSpec(0.0, 360.0, 1.0, initial_rotate),
            |value| format!("{value:.0}°"),
            move |value| {
                gyro.borrow_mut().rotate_output = value as f32;
                on_dirty();
            },
        )
        .0
    };
    let stick_max_output = {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        slider_row_with_scale(
            &crate::tr!("Maximum Joystick Output"),
            Some(&crate::tr!(
                "Maximum gyro input speed maps to this joystick output. Decrease it to avoid triggering a game's \"Extra Yaw\" setting."
            )),
            &SliderSpec(0.1, 1.0, 0.01, f64::from(stick.max_output)),
            format_percent,
            move |value| {
                gyro.borrow_mut().stick.max_output = value as f32;
                on_dirty();
            },
        )
        .0
    };
    let stick_response_style = {
        let styles = [
            crate::tr!("Circular"),
            crate::tr!("Per Axis"),
        ];
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        let combo = combo_row(&styles, response_style_index(&stick.response_style));
        combo.set_title(&crate::tr!("Response Axis Style"));
        combo.set_subtitle(&crate::tr!(
            "Apply the response curve per axis, or based on the distance from the center"
        ));
        combo.connect_selected_notify(move |combo| {
            gyro.borrow_mut().stick.response_style = match combo.selected() {
                1 => ira_input::GyroStickResponseStyle::PerAxis,
                _ => ira_input::GyroStickResponseStyle::Circular,
            };
            on_dirty();
        });
        combo
    };
    let stick_power_curve = {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        slider_row_with_scale(
            &crate::tr!("Joystick Power Curve"),
            Some(&crate::tr!(
                "How aggressively the joystick output deflects: 0.1 extremely aggressive, 1 linear, 4 extremely relaxed"
            )),
            &SliderSpec(0.1, 4.0, 0.1, f64::from(stick.power_curve)),
            |value| format!("{value:.1}"),
            move |value| {
                gyro.borrow_mut().stick.power_curve = value as f32;
                on_dirty();
            },
        )
        .0
    };
    let stick_lock_edges = switch_row(
        &crate::tr!("Lock at Edges"),
        Some(&crate::tr!(
            "Locks joystick output to the maximum deflection angle; off lets diagonals use the full output range"
        )),
        stick.lock_at_edges,
        {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            move |active| {
                gyro.borrow_mut().stick.lock_at_edges = active;
                on_dirty();
            }
        },
    );
    let stick_deadzone = {
        let gyro = gyro.clone();
        let on_dirty = on_dirty.clone();
        slider_row_with_scale(
            &crate::tr!("Gyro Speed Deadzone"),
            Some(&crate::tr!(
                "The minimum speed the gyro must move before there is a reaction. Combats hand shake; rotation lost to the deadzone is recovered when moving fast."
            )),
            &SliderSpec(0.0, 20.0, 0.1, f64::from(stick.deadzone_dps)),
            |value| format!("{value:.1}°/s"),
            move |value| {
                gyro.borrow_mut().stick.deadzone_dps = value as f32;
                on_dirty();
            },
        )
        .0
    };

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
            slider_entry_row(
                &crate::tr!("Sensitivity"),
                Some(&crate::tr!("Multiplier applied to gyro motion; type for precise values")),
                &SliderSpec(0.05, 20.0, 0.05, initial),
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
        dots_per_360,
        rotate_output,
        stick_max_output,
        stick_response_style,
        stick_power_curve,
        stick_lock_edges,
        stick_deadzone,
    };
    update_dependency_rows(&widgets, &gyro.borrow());

    group.add(&widgets.enable);
    group.add(&widgets.activation);
    group.add(&widgets.button);
    group.add(&widgets.output);
    group.add(&widgets.orientation);
    group.add(&widgets.dots_per_360);
    group.add(&widgets.sensitivity);
    group.add(&widgets.invert_x);
    group.add(&widgets.invert_y);
    group.add(&widgets.smoothing);
    group.add(&widgets.rotate_output);
    group.add(&widgets.stick_max_output);
    group.add(&widgets.stick_response_style);
    group.add(&widgets.stick_power_curve);
    group.add(&widgets.stick_lock_edges);
    group.add(&widgets.stick_deadzone);
    page.append(&group.root);
    connect_gyro_changes(&widgets, &button_options, gyro, on_dirty);
}

fn response_style_index(style: &ira_input::GyroStickResponseStyle) -> u32 {
    match style {
        ira_input::GyroStickResponseStyle::Circular => 0,
        ira_input::GyroStickResponseStyle::PerAxis => 1,
    }
}

fn connect_gyro_changes(
    widgets: &GyroWidgets,
    button_options: &[(InputSource, String)],
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    // The enable switch and output write the config through their own
    // construction closures; these connections just refresh dependent rows.
    let widgets_for_enable = widgets.clone();
    let gyro_for_enable = gyro.clone();
    widgets.enable.connect_active_notify(move |_| {
        update_dependency_rows(&widgets_for_enable, &gyro_for_enable.borrow());
    });

    let gyro_for_output = gyro.clone();
    let on_dirty_for_output = on_dirty.clone();
    let widgets_for_output = widgets.clone();
    widgets.output.connect_selected_notify(move |dropdown| {
        gyro_for_output.borrow_mut().output = match dropdown.selected() {
            1 => GyroOutput::LeftStick,
            2 => GyroOutput::RightStick,
            3 => GyroOutput::NativeMotion,
            _ => GyroOutput::Mouse,
        };
        update_dependency_rows(&widgets_for_output, &gyro_for_output.borrow());
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
            (GyroActivation::Suppress(_), 2) => GyroActivation::Suppress(button),
            (GyroActivation::Toggle(_), 3) => GyroActivation::Toggle(button),
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
        2 => GyroActivation::Suppress(fallback_button),
        3 => GyroActivation::Toggle(fallback_button),
        _ => GyroActivation::Always,
    };
    widgets.button.set_visible(selected != 0);
    widgets
        .activation
        .set_subtitle(&activation_description(selected));
    on_dirty();
}

fn update_dependency_rows(widgets: &GyroWidgets, gyro: &GyroConfig) {
    let enabled = gyro.enabled;
    let native = gyro.output == GyroOutput::NativeMotion;
    widgets.activation.set_sensitive(enabled);
    widgets.button.set_sensitive(enabled);
    widgets.output.set_sensitive(enabled);
    // Native motion skips the orientation math entirely: the game reads the
    // sensor axes as the device reports them. The invert flags still apply
    // there — they flip the wire's yaw and pitch — so they switch to the
    // axis names instead of greying out.
    widgets.orientation.set_sensitive(enabled && !native);
    if native {
        widgets.invert_x.set_title(&crate::tr!("Invert yaw"));
        widgets.invert_y.set_title(&crate::tr!("Invert pitch"));
    } else {
        widgets.invert_x.set_title(&crate::tr!("Invert horizontal"));
        widgets.invert_y.set_title(&crate::tr!("Invert vertical"));
    }
    widgets.sensitivity.set_sensitive(enabled);
    widgets.invert_x.set_sensitive(enabled);
    widgets.invert_y.set_sensitive(enabled);
    widgets.smoothing.set_sensitive(enabled);
    let stick_output =
        enabled && matches!(gyro.output, GyroOutput::LeftStick | GyroOutput::RightStick);
    widgets.stick_max_output.set_sensitive(stick_output);
    widgets.stick_response_style.set_sensitive(stick_output);
    widgets.stick_power_curve.set_sensitive(stick_output);
    widgets.stick_lock_edges.set_sensitive(stick_output);
    widgets.stick_deadzone.set_sensitive(stick_output);
    widgets
        .button
        .set_visible(enabled && widgets.activation.selected() != 0);
}

/// Steam's described orientation popup: every preset explains what it does
/// to horizontal and vertical output.
fn orientation_choices() -> Vec<(GyroOrientation, OptionChoice)> {
    vec![
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
) -> adw::ComboRow {
    let choices = orientation_choices();
    let current = choices
        .iter()
        .position(|(candidate, _)| *candidate == orientation)
        .unwrap_or(0);
    let combo = combo_row(
        &choices.iter().map(|(_, choice)| choice.title.clone()).collect::<Vec<_>>(),
        current as u32,
    );
    combo.set_title(&crate::tr!("Orientation"));
    let description = choices[current].1.description.clone().unwrap_or_default();
    combo.set_subtitle(description.as_str());
    let gyro = gyro.clone();
    let on_dirty = on_dirty.clone();
    let choices_for_signal = choices.clone();
    combo.connect_selected_notify(move |combo| {
        let Some((orientation, choice)) = choices_for_signal.get(combo.selected() as usize) else {
            return;
        };
        gyro.borrow_mut().orientation = *orientation;
        let description = choice.description.clone().unwrap_or_default();
        combo.set_subtitle(description.as_str());
        on_dirty();
    });
    combo
}

fn activation_labels() -> Vec<String> {
    [
        crate::tr!("None (Gyro Always On)"),
        crate::tr!("Hold to Enable Gyro"),
        crate::tr!("Hold to Suppress Gyro"),
        crate::tr!("Toggle Gyro On/Off"),
    ]
    .into_iter()
    .collect()
}

/// Steam's described activation popup.
fn activation_description(selected: u32) -> String {
    match selected {
        0 => crate::tr!("The gyro is always on").to_string(),
        1 => crate::tr!("Gyro is off unless any assigned button is held.").to_string(),
        2 => crate::tr!("Gyro is on unless any assigned button is held.").to_string(),
        3 => crate::tr!("Gyro will toggle on or off any time an assigned button is pressed.").to_string(),
        _ => String::new(),
    }
}

fn output_labels() -> Vec<String> {
    [
        crate::tr!("Mouse"),
        crate::tr!("Left stick"),
        crate::tr!("Right stick"),
        crate::tr!("Native motion sensors"),
    ]
    .into_iter()
    .collect()
}

fn activation_index(activation: &GyroActivation) -> u32 {
    match activation {
        GyroActivation::Always => 0,
        GyroActivation::Hold(_) => 1,
        GyroActivation::Suppress(_) => 2,
        GyroActivation::Toggle(_) => 3,
    }
}

fn output_index(output: GyroOutput) -> u32 {
    match output {
        GyroOutput::Mouse => 0,
        GyroOutput::LeftStick => 1,
        GyroOutput::RightStick => 2,
        GyroOutput::NativeMotion => 3,
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
