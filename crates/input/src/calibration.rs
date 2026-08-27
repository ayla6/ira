//! Per-controller gyro calibration storage.
//!
//! Calibration describes the *controller*, not the profile: bias measured on
//! one pad applies to every profile played with that pad. Entries are keyed
//! by USB vendor:product and kept in one JSON file in the app data dir; the
//! daemon receives its path via --calibration and seeds the gyro processor
//! from it.

use std::collections::HashMap;
use std::path::Path;

use crate::physical::DeviceInfo;
use crate::profile::ControllerCalibration;

/// `vendor:product` in lowercase hex, matching how controllers are identified
/// everywhere else.
pub fn device_key(vendor: u16, product: u16) -> String {
    format!("{vendor:04x}:{product:04x}")
}

/// The Nintendo button layout in effect for a controller: an explicitly
/// stored choice always wins, and a controller with no entry yet defaults
/// to its family's preference (on for Nintendo-family pads, off otherwise).
pub fn resolved_nintendo_layout(path: &Path, device: &DeviceInfo) -> bool {
    load_calibration(path, device.vendor, device.product)
        .map(|calibration| calibration.nintendo_layout)
        .unwrap_or_else(|| device.prefers_nintendo_layout())
}

/// A fresh calibration entry for a controller, seeded so that writing any
/// other per-controller value does not clobber the family's layout default:
/// an entry created implicitly carries the default the resolver would have
/// picked had no entry existed.
pub fn default_calibration_for(device: &DeviceInfo) -> ControllerCalibration {
    ControllerCalibration {
        nintendo_layout: device.prefers_nintendo_layout(),
        ..ControllerCalibration::default()
    }
}

/// The calibration file location for an app data directory.
pub fn calibration_store_path(save_dir: &str) -> std::path::PathBuf {
    Path::new(save_dir).join("controller_calibration.json")
}

pub fn load_calibration(path: &Path, vendor: u16, product: u16) -> Option<ControllerCalibration> {
    let text = std::fs::read_to_string(path).ok()?;
    let entries: HashMap<String, ControllerCalibration> = serde_json::from_str(&text).ok()?;
    entries.get(&device_key(vendor, product)).copied()
}

pub fn save_calibration(
    path: &Path,
    vendor: u16,
    product: u16,
    calibration: &ControllerCalibration,
) -> Result<(), String> {
    let mut entries: HashMap<String, ControllerCalibration> = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("could not parse controller calibration: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => return Err(format!("could not read controller calibration: {error}")),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create calibration folder: {error}"))?;
    }
    entries.insert(device_key(vendor, product), *calibration);
    let text = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("could not encode controller calibration: {error}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("could not write controller calibration: {error}"))
}

pub fn remove_calibration(path: &Path, vendor: u16, product: u16) -> Result<(), String> {
    let mut entries: HashMap<String, ControllerCalibration> = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("could not parse controller calibration: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("could not read controller calibration: {error}")),
    };
    entries.remove(&device_key(vendor, product));
    let text = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("could not encode controller calibration: {error}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("could not write controller calibration: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vendor: u16, name: &str) -> crate::physical::DeviceInfo {
        crate::physical::DeviceInfo {
            path: std::path::PathBuf::from("/dev/input/event9"),
            name: name.to_string(),
            vendor,
            product: 0x2009,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        }
    }

    fn write(path: &std::path::Path, vendor: u16, product: u16, bias: f32) {
        save_calibration(
            path,
            vendor,
            product,
            &ControllerCalibration {
                x: bias,
                y: 0.0,
                z: 0.0,
                stick_deadzone_left: 0.0,
                stick_deadzone_right: 0.0,
                nintendo_layout: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn test_device_key_formats_hex() {
        assert_eq!(device_key(0x2dc8, 0x6012), "2dc8:6012");
        assert_eq!(device_key(0x045e, 0x028e), "045e:028e");
    }

    #[test]
    fn test_calibration_roundtrips_per_device() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cal.json");
        write(&path, 0x2dc8, 0x6012, 0.05);
        write(&path, 0x054c, 0x0ce6, -0.03);

        let eight_bitdo = load_calibration(&path, 0x2dc8, 0x6012).unwrap();
        assert!((eight_bitdo.x - 0.05).abs() < 0.0001);
        let dualsense = load_calibration(&path, 0x054c, 0x0ce6).unwrap();
        assert!((dualsense.x + 0.03).abs() < 0.0001);
        assert!(load_calibration(&path, 0x057e, 0x2009).is_none());
    }

    #[test]
    fn test_missing_file_loads_nothing_and_remove_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("absent.json");
        assert!(load_calibration(&path, 1, 2).is_none());
        assert!(remove_calibration(&path, 1, 2).is_ok());
        write(&path, 1, 2, 0.1);
        assert!(remove_calibration(&path, 1, 2).is_ok());
        assert!(load_calibration(&path, 1, 2).is_none());
    }

    #[test]
    fn test_resolved_layout_defaults_on_for_nintendo_family_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cal.json");
        let pro = device(0x057e, "Nintendo Switch Pro Controller");
        let eight_bitdo = device(0x2dc8, "8BitDo Ultimate 2 Wireless Controller for PC");
        assert!(resolved_nintendo_layout(&path, &pro));
        assert!(!resolved_nintendo_layout(&path, &eight_bitdo));
    }

    #[test]
    fn test_resolved_layout_stored_choice_overrides_the_family_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cal.json");
        let pro = device(0x057e, "Nintendo Switch Pro Controller");
        let eight_bitdo = device(0x2dc8, "8BitDo Ultimate 2 Wireless Controller for PC");

        // Explicitly off on a Nintendo pad stays off...
        let mut entry = default_calibration_for(&pro);
        entry.nintendo_layout = false;
        save_calibration(&path, pro.vendor, pro.product, &entry).unwrap();
        assert!(!resolved_nintendo_layout(&path, &pro));

        // ...and explicitly on wins for a non-Nintendo pad.
        let mut entry = default_calibration_for(&eight_bitdo);
        entry.nintendo_layout = true;
        save_calibration(&path, eight_bitdo.vendor, eight_bitdo.product, &entry).unwrap();
        assert!(resolved_nintendo_layout(&path, &eight_bitdo));
    }

    #[test]
    fn test_default_entry_seeds_the_family_preference() {
        let pro = device(0x057e, "Nintendo Switch Pro Controller");
        assert!(default_calibration_for(&pro).nintendo_layout);
        let xbox = device(0x045e, "Xbox 360 Controller");
        assert!(!default_calibration_for(&xbox).nintendo_layout);
        // Seeding through the default keeps unrelated fields empty.
        let seeded = default_calibration_for(&pro);
        assert_eq!(seeded.stick_deadzone_left, 0.0);
        assert_eq!(seeded.x, 0.0);
    }
}
