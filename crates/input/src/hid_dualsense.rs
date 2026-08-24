//! A virtual DualSense built on [`super::uhid`]: SDL's PS5 hidapi driver
//! claims it through the licensed HORI VID/PID (typed PS5 in SDL's
//! controller table, ignored by kernel drivers just like our DS4 pad) and
//! parses raw report bytes directly. Two feature replies make the
//! third-party path work: capabilities advertise sensor support, and a
//! crafted identity calibration makes wire counts decode straight to
//! degrees/s and g.

use std::io;

use crate::motion_udp::{MotionSample, PadState};
use crate::uhid::{UhidDevice, UhidEvent, BUS_USB};

pub const VENDOR_ID: u32 = 0x0f0d;
pub const PRODUCT_ID: u32 = 0x0163;
/// Real DualSense hardware reports this exact product string; games and
/// launchers whitelist by name, and SDL's PS5 classification comes from the
/// VID/PID, not the name.
pub const DEVICE_NAME: &str = "DualSense Wireless Controller";

const FEATURE_CAPABILITIES: u8 = 0x03;
const FEATURE_CALIBRATION: u8 = 0x05;
const ENODATA: u16 = 61;

/// Raw counts per degree/s. The served calibration decodes sensitivity 64
/// against SDL's GYRO_RES_PER_DEGREE of 1024, i.e. counts/16 are deg/s —
/// the same scale SDL's un-calibrated fallback assumes.
const GYRO_COUNTS_PER_DPS: f32 = 16.0;
/// Raw counts per g against SDL's ACCEL_RES_PER_G.
const ACCEL_COUNTS_PER_G: f32 = 8192.0;
/// SDL derives sample spacing from this counter in 1/3 microsecond units
/// on USB; Cemu additionally drops any non-advancing timestamps.
const SENSOR_TICKS_PER_US: u32 = 3;
/// A battery/connection byte reading full on USB.
const BATTERY_FULL_USB: u8 = 100;
const GRAVITY_MS2: f32 = 9.80665;

const USB_STATE_REPORT_LEN: usize = 64;

/// Same rationale as the DS4 descriptor: SDL never consults it, but the
/// kernel needs a parseable gamepad for its evdev twin. The lone constant
/// input bit matters more than it looks: without any Input item the kernel
/// registers no input report and drops every UHID_INPUT2 before hidraw.
pub const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x05, // Usage (Gamepad)
    0xA1, 0x01, // Collection (Application)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x01, //   Report Count (1)
    0x81, 0x03, //   Input (Constant)
    0xC0, // End Collection
];

/// One live virtual DualSense: owns the uhid device and forwards whole
/// controller states as USB input reports.
pub struct DualsenseUhidDevice {
    device: UhidDevice,
    sequence: u32,
}

impl DualsenseUhidDevice {
    /// `uniq` becomes the pad's serial, matching how the other flavors pair
    /// companion devices.
    pub fn create(uniq: &str) -> io::Result<Self> {
        let device = UhidDevice::create(
            DEVICE_NAME,
            uniq,
            REPORT_DESCRIPTOR,
            BUS_USB,
            VENDOR_ID,
            PRODUCT_ID,
        )?;
        Ok(Self {
            device,
            sequence: 0,
        })
    }

    /// Pushes one whole-controller state and drains kernel events, serving
    /// the feature reports SDL's third-party probe asks for.
    pub fn send_state(&mut self, pad: &PadState, sample: &MotionSample) -> io::Result<()> {
        self.sequence = self.sequence.wrapping_add(1);
        let mut report = usb_state_report(pad, sample);
        report[12..16].copy_from_slice(&self.sequence.to_le_bytes());
        self.device.send_input_report(&report)?;
        for event in self.device.poll()? {
            if let UhidEvent::GetReport { id, number, kind } = event {
                if kind != crate::uhid::FEATURE_REPORT {
                    let _ = self.device.reply_get_report(id, ENODATA, &[]);
                } else if number == FEATURE_CAPABILITIES {
                    let _ = self.device.reply_get_report(id, 0, &capabilities_report());
                } else if number == FEATURE_CALIBRATION {
                    let _ = self.device.reply_get_report(id, 0, &calibration_report());
                } else {
                    let _ = self.device.reply_get_report(id, ENODATA, &[]);
                }
            }
        }
        Ok(())
    }
}

