//! Per-controller calibration in the controller settings — Steam's
//! "Calibration & Advanced Settings", one section per connected controller:
//! per-stick deadzones and gyro drift calibration. Values live in the
//! shared store keyed by vendor:product, not in any profile — one
//! calibration serves every profile played with that pad.

use super::input_calibration_dialog::show_input_calibration_dialog;
use super::input_profile_widgets::{slider_row_with_scale, SliderSpec};
use adw::prelude::*;
use ira_input::{ControllerCalibration, DeviceInfo};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Which stick a deadzone row edits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StickTab {
    Left,
    Right,
}

/// Append this controller's calibration rows to its expander card.
pub(super) fn add_controller_calibration(
    expander: &adw::ExpanderRow,
    store_path: &Path,
    device: &DeviceInfo,
    registry: Arc<ira_input::ControllerRegistry>,
) {
    // Owned up front so every closure captures data, not borrows of the
    // caller's references.
    let store_path = store_path.to_path_buf();
    let device = device.clone();
    let updating = Rc::new(Cell::new(false));
    let (left_scale, right_scale) = add_deadzone_rows(expander, &store_path, &device, &updating);
    add_low_deadzone_warning(expander, &left_scale, &right_scale);
    add_layout_row(expander, &store_path, &device);
    add_gyro_rows(
        expander,
        &store_path,
        &device,
        registry,
        &updating,
        &left_scale,
        &right_scale,
    );
}

/// Deadzone sliders, debounced so dragging them does not hit the disk on
/// every tick. Returns the scales for cross-updates from calibration.
fn add_deadzone_rows(
    expander: &adw::ExpanderRow,
    store_path: &Path,
    device: &DeviceInfo,
    updating: &Rc<Cell<bool>>,
) -> (gtk4::Scale, gtk4::Scale) {
    let description = crate::tr!(
        "Tune the Joystick Deadzone to fix \"drift\" on joysticks that don't properly re-center. To override this Deadzone Preference per game, change the \"Deadzone Source\" dropdown in the game's Joystick Settings."
    );
    let (left_row, left_scale) = deadzone_row(
        &crate::tr!("Left Joystick Deadzone"),
        &description,
        deadzone_of(store_path, device, StickTab::Left),
        commit_deadzone(store_path, device, StickTab::Left, updating),
        updating,
    );
    let (right_row, right_scale) = deadzone_row(
        &crate::tr!("Right Joystick Deadzone"),
        &description,
        deadzone_of(store_path, device, StickTab::Right),
        commit_deadzone(store_path, device, StickTab::Right, updating),
        updating,
    );
    expander.add_row(&left_row);
    expander.add_row(&right_row);
    (left_scale, right_scale)
}

/// Steam's low-deadzone warning: drift is likely below a few percent.
fn add_low_deadzone_warning(
    expander: &adw::ExpanderRow,
    left_scale: &gtk4::Scale,
    right_scale: &gtk4::Scale,
) {
    let warning = adw::ActionRow::new();
    warning.set_title(&crate::tr!("Warning: Low Deadzone"));
    warning.set_subtitle(&crate::tr!(
        "When joystick deadzones are low, you are more likely to encounter drift or accidental inputs."
    ));
    warning.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
    let refresh: Rc<dyn Fn()> = {
        let left_scale = left_scale.clone();
        let right_scale = right_scale.clone();
        let warning = warning.clone();
        Rc::new(move || warning.set_visible(left_scale.value() < 5.0 || right_scale.value() < 5.0))
    };
    refresh();
    let refresh_for_left = refresh.clone();
    left_scale.connect_value_changed(move |_| refresh_for_left());
    right_scale.connect_value_changed(move |_| refresh());
    expander.add_row(&warning);
}

/// The controller-level Nintendo button layout, Steam's per-controller
/// toggle: face buttons swap (A↔B, X↔Y) so actions follow the labels
/// printed on Nintendo-style pads. On by default for Nintendo-family
/// controllers, off for everything else, and any explicit change is
/// stored per controller. Takes effect the next time the input daemon
/// starts for this controller.
fn add_layout_row(expander: &adw::ExpanderRow, store_path: &Path, device: &DeviceInfo) {
    let row = adw::SwitchRow::new();
    row.set_title(&crate::tr!("Nintendo Button Layout"));
    row.set_subtitle(&crate::tr!(
        "Swap A/B and X/Y so buttons match Nintendo labeling. Enabled by default on Nintendo controllers"
    ));
    row.set_active(ira_input::resolved_nintendo_layout(store_path, device));
    let store_path = store_path.to_path_buf();
    let device = device.clone();
    row.connect_active_notify(move |row| {
        let mut calibration =
            ira_input::load_calibration(&store_path, device.vendor, device.product)
                .unwrap_or_default();
        calibration.nintendo_layout = row.is_active();
        if let Err(error) =
            ira_input::save_calibration(&store_path, device.vendor, device.product, &calibration)
        {
            eprintln!("Could not save controller calibration: {error}");
        }
    });
    expander.add_row(&row);
}

