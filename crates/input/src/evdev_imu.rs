//! Motion read straight from a kernel sensor companion node.
//!
//! Controller drivers like `hid-nintendo` and `hid-sony` parse their pad's
//! IMU into a second evdev node (named `"<pad> (IMU)"` by hid-nintendo)
//! with calibrated accelerometer and gyroscope axes. Reading that node
//! directly needs no in-process SDL, no hidraw permissions, and no
//! name-matching against a dlopen'd library — the daemon's other gyro path
//! (`Sdl3SensorBackend`) stays as the fallback for pads the kernel does not
//! translate (8BitDo's DInput protocol, for example).
//!
//! hid-nintendo publishes accelerometer axes in counts per g and gyroscope
//! axes in counts per degree per second (its gyro values carry a ×1000
//! precision factor that the published resolution already includes), so
//! both convert with one division. Axes arrive in the device frame
//! (X toward the triggers, Y to the left, Z out of the face); SDL's
//! Nintendo shuffle `[-y, z, -x]` turns them into the sensor frame every
//! downstream consumer here speaks.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use evdev::{AbsoluteAxisCode, Device, KeyCode};

use crate::{DeviceInfo, SensorSample};

/// The companion-node suffix hid-nintendo appends to the pad's name.
const IMU_NAME_SUFFIX: &str = " (IMU)";

/// Opens the IMU companion node of a physical pad, when the kernel exposes
/// one. Discovery requires the full six-axis set and no face buttons (the
/// pad node itself also has ABS_X sticks, so the buttonless check is what
/// separates the sensor node), then prefers the exact `<pad> (IMU)` name
/// over any other same-identity sensor node.
pub fn discover_imu_node(pad: &DeviceInfo) -> Option<PathBuf> {
    discover_imu_node_in(Path::new("/dev/input"), pad)
}

/// [`discover_imu_node`] against an explicit evdev directory — probe tests
/// pass the host's tree where sandbox bind-mounts would hide freshly
/// created nodes.
pub fn discover_imu_node_in(dir: &Path, pad: &DeviceInfo) -> Option<PathBuf> {
    let mut by_identity = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("event"))
        {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        if !is_imu_companion(&device) {
            continue;
        }
        let name = device.name().unwrap_or_default();
        if name == format!("{}{IMU_NAME_SUFFIX}", pad.name) {
            return Some(path);
        }
        let id = device.input_id();
        if id.vendor() == pad.vendor && id.product() == pad.product {
            by_identity = by_identity.or(Some(path));
        }
    }
    by_identity
}

/// A sensor node carries all six motion axes and no gamepad face button;
/// stick-less and buttonless is what distinguishes it from the pad node,
/// whose ABS_X/Y are sticks rather than an accelerometer.
fn is_imu_companion(device: &Device) -> bool {
    // The daemon's paired virtual IMU ("Ira Virtual Motion Sensors")
    // carries six axes and no buttons too; it must never be mistaken for
    // a physical pad's companion. Kernel companions of our uhid twins
    // ("Ira Virtual ... (IMU)") stay eligible: they are created by
    // hid-nintendo exactly like a physical pad's.
    if device.name() == Some("Ira Virtual Motion Sensors") {
        return false;
    }
    let axes = device
        .supported_absolute_axes()
        .is_some_and(|axes| {
            [
                AbsoluteAxisCode::ABS_X,
                AbsoluteAxisCode::ABS_Y,
                AbsoluteAxisCode::ABS_Z,
                AbsoluteAxisCode::ABS_RX,
                AbsoluteAxisCode::ABS_RY,
                AbsoluteAxisCode::ABS_RZ,
            ]
            .into_iter()
            .all(|axis| axes.contains(axis))
        });
    let no_face_button = device
        .supported_keys()
        .is_none_or(|keys| !keys.contains(KeyCode::BTN_SOUTH));
    axes && no_face_button
}

/// Names of every evdev node in a directory that looks like a sensor node,
/// for startup diagnostics: when companion discovery fails, this tells a
/// log reader whether the driver registered no IMU node at all (empty) or
/// one under an unexpected name.
pub fn sensor_node_names(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("event"))
        {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        if is_imu_companion(&device) {
            names.push(device.name().unwrap_or_default().to_string());
        }
    }
    names
}

/// One live kernel IMU node, polled for motion samples.
pub struct EvdevImu {
    device: Device,
    /// Counts per g for the accelerometer axes.
    accel_resolution: f32,
    /// Counts per degree per second for the gyroscope axes.
    gyro_resolution: f32,
    accel_raw: [i32; 3],
    gyro_raw: [i32; 3],
}

impl EvdevImu {
    /// Opens the companion node discovered for `pad`. `None` when the
    /// kernel exposes no sensor node for it.
    pub fn open(pad: &DeviceInfo) -> Option<Self> {
        Self::open_in(Path::new("/dev/input"), pad)
    }

    /// [`open`] against an explicit evdev directory.
    pub fn open_in(dir: &Path, pad: &DeviceInfo) -> Option<Self> {
        let path = discover_imu_node_in(dir, pad)?;
        let device = Device::open(&path).ok()?;
        if device.set_nonblocking(true).is_err() {
            return None;
        }
        let resolutions: Vec<(AbsoluteAxisCode, evdev::AbsInfo)> =
            device.get_absinfo().ok()?.collect();
        let resolution = |code: AbsoluteAxisCode| -> Option<f32> {
            resolutions
                .iter()
                .find(|(axis, _)| *axis == code)
                .map(|(_, info)| info.resolution() as f32)
                .filter(|value| *value > 0.0)
        };
        Some(Self {
            accel_resolution: resolution(AbsoluteAxisCode::ABS_X)?,
            gyro_resolution: resolution(AbsoluteAxisCode::ABS_RX)?,
            device,
            accel_raw: [0; 3],
            gyro_raw: [0; 3],
        })
    }

