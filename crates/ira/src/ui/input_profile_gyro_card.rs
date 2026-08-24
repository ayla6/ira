//! Whole-controller gyro configuration card for the editor's Gyro page.
//!
//! One full-width libadwaita row per setting: enable switch, activation rule,
//! output, sensitivity multiplier, invert/smoothing toggles — mirroring the
//! Steam Input gyro panel — plus the motion transport rows: the native
//! sensor switch and the DSU output-mode note (the cemuhook stream is
//! inherent to picking DSU, never a toggle).

use super::input_profile_activator_sheet::{combo_row, spin_row};
use super::input_profile_options::source_options_for_device;
use adw::prelude::*;
use ira_input::{
    DeviceInfo, GamepadButton, GyroActivation, GyroConfig, GyroOrientation, GyroOutput,
    InputProfile, InputSource, VirtualGamepadBackend,
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
    sensitivity: adw::SpinRow,
    invert_x: adw::SwitchRow,
    invert_y: adw::SwitchRow,
    smoothing: adw::SwitchRow,
}

/// The profile-level motion transport rows. The editor keeps this handle so
/// [`refresh_motion_rows`] can re-evaluate visibility whenever the profile's
/// backend changes.
#[derive(Clone)]
pub(super) struct MotionRows {
    pub(super) native: adw::SwitchRow,
}

impl MotionRows {
    fn apply_backend(&self, backend: VirtualGamepadBackend) {
        // With the DSU backend the cemuhook stream IS the transport for the
        // whole controller, so native motion does not apply.
        self.native.set_visible(backend != VirtualGamepadBackend::Dsu);
    }
}

pub(super) fn refresh_motion_rows(rows: &MotionRows, backend: VirtualGamepadBackend) {
    rows.apply_backend(backend);
}

pub(super) fn add_gyro_group(
    page: &gtk4::Box,
    gyro: &Rc<RefCell<GyroConfig>>,
    profile: &Rc<RefCell<InputProfile>>,
    device: Option<&DeviceInfo>,
    on_dirty: &Rc<dyn Fn()>,
) -> MotionRows {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Gyro"));
    group.set_description(Some(&crate::tr!(
        "Rotating the controller steers the output. Yaw and pitch are measured relative to gravity, so aiming stays consistent no matter how the controller is held."
    )));

    let button_options = activation_button_options(device, &gyro.borrow().activation);
    let widgets = GyroWidgets {
        enable: switch_row(
            &crate::tr!("Enable gyro"),
            None,
            gyro.borrow().enabled,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                Rc::new(move |active| {
                    gyro.borrow_mut().enabled = active;
                    on_dirty();
                })
            },
        ),
        activation: combo_row(
            &activation_labels(),
            activation_index(&gyro.borrow().activation),
        ),
        button: combo_row(
            &button_option_labels(&button_options),
            activation_button_index(&button_options, &gyro.borrow().activation),
        ),
        output: combo_row(&output_labels(), output_index(gyro.borrow().output)),
        orientation: combo_row(
            &orientation_labels(),
            orientation_index(gyro.borrow().orientation),
        ),
        sensitivity: spin_row(
            &crate::tr!("Sensitivity"),
            0.05,
            20.0,
            0.05,
            gyro.borrow().sensitivity as f64,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                Rc::new(move |value| {
                    gyro.borrow_mut().sensitivity = value as f32;
                    on_dirty();
                })
            },
        ),
        invert_x: switch_row(
            &crate::tr!("Invert horizontal"),
            None,
            gyro.borrow().invert_x,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                Rc::new(move |active| {
                    gyro.borrow_mut().invert_x = active;
                    on_dirty();
                })
            },
        ),
        invert_y: switch_row(
            &crate::tr!("Invert vertical"),
            None,
            gyro.borrow().invert_y,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                Rc::new(move |active| {
                    gyro.borrow_mut().invert_y = active;
                    on_dirty();
                })
            },
        ),
        smoothing: switch_row(
            &crate::tr!("Smoothing"),
            Some(&crate::tr!("Damps jitter while aiming slowly; flicks stay untouched")),
            gyro.borrow().smoothing,
            {
                let gyro = gyro.clone();
                let on_dirty = on_dirty.clone();
                Rc::new(move |active| {
                    gyro.borrow_mut().smoothing = active;
                    on_dirty();
                })
            },
        ),
    };
    widgets.sensitivity.set_subtitle(&crate::tr!(
        "Multiplier applied to gyro motion"
    ));

    let native_motion = switch_row(
        &crate::tr!("Native motion sensors"),
        Some(&crate::tr!(
            "Expose the sensors as raw evdev axes next to the virtual pad; emulators cannot read them yet until SDL and the kernel support uinput motion"
        )),
        profile.borrow().native_motion,
        {
            let profile = profile.clone();
            let on_dirty = on_dirty.clone();
            Rc::new(move |active| {
                profile.borrow_mut().native_motion = active;
                on_dirty();
            })
        },
    );
    native_motion.add_suffix(&experimental_badge());

    update_dependency_rows(&widgets, gyro.borrow().enabled);

    let rows: [&adw::PreferencesRow; 10] = [
        widgets.enable.upcast_ref(),
        widgets.activation.upcast_ref(),
        widgets.button.upcast_ref(),
        widgets.output.upcast_ref(),
        widgets.orientation.upcast_ref(),
        widgets.sensitivity.upcast_ref(),
        widgets.invert_x.upcast_ref(),
        widgets.invert_y.upcast_ref(),
        widgets.smoothing.upcast_ref(),
        native_motion.upcast_ref(),
    ];
    for row in rows {
        group.add(row);
    }
    page.append(&group);
    connect_gyro_changes(&widgets, &button_options, gyro, on_dirty);

    let motion_rows = MotionRows {
        native: native_motion,
    };
    motion_rows.apply_backend(profile.borrow().backend);
    motion_rows
}

/// Small orange pill marking a setting as not production-grade yet.
fn experimental_badge() -> gtk4::Label {
    let badge = gtk4::Label::new(Some(&crate::tr!("Experimental")));
    badge.add_css_class(super::css::CSS_EXPERIMENTAL_BADGE);
    badge.set_valign(gtk4::Align::Center);
    badge
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

fn switch_row(
    title: &str,
    subtitle: Option<&str>,
    active: bool,
    on_change: Rc<dyn Fn(bool)>,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(active).build();
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.connect_active_notify(move |row| on_change(row.is_active()));
    row
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
