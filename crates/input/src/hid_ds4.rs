//! A DualShock4-compatible controller built on [`super::uhid`].
//!
//! The identity walks a narrow line: a vendor whose PS4 pad is in SDL's
//! controller database (Hori) so the evdev twin carries a real PS4 mapping,
//! while also being in SDL's PlayStation-detection vendor list so its
//! hidapi DS4 driver probes our capabilities reply and claims the hidraw
//! node — and the kernel side, finding no vendor driver, lets hid-generic
//! claim it (Sony's own driver would reject our non-authentic DS4
//! descriptor mid-probe and leave no nodes behind). The capabilities
//! numerators (1/16 gyro degrees per second, 1/8192 accelerometer g) are
//! exactly the units [`usb_state_report`] emits, so motion arrives
//! correctly scaled with no calibration exchange.

use std::io;

use crate::motion_udp::{MotionSample, PadState};
use crate::uhid::{UhidDevice, BUS_USB};

/// Hori's PS4 mini pad identity: present in SDL's controller database as a
/// PS4 controller (so the evdev twin carries a real mapping and proper
/// type, instead of generic a/b/x/y), inside SDL's PlayStation-detection
/// vendor list (so its hidapi DS4 driver probes our capabilities reply and
/// claims the hidraw node), and unknown to Linux kernel HID drivers (so
/// hid-generic owns the device rather than a Sony-specific driver rejecting
/// our non-authentic DS4 descriptor mid-probe).
pub const VENDOR_ID: u32 = 0x0f0d;
pub const PRODUCT_ID: u32 = 0x00ee;
/// Real DS4 controllers literally report this product string; games and
/// launchers whitelist by name, so the virtual pad should read identically.
pub const DEVICE_NAME: &str = "Wireless Controller";

/// Feature report id SDL probes on third-party controllers to learn
/// capabilities and motion scaling numerators.
const FEATURE_CAPABILITIES: u8 = 0x03;
/// errno-style "no data available" for feature reports we do not serve.
const ENODATA: u16 = 61;

/// The 48-byte capabilities payload SDL requires (`size == 48 &&
/// data[2] == 0x27`): gamepad type, sensors enabled, and the motion
/// scaling numerators that make our raw counts read correctly.
fn capabilities_report() -> [u8; 48] {
    let mut r = [0u8; 48];
    r[0] = FEATURE_CAPABILITIES;
    r[2] = 0x27;
    r[4] = 0x02; // sensors supported
    r[5] = 0x00; // device type: gamepad
                 // SDL scales raw counts by (numerator / denominator): 1/16 turns our
                 // gyro counts into degrees per second, 1/8192 accelerometer counts
                 // into g.
    r[10..12].copy_from_slice(&1u16.to_le_bytes());
    r[12..14].copy_from_slice(&16u16.to_le_bytes());
    r[14..16].copy_from_slice(&1u16.to_le_bytes());
    r[16..18].copy_from_slice(&8192u16.to_le_bytes());
    r
}

/// SDL's DS4 driver expects the USB state report under report id 0x01.
const REPORT_ID_USB_STATE: u8 = 0x01;
pub const USB_STATE_REPORT_LEN: usize = 64;

/// Raw gyro counts per degree per second (matches SDL's 1/16 fallback).
const GYRO_COUNTS_PER_DPS: f32 = 16.0;
/// Raw accelerometer counts per g (matches SDL's 1/8192 fallback).
const ACCEL_COUNTS_PER_G: f32 = 8192.0;
/// Standard gravity in m/s^2; sensor samples arrive in SI units.
const GRAVITY_MS2: f32 = 9.80665;
/// Digital trigger bits engage past half travel, like the click synthesis.
const TRIGGER_CLICK_LEVEL: f32 = 0.5;