/// Gyro calibration rows: stored bias summary plus calibrate/reset.
fn add_gyro_rows(
    expander: &adw::ExpanderRow,
    store_path: &Path,
    device: &DeviceInfo,
    registry: Arc<ira_input::ControllerRegistry>,
    updating: &Rc<Cell<bool>>,
    left_scale: &gtk4::Scale,
    right_scale: &gtk4::Scale,
) {
    let summary = adw::ActionRow::new();
    summary.set_title(&crate::tr!("Gyro calibration"));
    summary.set_subtitle(&crate::tr!(
        "Pitch, yaw and roll speeds should each be very close to zero when the controller is stationary"
    ));
    let bias_label = gtk4::Label::new(None);
    bias_label.set_xalign(1.0);
    bias_label.add_css_class("dim-label");
    bias_label.set_text(
        &ira_input::load_calibration(store_path, device.vendor, device.product)
            .map(format_calibration)
            .unwrap_or_else(|| crate::tr!("Not calibrated").to_string()),
    );
    summary.add_suffix(&bias_label);
    expander.add_row(&summary);

    let actions = adw::ActionRow::new();
    actions.set_title(&crate::tr!("Calibrate while controller is flat and still"));
    let calibrate = gtk4::Button::with_label(&crate::tr!("Calibrate"));
    calibrate.set_valign(gtk4::Align::Center);
    let reset = gtk4::Button::with_label(&crate::tr!("Reset"));
    reset.set_valign(gtk4::Align::Center);
    actions.add_suffix(&reset);
    actions.add_suffix(&calibrate);
    expander.add_row(&actions);

    {
        let store_path = store_path.to_path_buf();
        let device = device.clone();
        let left_scale = left_scale.clone();
        let right_scale = right_scale.clone();
        let bias_label = bias_label.clone();
        let updating = updating.clone();
        reset.connect_clicked(move |_| {
            // Clear the measured values but keep controller-level choices:
            // deleting the entry outright would also drop the Nintendo
            // layout toggle stored beside them.
            let stored =
                ira_input::load_calibration(&store_path, device.vendor, device.product);
            let mut cleared =
                stored.unwrap_or_else(|| ira_input::default_calibration_for(&device));
            cleared.x = 0.0;
            cleared.y = 0.0;
            cleared.z = 0.0;
            cleared.stick_deadzone_left = 0.0;
            cleared.stick_deadzone_right = 0.0;
            if let Err(error) =
                ira_input::save_calibration(&store_path, device.vendor, device.product, &cleared)
            {
                eprintln!("Could not clear controller calibration: {error}");
            }
            // Move the sliders without their callbacks writing an empty
            // entry right back.
            updating.set(true);
            left_scale.set_value(0.0);
            right_scale.set_value(0.0);
            updating.set(false);
            bias_label.set_text(&crate::tr!("Not calibrated"));
        });
    }

    let store_path = store_path.to_path_buf();
    let device = device.clone();
    let left_scale = left_scale.clone();
    let right_scale = right_scale.clone();
    let bias_label = bias_label.clone();
    let updating = updating.clone();
    calibrate.connect_clicked(move |button| {
        let Some(window) = button
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let store_path = store_path.clone();
        let device = device.clone();
        let left_scale = left_scale.clone();
        let right_scale = right_scale.clone();
        let bias_label = bias_label.clone();
        let updating = updating.clone();
        show_input_calibration_dialog(&window, registry.clone(), device.clone(), move |result| {
            match result {
                Ok(value) => {
                    save_gyro_bias(&store_path, &device, value, &bias_label);
                    // Reflect what actually ended up in the store, without
                    // the sliders writing anything back mid-update.
                    updating.set(true);
                    let stored =
                        ira_input::load_calibration(&store_path, device.vendor, device.product);
                    left_scale.set_value(
                        f64::from(
                            stored
                                .as_ref()
                                .map(|c| c.stick_deadzone_left)
                                .unwrap_or(0.0),
                        ) * 100.0,
                    );
                    right_scale.set_value(
                        f64::from(
                            stored
                                .as_ref()
                                .map(|c| c.stick_deadzone_right)
                                .unwrap_or(0.0),
                        ) * 100.0,
                    );
                    updating.set(false);
                }
                Err(error) => eprintln!("Gyro calibration failed: {error}"),
            }
        });
    });
}

