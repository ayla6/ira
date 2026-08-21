use super::input_calibration_dialog::show_input_calibration_dialog;
use adw::prelude::*;
use ira_input::DeviceInfo;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub(super) fn add_calibration_group(
    page: &gtk4::Box,
    calibration: &Rc<RefCell<ira_input::GyroCalibration>>,
    device: Option<DeviceInfo>,
    registry: &Arc<ira_input::ControllerRegistry>,
    mark_dirty: &Rc<dyn Fn()>,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Calibration"));
    let summary = adw::ActionRow::new();
    summary.set_title(&crate::tr!("Profile gyro calibration"));
    summary.set_subtitle(&crate::tr!("These bias values belong to this profile"));
    let values = gtk4::Label::new(None);
    values.set_xalign(1.0);
    values.add_css_class("dim-label");
    values.set_text(&format_calibration(*calibration.borrow()));
    summary.add_suffix(&values);
    group.add(&summary);

    let actions = adw::ActionRow::new();
    actions.set_title(&crate::tr!("Calibrate while controller is flat and still"));
    let calibrate = gtk4::Button::with_label(&crate::tr!("Calibrate"));
    calibrate.set_valign(gtk4::Align::Center);
    let reset = gtk4::Button::with_label(&crate::tr!("Reset"));
    reset.set_valign(gtk4::Align::Center);
    actions.add_suffix(&reset);
    actions.add_suffix(&calibrate);
    group.add(&actions);
    page.prepend(&group);

    let calibration_for_reset = calibration.clone();
    let values_for_reset = values.clone();
    let mark_dirty_for_reset = mark_dirty.clone();
    reset.connect_clicked(move |_| {
        *calibration_for_reset.borrow_mut() = ira_input::GyroCalibration::default();
        values_for_reset.set_text(&format_calibration(*calibration_for_reset.borrow()));
        mark_dirty_for_reset();
    });

    if device.is_none() {
        calibrate.set_sensitive(false);
        actions.set_subtitle(&crate::tr!(
            "Calibration requires one connected gyro-capable controller"
        ));
    }
    let Some(device) = device else { return };
    let calibration_for_dialog = calibration.clone();
    let values_for_dialog = values;
    let mark_dirty_for_dialog = mark_dirty.clone();
    let registry = registry.clone();
    calibrate.connect_clicked(move |button| {
        let Some(window) = button
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let calibration = calibration_for_dialog.clone();
        let values = values_for_dialog.clone();
        let mark_dirty = mark_dirty_for_dialog.clone();
        show_input_calibration_dialog(&window, registry.clone(), device.clone(), move |result| {
            match result {
                Ok(value) => {
                    *calibration.borrow_mut() = value;
                    values.set_text(&format_calibration(value));
                    mark_dirty();
                }
                Err(error) => eprintln!("Gyro calibration failed: {error}"),
            }
        });
    });
}

fn format_calibration(calibration: ira_input::GyroCalibration) -> String {
    format!(
        "X {:.3}  Y {:.3}  Z {:.3}",
        calibration.x, calibration.y, calibration.z
    )
}
