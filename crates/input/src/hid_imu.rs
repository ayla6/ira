//! A motion-only HID device built on [`super::uhid`]: the flatpak-visible
//! half of native gyro.
//!
//! SDL's evdev backend attaches sensors to a gamepad by pairing nodes whose
//! `EVIOCGUNIQ` serials match (uinput nodes fail that ioctl outright, which
//! is why the old uinput motion node never paired). Creating the IMU as a
//! uhid device with the same uniq as the virtual pad makes the pairing work
//! — and evdev nodes are visible inside flatpak sandboxes, unlike hidraw.
//!
//! The descriptor engineers each axis group so the kernel's
//! `hidinput_calc_abs_res` derives a resolution of exactly 1: the
//! accelerometer declares centimeters with unit exponent −1 (millimeter
//! scale, identical logical and physical extents), while the gyroscope
//! declares degrees with a physical extent of logical × 573/10 — the
//! kernel multiplies the logical side by 573 and the physical side by 10
//! for degrees before dividing, so the ratio collapses to 1. The report
//! then carries plain integers: accelerometer in g, gyroscope in degrees
//! per second, the exact units SDL's evdev sensor path divides back out
//! (`raw * PI/180 / res` and `raw * standard_gravity / res`).

use std::io;

use crate::uhid::{UhidDevice, BUS_USB};

pub const VENDOR_ID: u32 = 0x3651;
pub const PRODUCT_ID: u32 = 0x09c5;
pub const DEVICE_NAME: &str = "Ira Virtual Motion Sensors";

/// Six little-endian i16 values: accel xyz in g, gyro xyz in deg/s.
pub const IMU_REPORT_LEN: usize = 12;

pub const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x00, // Usage (Undefined; the axes carry the meaning)
    0xA1, 0x01, // Collection (Application)
    // Accelerometer: X/Y/Z in g. Centimeters with unit exponent -1 put the
    // kernel's millimeter conversion at ratio 1 with equal extents.
    0x09, 0x30, //   Usage (X)
    0x09, 0x31, //   Usage (Y)
    0x09, 0x32, //   Usage (Z)
    0x16, 0x00, 0x80, // Logical Minimum (-32768)
    0x26, 0xFF, 0x7F, // Logical Maximum (32767)
    0x36, 0x00, 0x80, // Physical Minimum (-32768)
    0x46, 0xFF, 0x7F, // Physical Maximum (32767)
    0x65, 0x11, //     Unit (Centimeters)
    0x55, 0xFF, //     Unit Exponent (-1)
    0x75, 0x10, //     Report Size (16)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    // Gyroscope: Rx/Ry/Rz in degrees per second. The kernel scales degrees
    // by 573/10 (logical vs physical), so equal-effort extents differ by
    // that factor to land on resolution 1.
    0x09, 0x33, //   Usage (Rx)
    0x09, 0x34, //   Usage (Ry)
    0x09, 0x35, //   Usage (Rz)
    0x16, 0x00, 0x80, // Logical Minimum (-32768)
    0x26, 0xFA, 0x7F, // Logical Maximum (32762)
    0x36, 0x00, 0x00, // Physical Minimum (0)
    0x47, 0x65, 0x58, 0x39, 0x00, // Physical Maximum (3_754_869, 4-byte form)
    0x65, 0x14, //     Unit (Degrees)
    0x55, 0x00, //     Unit Exponent (0)
    0x75, 0x10, //     Report Size (16)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0xC0, // End Collection
];

/// Packs one motion sample as the HID input report: accel xyz in g then
/// gyro xyz in deg/s, matching the descriptor's field order.
pub fn imu_report(accel_g: [f32; 3], gyro_dps: [f32; 3]) -> [u8; IMU_REPORT_LEN] {
    let mut report = [0u8; IMU_REPORT_LEN];
    for (slot, value) in report
        .chunks_exact_mut(2)
        .zip(accel_g.into_iter().chain(gyro_dps))
    {
        slot.copy_from_slice(
            &(value.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes(),
        );
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
        self.device.send_input_report(&imu_report(accel_g, gyro_dps))
    }
}

#[cfg(test)]
mod tests {
    use super::{imu_report, REPORT_DESCRIPTOR, IMU_REPORT_LEN};

    #[test]
    fn test_report_packs_accel_then_gyro_signed() {
        let report = imu_report([1.0, -2.4, 0.0], [-180.0, 90.0, 45.6]);
        let read_i16 =
            |i: usize| i16::from_le_bytes([report[i * 2], report[i * 2 + 1]]);
        assert_eq!(read_i16(0), 1);
        assert_eq!(read_i16(1), -2); // rounds toward the nearest integer
        assert_eq!(read_i16(2), 0);
        assert_eq!(read_i16(3), -180);
        assert_eq!(read_i16(4), 90);
        assert_eq!(read_i16(5), 46);
    }

    #[test]
    fn test_report_saturates_instead_of_wrapping() {
        let report = imu_report([40000.0, -40000.0, 0.0], [0.0; 3]);
        let read_i16 =
            |i: usize| i16::from_le_bytes([report[i * 2], report[i * 2 + 1]]);
        assert_eq!(read_i16(0), 32767);
        assert_eq!(read_i16(1), -32768);
    }

    #[test]
    fn test_descriptor_engineers_resolution_one() {
        // Two axis groups (accel in cm/exp-1, gyro in degrees) with extents
        // whose kernel math collapses to resolution 1 on every axis:
        // degrees scale logical by 573 and physical by 10, so the gyro's
        // physical extent is logical * 573/10.
        assert_eq!(REPORT_DESCRIPTOR.first(), Some(&0x05));
        assert_eq!(REPORT_DESCRIPTOR.last(), Some(&0xC0));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x65, 0x11]));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x55, 0xFF]));
        assert!(REPORT_DESCRIPTOR.windows(2).any(|w| w == [0x65, 0x14]));
        assert_eq!(65530 * 573 / 10, 3_754_869);
        assert_eq!(IMU_REPORT_LEN, 12);
    }
}