/// Persists a fresh gyro bias on top of the existing calibration entry so
/// measured stick deadzones survive the round-trip.
fn save_gyro_bias(
    store_path: &Path,
    device: &DeviceInfo,
    bias: ControllerCalibration,
    bias_label: &gtk4::Label,
) {
    let mut calibration =
        ira_input::load_calibration(store_path, device.vendor, device.product).unwrap_or_else(
            || {
                let mut seeded = bias;
                seeded.nintendo_layout = device.prefers_nintendo_layout();
                seeded
            },
        );
    calibration.x = bias.x;
    calibration.y = bias.y;
    calibration.z = bias.z;
    if let Err(error) =
        ira_input::save_calibration(store_path, device.vendor, device.product, &calibration)
    {
        eprintln!("Could not save controller calibration: {error}");
    }
    bias_label.set_text(&format_calibration(calibration));
}

fn deadzone_row(
    title: &str,
    subtitle: &str,
    initial_percent: f32,
    on_commit: Rc<dyn Fn(f64)>,
    updating: &Rc<Cell<bool>>,
) -> (gtk4::ListBoxRow, gtk4::Scale) {
    let (row, scale) = slider_row_with_scale(
        title,
        Some(subtitle),
        &SliderSpec(0.0, 50.0, 1.0, f64::from(initial_percent) * 100.0),
        |value| format!("{value:.0} %"),
        |_| {},
    );
    // Coalesce the load-modify-store cycle until the slider settles.
    // Programmatic updates (reset, post-calibration sync) run while the
    // updating guard is held and must not schedule a write at all.
    let pending: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));
    let source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let scheduling = updating.clone();
    scale.connect_value_changed(move |scale| {
        if scheduling.get() {
            return;
        }
        *pending.borrow_mut() = Some(scale.value());
        if let Some(id) = source.borrow_mut().take() {
            id.remove();
        }
        let pending = pending.clone();
        let source_for_timeout = source.clone();
        let on_commit = on_commit.clone();
        let id = glib::timeout_add_local(Duration::from_millis(250), move || {
            if let Some(value) = pending.borrow_mut().take() {
                on_commit(value);
            }
            *source_for_timeout.borrow_mut() = None;
            glib::ControlFlow::Break
        });
        *source.borrow_mut() = Some(id);
    });
    (row, scale)
}

fn commit_deadzone(
    store_path: &Path,
    device: &DeviceInfo,
    tab: StickTab,
    updating: &Rc<Cell<bool>>,
) -> Rc<dyn Fn(f64)> {
    let store_path = store_path.to_path_buf();
    let device = device.clone();
    let updating = updating.clone();
    Rc::new(move |percent| {
        if updating.get() {
            return;
        }
        let mut calibration =
            ira_input::load_calibration(&store_path, device.vendor, device.product)
                .unwrap_or_else(|| ira_input::default_calibration_for(&device));
        match tab {
            StickTab::Left => calibration.stick_deadzone_left = (percent / 100.0) as f32,
            StickTab::Right => calibration.stick_deadzone_right = (percent / 100.0) as f32,
        }
        if let Err(error) =
            ira_input::save_calibration(&store_path, device.vendor, device.product, &calibration)
        {
            eprintln!("Could not save controller calibration: {error}");
        }
    })
}

fn deadzone_of(store_path: &Path, device: &DeviceInfo, tab: StickTab) -> f32 {
    let calibration = ira_input::load_calibration(store_path, device.vendor, device.product);
    match tab {
        StickTab::Left => calibration.map(|c| c.stick_deadzone_left),
        StickTab::Right => calibration.map(|c| c.stick_deadzone_right),
    }
    .unwrap_or(0.0)
}

fn format_calibration(calibration: ControllerCalibration) -> String {
    format!(
        "X {:.3}  Y {:.3}  Z {:.3}",
        calibration.x, calibration.y, calibration.z
    )
}