fn usb_state_report(pad: &PadState, sample: &MotionSample) -> Vec<u8> {
    let mut r = vec![0u8; USB_STATE_REPORT_LEN];
    r[0] = 0x01;
    let stick = |value: f32| (((value.clamp(-1.0, 1.0) + 1.0) / 2.0) * 255.0).round() as u8;
    r[1] = stick(pad.lx);
    r[2] = stick(pad.ly);
    r[3] = stick(pad.rx);
    r[4] = stick(pad.ry);
    r[5] = (pad.l2.clamp(0.0, 1.0) * 255.0).round() as u8;
    r[6] = (pad.r2.clamp(0.0, 1.0) * 255.0).round() as u8;

    // Buttons/hat word: face buttons in the high nibble, hat in the low.
    let face = u8::from(pad.square) << 4
        | u8::from(pad.cross) << 5
        | u8::from(pad.circle) << 6
        | u8::from(pad.triangle) << 7;
    r[8] = face | hat_value(pad);
    r[9] = u8::from(pad.l1)
        | u8::from(pad.r1) << 1
        | u8::from(pad.l2 >= 0.5) << 2
        | u8::from(pad.r2 >= 0.5) << 3
        | u8::from(pad.back) << 4
        | u8::from(pad.start) << 5
        | u8::from(pad.l3) << 6
        | u8::from(pad.r3) << 7;
    r[10] = u8::from(pad.guide);

    let write_i16 = |target: &mut [u8], value: f32| {
        target.copy_from_slice(&(value.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
    };
    for axis in 0..3 {
        write_i16(
            &mut r[16 + axis * 2..18 + axis * 2],
            sample.gyro_dps[axis] * GYRO_COUNTS_PER_DPS,
        );
        write_i16(
            &mut r[22 + axis * 2..24 + axis * 2],
            sample.accel_ms2[axis] / GRAVITY_MS2 * ACCEL_COUNTS_PER_G,
        );
    }
    let ticks =
        (sample.timestamp_us.min(u32::MAX as u64 / 3) as u32).wrapping_mul(SENSOR_TICKS_PER_US);
    r[28..32].copy_from_slice(&ticks.to_le_bytes());

    // Both touch points released (high bit set), full battery, USB link.
    r[33] = 0x80;
    r[37] = 0x80;
    r[53] = BATTERY_FULL_USB;
    r[54] = 0x08;
    r
}

/// Hat switch values: 0=up through 7=up-left clockwise, 8=centered.
fn hat_value(pad: &PadState) -> u8 {
    match (pad.dpad_up, pad.dpad_right, pad.dpad_down, pad.dpad_left) {
        (true, false, false, false) => 0,
        (true, true, false, false) => 1,
        (false, true, false, false) => 2,
        (false, true, true, false) => 3,
        (false, false, true, false) => 4,
        (false, false, true, true) => 5,
        (false, false, false, true) => 6,
        (true, false, false, true) => 7,
        _ => 8,
    }
}

/// SDL's third-party probe requires 48 bytes with magic 0x28 at [2]; the
/// capability bits then enable sensors, rumble, lightbar and touchpad.
fn capabilities_report() -> Vec<u8> {
    let mut r = vec![0u8; 48];
    r[0] = FEATURE_CAPABILITIES;
    r[2] = 0x28;
    r[4] = 0x02 // sensors
        | 0x04  // lightbar
        | 0x08  // vibration
        | 0x40; // touchpad
    r[5] = 0x00; // device type: gamepad
    r[20] = 0x80 // player LEDs
        | 0x01; // battery reporting
    r
}

/// Identity IMU calibration: biases at zero, gyro sensitivity decoding to
/// SDL's expected 64 (so counts/16 are degrees per second) and accel
/// ranges of exactly 2 g around center (so counts are g against 8192).
fn calibration_report() -> Vec<u8> {
    let mut r = vec![0u8; 35];
    r[0] = FEATURE_CALIBRATION;
    let write_i16 = |target: &mut [u8], value: i16| {
        target.copy_from_slice(&value.to_le_bytes());
    };
    for axis in 0..3 {
        write_i16(&mut r[7 + axis * 4..9 + axis * 4], 16384);
        write_i16(&mut r[9 + axis * 4..11 + axis * 4], -16384);
    }
    write_i16(&mut r[19..21], 1024);
    write_i16(&mut r[21..23], 1024);
    for axis in 0..3 {
        write_i16(&mut r[23 + axis * 4..25 + axis * 4], 8192);
        write_i16(&mut r[25 + axis * 4..27 + axis * 4], -8192);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::{
        calibration_report, capabilities_report, hat_value, usb_state_report, MotionSample,
        PadState,
    };

    #[test]
    fn test_capabilities_match_sdl_third_party_probe() {
        let r = capabilities_report();
        assert_eq!(r.len(), 48);
        assert_eq!(r[2], 0x28);
        assert_eq!(r[4] & 0x02, 0x02); // sensors supported
        assert_eq!(r[5], 0x00); // gamepad device type
    }

    #[test]
    fn test_calibration_decodes_identity_units() {
        let read = |offset: usize| {
            i16::from_le_bytes([
                calibration_report()[offset],
                calibration_report()[offset + 1],
            ])
        };
        // Sensitivity = (speed+ + speed-) * 1024 / (plus - minus) must be
        // exactly 64 so counts/16 decode as degrees per second.
        let speed_sum = i32::from(read(19)) + i32::from(read(21));
        let span = i32::from(read(7)) - i32::from(read(9));
        assert_eq!(speed_sum * 1024 / span, 64);
        for offset in [7, 11, 15] {
            assert_eq!(read(offset), 16384);
            assert_eq!(read(offset + 2), -16384);
        }
        // Accel: range exactly 2 g around zero bias against 8192 per g.
        for offset in [23, 27, 31] {
            assert_eq!(read(offset), 8192);
            assert_eq!(read(offset + 2), -8192);
        }
    }

    #[test]
    fn test_report_layout_buttons_and_sensors() {
        let pad = PadState {
            square: true,
            l1: true,
            guide: true,
            dpad_right: true,
            ..PadState::default()
        };
        let sample = MotionSample {
            accel_ms2: [9.80665, -9.80665, 0.0],
            gyro_dps: [90.0, 0.0, -1024.0],
            timestamp_us: 30_000,
        };
        let r = usb_state_report(&pad, &sample);
        assert_eq!(r[0], 0x01);
        assert_eq!(r[8] >> 4, 1 << 0); // square in the high nibble
        assert_eq!(r[8] & 0x0F, 2); // right hat
        assert_eq!(r[9] & 0x03, 0b01); // L1 only
        assert_eq!(r[10] & 0x01, 1); // guide
        let read_i16 = |offset: usize| i16::from_le_bytes([r[offset], r[offset + 1]]);
        assert_eq!(read_i16(16), (90.0 * 16.0) as i16);
        assert_eq!(read_i16(20), (-1024.0 * 16.0) as i16);
        assert_eq!(read_i16(22), 8192);
        assert_eq!(read_i16(24), -8192);
        let ticks = u32::from_le_bytes([r[28], r[29], r[30], r[31]]);
        assert_eq!(ticks, 90_000); // microseconds times three
        assert_eq!(r[53], 100);
        assert_eq!(r[54], 0x08);
    }

    #[test]
    fn test_hat_values_cover_the_compass() {
        let mut pad = PadState::default();
        assert_eq!(hat_value(&pad), 8);
        pad.dpad_left = true;
        assert_eq!(hat_value(&pad), 6);
    }
}
