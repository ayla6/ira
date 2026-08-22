use super::input_calibration_dialog::show_input_calibration_dialog;
use adw::prelude::*;
use ira_input::{DeviceInfo, GyroCalibration};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Per-controller calibration: values live in a shared store keyed by
/// vendor:product, not in the profile — one calibration serves every profile
/// played with that pad.
pub(super) fn add_calibration_group(
    page: &gtk4::Box,
    store_path: PathBuf,
    device: Option<DeviceInfo>,
    registry: Arc<ira_input::ControllerRegistry>,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Calibration"));
    let summary = adw::ActionRow::new();
    summary.set_title(&crate::tr!("Gyro calibration"));
    summary.set_subtitle(&crate::tr!(
        "Stored per controller and applied to every profile"
    ));
    let values = gtk4::Label::new(None);
    values.set_xalign(1.0);
    values.add_css_class("dim-label");
    let refresh: Rc<dyn Fn()> = {
        let values = values.clone();
        let store_path = store_path.clone();
        let device = device.clone();
        Rc::new(move || {
            let text = match device.as_ref() {
                Some(device) => ira_input::load_calibration(
                    &store_path,
                    device.vendor,
                    device.product,
                )
                .map(format_calibration)
                .unwrap_or_else(|| crate::tr!("Not calibrated").to_string()),
                None => crate::tr!("No controller selected").to_string(),
            };
            values.set_text(&text);
        })
    };
    refresh();
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

    if device.is_none() {
        calibrate.set_sensitive(false);
        reset.set_sensitive(false);
        actions.set_subtitle(&crate::tr!(
            "Calibration requires one connected gyro-capable controller"
        ));
    }
    let Some(device) = device else { return };

    // Reset clears this controller's stored bias.
    {
        let store_path = store_path.clone();
        let device = device.clone();
        let refresh = refresh.clone();
        reset.connect_clicked(move |_| {
            if let Err(error) =
                ira_input::remove_calibration(&store_path, device.vendor, device.product)
            {
                eprintln!("Could not clear controller calibration: {error}");
            }
            refresh();
        });
    }

    calibrate.connect_clicked(move |button| {
        let Some(window) = button
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let store_path = store_path.clone();
        let device_for_save = device.clone();
        let refresh = refresh.clone();
        show_input_calibration_dialog(
            &window,
            registry.clone(),
            device.clone(),
            move |result| match result {
                Ok(value) => {
                    if let Err(error) = ira_input::save_calibration(
                        &store_path,
                        device_for_save.vendor,
                        device_for_save.product,
                        &value,
                    ) {
                        eprintln!("Could not save controller calibration: {error}");
                    }
                    refresh();
                }
                Err(error) => eprintln!("Gyro calibration failed: {error}"),
            },
        );
    });
}

fn format_calibration(calibration: GyroCalibration) -> String {
    format!(
        "X {:.3}  Y {:.3}  Z {:.3}",
        calibration.x, calibration.y, calibration.z
    )
}
