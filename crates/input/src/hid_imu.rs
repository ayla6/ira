//! A motion-only HID device built on [`super::uhid`]: the flatpak-visible
//! half of native gyro.
//!
//! SDL's evdev backend attaches sensors to a gamepad by pairing nodes whose
//! `EVIOCGUNIQ` serials match (uinput nodes fail that ioctl outright, which
//! is why the old uinput motion node never paired). Creating the IMU as a
//! uhid device with the same uniq as the virtual pad makes the pairing work
//! — and evdev nodes are visible inside flatpak sandboxes, unlike hidraw.
//!
//! The descriptor engineers each axis group so the kernel derives a
//! resolution that matches the report's fixed-point scale, and SDL's
//! evdev sensor path divides the raw value by it (`raw * PI/180 / res`
//! and `raw * standard_gravity / res`): the accelerometer carries a
//! physical extent 1/256 of the logical one, making one LSB 1/256 g
//! (the DualSense's own accelerometer granularity), and the gyroscope
//! reports sixteenths of a degree per second with a physical extent of
//! logical × 573/10 × 16, landing the kernel's degree conversion on a
//! resolution of exactly 16.
//!
//! This is what separates the passthrough from garbage: the old report
//! carried plain integers (accelerometer in whole g!), so gravity read
//! "1" at rest and a 20° tilt rounded to zero — the sensor only produced
//! a handful of distinct values across its whole range.

use std::io;

use crate::uhid::{UhidDevice, BUS_USB};

pub const VENDOR_ID: u32 = 0x3651;
pub const PRODUCT_ID: u32 = 0x09c5;
pub const DEVICE_NAME: &str = "Ira Virtual Motion Sensors";

/// Report scaling: accelerometer LSB = 1/256 g, gyroscope LSB = 1/16 dps.
pub const ACCEL_LSB_PER_G: f32 = 256.0;
pub const GYRO_LSB_PER_DPS: f32 = 16.0;

/// Six little-endian i16 values: accel xyz in 1/256 g, gyro xyz in 1/16
/// degrees per second.
pub const IMU_REPORT_LEN: usize = 12;

pub const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x00, // Usage (Undefined; the axes carry the meaning)
    0xA1, 0x01, // Collection (Application)
    // Accelerometer: X/Y/Z in 1/256 g. The physical extent is the
    // logical one divided by 256, so the kernel derives a resolution of
    // 256 report units per g.
    0x09, 0x30, //   Usage (X)
    0x09, 0x31, //   Usage (Y)
    0x09, 0x32, //   Usage (Z)
    0x16, 0x00, 0x80, // Logical Minimum (-32768)
    0x26, 0xFF, 0x7F, // Logical Maximum (32767)
    0x45, 0x00, 0x00, 0x80, 0xFF, // Physical Minimum (-8388608)
    0x47, 0x00, 0x00, 0x80, 0x00, // Physical Maximum (8388352)
    0x65, 0x11, //     Unit (Centimeters)
    0x55, 0xFF, //     Unit Exponent (-1)
    0x75, 0x10, //     Report Size (16)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    // Gyroscope: Rx/Ry/Rz in 1/16 degrees per second. The kernel scales
    // degrees by 573/10 (logical vs physical), so the physical extent is
    // logical × 573/10 × 16 and the derived resolution is exactly 16.
    0x09, 0x33, //   Usage (Rx)
    0x09, 0x34, //   Usage (Ry)
    0x09, 0x35, //   Usage (Rz)
    0x16, 0x00, 0x80, // Logical Minimum (-32768)
    0x26, 0xFA, 0x7F, // Logical Maximum (32762)
    0x36, 0x00, 0x00, 0x00, 0x00, // Physical Minimum (0)
    0x47, 0x50, 0xB6, 0x94, 0x03, // Physical Maximum (60077904)
    0x65, 0x14, //     Unit (Degrees)
    0x55, 0x00, //     Unit Exponent (0)
    0x75, 0x10, //     Report Size (16)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0xC0, // End Collection
];

