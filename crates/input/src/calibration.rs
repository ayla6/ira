//! Per-controller gyro calibration storage.
//!
//! Calibration describes the *controller*, not the profile: bias measured on
//! one pad applies to every profile played with that pad. Entries are keyed
//! by USB vendor:product and kept in one JSON file in the app data dir; the
//! daemon receives its path via --calibration and seeds the gyro processor
//! from it.

use std::collections::HashMap;
use std::path::Path;

use crate::profile::ControllerCalibration;

/// `vendor:product` in lowercase hex, matching how controllers are identified
/// everywhere else.
pub fn device_key(vendor: u16, product: u16) -> String {
    format!("{vendor:04x}:{product:04x}")
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
}