/// Report id 1 byte + PS4StatePacket_t (54 bytes) rounded out to the real
/// controller's 64-byte USB frame. Offsets are absolute report positions.
pub fn usb_state_report(
    pad: &PadState,
    sample: &MotionSample,
    timestamp_us: u64,
) -> [u8; USB_STATE_REPORT_LEN] {
    let mut r = [0u8; USB_STATE_REPORT_LEN];
    r[0] = REPORT_ID_USB_STATE;
    let stick = |value: f32| (((value + 1.0) / 2.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    r[1] = stick(pad.lx);
    r[2] = stick(pad.ly);
    r[3] = stick(pad.rx);
    r[4] = stick(pad.ry);

    // Byte 5: face buttons in the high nibble (west/south/east/north),
    // hat switch in the low nibble.
    let face = u8::from(pad.square)
        | u8::from(pad.cross) << 1
        | u8::from(pad.circle) << 2
        | u8::from(pad.triangle) << 3;
    r[5] = face << 4 | hat_value(pad);
    // Byte 6: L1, R1, digital L2/R2, share, options, L3, R3.
    r[6] = u8::from(pad.l1)
        | u8::from(pad.r1) << 1
        | u8::from(pad.l2 >= TRIGGER_CLICK_LEVEL) << 2
        | u8::from(pad.r2 >= TRIGGER_CLICK_LEVEL) << 3
        | u8::from(pad.back) << 4
        | u8::from(pad.start) << 5
        | u8::from(pad.l3) << 6
        | u8::from(pad.r3) << 7;
    // Byte 7: bit0 PS button, bits 2..7 a rolling sequence counter (SDL only
    // compares the byte for change and masks the two low bits).
    let sequence = ((timestamp_us >> 10) as u8 & 0x3F) << 2;
    r[7] = u8::from(pad.guide) | sequence;
    // Bytes 8/9: analog triggers, full pull = 255.
    r[8] = (pad.l2.clamp(0.0, 1.0) * 255.0).round() as u8;
    r[9] = (pad.r2.clamp(0.0, 1.0) * 255.0).round() as u8;
    // Bytes 10/11: sensor timestamp in 16/3 microsecond ticks; SDL derives
    // sample spacing from its deltas, so monotonic wrapping is what matters.
    let ticks = timestamp_us.wrapping_mul(3) / 16;
    r[10..12].copy_from_slice(&(ticks as u16).to_le_bytes());

    // SDL's PS4 driver passes sensor axes through untouched, and Cemu
    // applies the same [gx, -gy, -gz] correction here that it applies to
    // Nintendo pads — so the source SDL frame goes on the wire verbatim.
    write_i16(&mut r[13..15], sample.gyro_dps[0] * GYRO_COUNTS_PER_DPS);
    write_i16(&mut r[15..17], sample.gyro_dps[1] * GYRO_COUNTS_PER_DPS);
    write_i16(&mut r[17..19], sample.gyro_dps[2] * GYRO_COUNTS_PER_DPS);
    write_i16(
        &mut r[19..21],
        sample.accel_ms2[0] / GRAVITY_MS2 * ACCEL_COUNTS_PER_G,
    );
    write_i16(
        &mut r[21..23],
        sample.accel_ms2[1] / GRAVITY_MS2 * ACCEL_COUNTS_PER_G,
    );
    write_i16(
        &mut r[23..25],
        sample.accel_ms2[2] / GRAVITY_MS2 * ACCEL_COUNTS_PER_G,
    );

    // Byte 30: battery — level 5 of 10, discharging. Bytes 35/39: both touch
    // points up (bit 7 set means released).
    r[30] = 0x05;
    r[35] = 0x80;
    r[39] = 0x80;
    r
}

fn write_i16(target: &mut [u8], value: f32) {
    target.copy_from_slice(&(value.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
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

/// A minimal gamepad report descriptor for the kernel's evdev twin. SDL's
/// hidapi driver parses our raw report bytes directly and never consults
/// this; it exists so hid parsing succeeds and non-hidapi consumers get a
/// sane joystick node (sticks on X/Y/Z/Rz, hat on hat0, 13 buttons).
pub const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x05, // Usage (Gamepad)
    0xA1, 0x01, // Collection (Application)
    0xA1, 0x00, //   Collection (Physical)
    0x09, 0x30, //     Usage (X)
    0x09, 0x31, //     Usage (Y)
    0x09, 0x32, //     Usage (Z)
    0x09, 0x35, //     Usage (Rz)
    0x15, 0x00, //     Logical Minimum (0)
    0x26, 0xFF, 0x00, // Logical Maximum (255)
    0x75, 0x08, //     Report Size (8)
    0x95, 0x04, //     Report Count (4)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0xC0, //   End Collection
    0x09, 0x39, //   Usage (Hat switch)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x07, //   Logical Maximum (7)
    0x35, 0x00, //   Physical Minimum (0)
    0x46, 0x3B, 0x01, // Physical Maximum (315 degrees)
    0x65, 0x14, //   Unit (Eng Rot: degrees)
    0x75, 0x04, //   Report Size (4)
    0x95, 0x01, //   Report Count (1)
    0x81, 0x42, //   Input (Data, Variable, Null State)
    0x05, 0x09, //   Usage Page (Button)
    0x19, 0x01, //   Usage Minimum (Button 1)
    0x29, 0x0D, //   Usage Maximum (Button 13)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x0D, //   Report Count (13)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x03, //   Report Count (3)
    0x81, 0x03, //   Input (Constant) — pad to byte boundary
    0xC0, // End Collection
];

/// One live virtual DualShock4: owns the uhid device and forwards whole
/// controller states as USB input reports.
pub struct Ds4UhidDevice {
    device: UhidDevice,
}

impl Ds4UhidDevice {
    /// `uniq` becomes the pad's serial; the companion IMU device created
    /// with the same serial is what SDL pairs motion from on the evdev
    /// side (flatpak-visible).
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

    /// Pushes one whole-controller state and drains kernel events. Reports
    /// before any hidraw reader opened the node are simply discarded by the
    /// kernel, so polling is best-effort logging only.
    pub fn send_state(&mut self, pad: &PadState, sample: &MotionSample) -> io::Result<()> {
        self.device
            .send_input_report(&usb_state_report(pad, sample, sample.timestamp_us))?;
        for event in self.device.poll()? {
            match event {
                crate::uhid::UhidEvent::Open => {
                    eprintln!("ira-input: virtual DS4 opened by a reader");
                }
                crate::uhid::UhidEvent::Start => {
                    eprintln!("ira-input: virtual DS4 started by the kernel");
                }
                crate::uhid::UhidEvent::Stop => {
                    eprintln!("ira-input: virtual DS4 stopped by the kernel");
                }
                crate::uhid::UhidEvent::Close => {
                    eprintln!("ira-input: virtual DS4 closed by its reader");
                }
                crate::uhid::UhidEvent::GetReport { id, number, kind } => {
                    if number == FEATURE_CAPABILITIES && kind == crate::uhid::FEATURE_REPORT {
                        // SDL's third-party path: this reply is what enables
                        // sensors and sets the motion scaling numerators.
                        let _ = self.device.reply_get_report(id, 0, &capabilities_report());
                    } else {
                        let _ = self.device.reply_get_report(id, ENODATA, &[]);
                    }
                }
                other => {
                    eprintln!("ira-input: virtual DS4 event: {other:?}");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        capabilities_report, hat_value, usb_state_report, ACCEL_COUNTS_PER_G, GRAVITY_MS2,
        GYRO_COUNTS_PER_DPS, REPORT_DESCRIPTOR, USB_STATE_REPORT_LEN,
    };
    use crate::motion_udp::{MotionSample, PadState};

    fn resting_sample() -> MotionSample {
        MotionSample {
            accel_ms2: [0.0; 3],
            gyro_dps: [0.0; 3],
            timestamp_us: 0,
        }
    }

    #[test]
    fn test_report_descriptor_is_wellformed_collection() {
        // Application collection opens once and closes once; every other
        // item is two bytes or three for wide logical/physical maxima.
        assert_eq!(REPORT_DESCRIPTOR.first(), Some(&0x05));
        assert_eq!(REPORT_DESCRIPTOR.last(), Some(&0xC0));
        assert_eq!(REPORT_DESCRIPTOR.iter().filter(|&&b| b == 0xC0).count(), 2);
    }

    #[test]
    fn test_resting_report_reads_centered_and_released() {
        let report = usb_state_report(&PadState::default(), &resting_sample(), 0);
        assert_eq!(report.len(), USB_STATE_REPORT_LEN);
        assert_eq!(report[0], 0x01);
        assert_eq!(report[1], 128);
        assert_eq!(report[4], 128);
        assert_eq!(report[5], 0x08); // hat centered, no face buttons
        assert_eq!(report[6], 0);
        assert_eq!(report[8], 0); // triggers rest at zero
        assert_eq!(report[9], 0);
        assert_eq!(report[35], 0x80); // touchpad up
    }

    #[test]
    fn test_buttons_sticks_and_triggers_reach_their_bytes() {
        let pad = PadState {
            cross: true,
            circle: true,
            triangle: true,
            square: true,
            l1: true,
            r1: true,
            back: true,
            start: true,
            l3: true,
            r3: true,
            guide: true,
            dpad_up: true,
            dpad_right: true,
            lx: -1.0,
            ry: 1.0,
            l2: 1.0,
            r2: 0.5,
            ..PadState::default()
        };
        let report = usb_state_report(&pad, &resting_sample(), 0);
        // Faces west/south/east/north = bits 0-3 of the high nibble.
        assert_eq!(report[5], (0b1111 << 4) | 0x01); // up-right hat
                                                     // L1|R1|L2 click|R2 click|share|options|L3|R3 all set.
        assert_eq!(report[6], 0b11111111);
        assert_eq!(report[7] & 0x01, 1); // PS/guide
        assert_eq!(report[8], 255);
        assert_eq!(report[9], 128); // half pull rounds to mid-scale
        assert_eq!(report[1], 0); // lx -1 -> left extreme
        assert_eq!(report[4], 255); // ry +1 -> bottom extreme
    }

    #[test]
    fn test_motion_lands_in_si_scaled_counts() {
        let sample = MotionSample {
            accel_ms2: [GRAVITY_MS2, -GRAVITY_MS2, 0.0],
            gyro_dps: [90.0, -180.0, 2048.0],
            timestamp_us: 160_000, // -> 30000 ticks
        };
        let report = usb_state_report(&PadState::default(), &sample, sample.timestamp_us);
        let read_i16 = |offset: usize| i16::from_le_bytes([report[offset], report[offset + 1]]);
        assert_eq!(read_i16(13), (90.0 * GYRO_COUNTS_PER_DPS) as i16);
        // The wire carries the source frame verbatim: Cemu's own correction
        // ([gx, -gy, -gz]) matches what it applies to Nintendo pads.
        assert_eq!(read_i16(15), (-180.0 * GYRO_COUNTS_PER_DPS) as i16);
        assert_eq!(read_i16(17), 32767); // saturated, not wrapped
        assert_eq!(read_i16(19), ACCEL_COUNTS_PER_G as i16);
        assert_eq!(read_i16(21), -(ACCEL_COUNTS_PER_G as i16));
        assert_eq!(read_i16(23), 0);
        let tick = u16::from_le_bytes([report[10], report[11]]);
        assert_eq!(tick, 30_000);
    }

    #[test]
    fn test_capabilities_report_matches_sdl_third_party_probe() {
        let r = capabilities_report();
        assert_eq!(r[0], 0x03);
        assert_eq!(r[2], 0x27);
        assert_eq!(r[4] & 0x02, 0x02); // sensors enabled
        let read_u16 = |offset: usize| u16::from_le_bytes([r[offset], r[offset + 1]]);
        assert_eq!(read_u16(10), 1); // gyro numerator
        assert_eq!(read_u16(12), 16); // gyro denominator: counts per deg/s
        assert_eq!(read_u16(14), 1); // accel numerator
        assert_eq!(read_u16(16), 8192); // accel denominator: counts per g
    }

    #[test]
    fn test_hat_values_cover_the_compass() {
        let mut pad = PadState::default();
        assert_eq!(hat_value(&pad), 8);
        pad.dpad_left = true;
        assert_eq!(hat_value(&pad), 6);
        pad.dpad_down = true;
        assert_eq!(hat_value(&pad), 5);
        pad.dpad_left = false;
        assert_eq!(hat_value(&pad), 4);
        pad.dpad_up = true;
        pad.dpad_right = true;
        pad.dpad_down = false;
        assert_eq!(hat_value(&pad), 1);
    }
}