    /// Drains pending axis updates into one sample. Multiple kernel sensor
    /// batches in a single drain collapse to the newest values; callers poll
    /// at report rate anyway.
    pub fn read(&mut self, fallback_timestamp_us: u64) -> Result<Option<SensorSample>, String> {
        let events = self
            .device
            .fetch_events()
            .map_err(|error| format!("failed reading IMU node: {error}"))?
            .collect::<Vec<_>>();
        let mut timestamp_us = None;
        let mut updated = false;
        for event in events {
            if let evdev::EventSummary::AbsoluteAxis(_, code, value) = event.destructure() {
                let slot = match code {
                    AbsoluteAxisCode::ABS_X => Some((&mut self.accel_raw, 0)),
                    AbsoluteAxisCode::ABS_Y => Some((&mut self.accel_raw, 1)),
                    AbsoluteAxisCode::ABS_Z => Some((&mut self.accel_raw, 2)),
                    AbsoluteAxisCode::ABS_RX => Some((&mut self.gyro_raw, 0)),
                    AbsoluteAxisCode::ABS_RY => Some((&mut self.gyro_raw, 1)),
                    AbsoluteAxisCode::ABS_RZ => Some((&mut self.gyro_raw, 2)),
                    _ => None,
                };
                if let Some((axes, index)) = slot {
                    axes[index] = value;
                    updated = true;
                    timestamp_us = Some(
                        event
                            .timestamp()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.as_micros() as u64)
                            .unwrap_or(fallback_timestamp_us),
                    );
                }
            }
        }
        if !updated {
            return Ok(None);
        }
        Ok(Some(imu_sample(
            self.accel_raw,
            self.gyro_raw,
            self.accel_resolution,
            self.gyro_resolution,
            timestamp_us.unwrap_or(fallback_timestamp_us),
        )))
    }
}

/// Converts raw axis values (accelerometer counts, gyroscope counts) into a
/// sensor-frame sample: accelerometer in m/s² and gyroscope in rad/s — the
/// units every consumer of [`SensorSample`] applies its gravity division
/// to — both after the Nintendo device→sensor axis shuffle.
pub fn imu_sample(
    accel_raw: [i32; 3],
    gyro_raw: [i32; 3],
    accel_resolution: f32,
    gyro_resolution: f32,
    timestamp_us: u64,
) -> SensorSample {
    let to_device_frame =
        |raw: [i32; 3], resolution: f32| -> [f32; 3] {
            [raw[0] as f32 / resolution, raw[1] as f32 / resolution, raw[2] as f32 / resolution]
        };
    let accel_g = to_device_frame(accel_raw, accel_resolution);
    let gyro_dps = to_device_frame(gyro_raw, gyro_resolution);
    const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
    const GRAVITY_MS2: f32 = 9.80665;
    SensorSample {
        gyro: [
            -gyro_dps[1] * DEG_TO_RAD,
            gyro_dps[2] * DEG_TO_RAD,
            -gyro_dps[0] * DEG_TO_RAD,
        ],
        accel: Some([
            -accel_g[1] * GRAVITY_MS2,
            accel_g[2] * GRAVITY_MS2,
            -accel_g[0] * GRAVITY_MS2,
        ]),
        timestamp_us,
    }
}

#[cfg(test)]
mod tests {
    use super::imu_sample;

    #[test]
    fn test_imu_sample_divides_by_resolution_and_shuffles_axes() {
        // hid-nintendo's published resolutions: 4096 counts per g, 14247
        // counts per degree per second (precision factor included).
        let sample = imu_sample(
            [4096, -8192, 16384], // +1 g, -2 g, +4 g in the device frame
            [14247, 0, -28494],   // +1, 0, -2 degrees per second
            4096.0,
            14247.0,
            99,
        );
        // Sensor frame: [-y, z, -x], accel in m/s².
        let gravity = 9.80665_f32;
        assert!((sample.accel.unwrap()[0] - 2.0 * gravity).abs() < 0.01);
        assert!((sample.accel.unwrap()[1] - 4.0 * gravity).abs() < 0.02);
        assert!((sample.accel.unwrap()[2] + 1.0 * gravity).abs() < 0.01);
        assert!((sample.gyro[0] - 0.0).abs() < 1e-4);
        assert!((sample.gyro[1] + 2.0 * std::f32::consts::PI / 180.0).abs() < 1e-4);
        assert!((sample.gyro[2] + 1.0 * std::f32::consts::PI / 180.0).abs() < 1e-4);
        assert_eq!(sample.timestamp_us, 99);
    }

    #[test]
    fn test_imu_sample_round_trips_the_twin_shuffle() {
        // The virtual Switch twin pre-shuffles SDL-frame values into the
        // device frame with [-z, -x, y]; reading a kernel node back must
        // undo to the original SDL frame, not double-apply.
        let sdl_accel_ms2 = [0.25, -0.5, 1.0];
        let device_frame_g = [
            -sdl_accel_ms2[2] / 9.80665,
            -sdl_accel_ms2[0] / 9.80665,
            sdl_accel_ms2[1] / 9.80665,
        ];
        let raw: [i32; 3] = device_frame_g.map(|value| (value * 4096.0) as i32);
        let sample = imu_sample(raw, [0; 3], 4096.0, 14247.0, 1);
        for (recovered, expected) in sample.accel.unwrap().iter().zip(sdl_accel_ms2) {
            assert!((recovered - expected).abs() < 0.02);
        }
    }
}