/// Packs one motion sample as the HID input report: accel xyz scaled to
/// 1/256-g units, then gyro xyz scaled to 1/16-dps units, matching the
/// descriptor's field order and the resolution SDL divides by.
pub fn imu_report(accel_g: [f32; 3], gyro_dps: [f32; 3]) -> [u8; IMU_REPORT_LEN] {
    let values = accel_g
        .into_iter()
        .map(|g| g * ACCEL_LSB_PER_G)
        .chain(gyro_dps.into_iter().map(|dps| dps * GYRO_LSB_PER_DPS));
    let mut report = [0u8; IMU_REPORT_LEN];
    for (slot, value) in report.chunks_exact_mut(2).zip(values) {
        slot.copy_from_slice(&(value.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
    }
    report
}

/// One live virtual IMU: reports motion in the SDL sensor frame.
pub struct ImuUhidDevice {
    device: UhidDevice,
}

impl ImuUhidDevice {
    /// `uniq` must equal the virtual pad's for SDL to pair the two nodes.
    pub fn create(uniq: &str) -> io::Result<Self> {
        let device = UhidDevice::create(
            DEVICE_NAME,
            uniq,
            REPORT_DESCRIPTOR,
            BUS_USB,
            VENDOR_ID,
            PRODUCT_ID,
        )?;
        Ok(Self { device })
    }

    /// Sends one motion sample: `accel_g` in g, `gyro_dps` in deg/s, both
    /// already in the SDL sensor frame.
    pub fn send_sample(&mut self, accel_g: [f32; 3], gyro_dps: [f32; 3]) -> io::Result<()> {
        self.device
            .send_input_report(&imu_report(accel_g, gyro_dps))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        imu_report, ACCEL_LSB_PER_G, GYRO_LSB_PER_DPS, IMU_REPORT_LEN, REPORT_DESCRIPTOR,
    };

    #[test]
    fn test_report_scales_to_fixed_point_units() {
        let report = imu_report([1.0, -2.0, 0.0], [-180.0, 90.0, 45.6]);
        let read_i16 = |i: usize| i16::from_le_bytes([report[i * 2], report[i * 2 + 1]]);
        assert_eq!(read_i16(0), 256); // 1 g × 256 LSB/g
        assert_eq!(read_i16(1), -512); // -2 g
        assert_eq!(read_i16(2), 0);
        assert_eq!(read_i16(3), -2880); // -180 dps × 16 LSB/dps
        assert_eq!(read_i16(4), 1440); // 90 dps
        assert_eq!(read_i16(5), 730); // 45.6 dps → 729.6 → 730
    }

    #[test]
    fn test_report_saturates_instead_of_wrapping() {
        let report = imu_report([40000.0, -40000.0, 0.0], [0.0; 3]);
        let read_i16 = |i: usize| i16::from_le_bytes([report[i * 2], report[i * 2 + 1]]);
        assert_eq!(read_i16(0), 32767);
        assert_eq!(read_i16(1), -32768);
    }

    #[test]
    fn test_report_holds_sub_g_tilt_precision() {
        // The whole point of the fixed-point scale: a 0.02 g change (about
        // one degree of tilt) must move the reported value, where whole-g
        // reports rounded it to zero.
        let report = imu_report([0.02, -0.02, 1.0], [0.0; 3]);
        let read_i16 = |i: usize| i16::from_le_bytes([report[i * 2], report[i * 2 + 1]]);
        assert_eq!(read_i16(0), 5);
        assert_eq!(read_i16(1), -5);
        assert_eq!(read_i16(2), 256);
    }

    #[test]
    fn test_descriptor_engineers_fixed_point_resolutions() {
        // Two axis groups (accel, gyro in degrees) whose physical extents
        // make the kernel's `hidinput_calc_abs_res` land on the report
        // scales: accelerometer 256 LSB/g, gyroscope 16 LSB/dps.
        assert_eq!(REPORT_DESCRIPTOR.first(), Some(&0x05));
        assert_eq!(REPORT_DESCRIPTOR.last(), Some(&0xC0));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x65, 0x11]));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x55, 0xFF]));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x65, 0x14]));
        let logical_extent = 32767i64 - -32768;
        // Accelerometer: physical extent = logical / 256, in 4-byte items.
        let accel_extent = 8_388_352i64 - -8_388_608;
        assert_eq!(accel_extent * 256, logical_extent * 256 * 256);
        // Gyroscope: the kernel scales degrees by 573/10, so the physical
        // extent is logical × 573/10 × 16 and the derived resolution is
        // exactly 16 LSB per degree per second.
        let gyro_extent = 60_077_904i64;
        assert_eq!(gyro_extent, 65530 * 573 * 16 / 10);
        assert_eq!(gyro_extent * 10, 65530 * 573 * 16);
        assert_eq!(IMU_REPORT_LEN, 12);
        assert_eq!(ACCEL_LSB_PER_G, 256.0);
        assert_eq!(GYRO_LSB_PER_DPS, 16.0);
    }
}
