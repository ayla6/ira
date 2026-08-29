//! A virtual DualSense built on [`super::uhid`]: SDL's PS5 hidapi driver
//! claims it through the licensed HORI VID/PID (typed PS5 in SDL's
//! controller table, ignored by kernel drivers just like our DS4 pad) and
//! parses raw report bytes directly. Two feature replies make the
//! third-party path work: capabilities advertise sensor support, and a
//! crafted identity calibration makes wire counts decode straight to
//! degrees/s and g.

use std::io;

use crate::motion_udp::{MotionSample, PadState};
use crate::rumble::RumbleCommand;
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
const GRAVITY_MS2: f32 = 9.80665;

const USB_STATE_REPORT_LEN: usize = 64;

/// The kernel's evdev twin descriptor, byte-for-byte the wire prefix of
/// [`usb_state_report`] (SDL's PS5 wire layout): report id 0x01, four stick
/// axes, analog triggers on bytes 5/6, a reserved byte, the hat plus face
/// nibbles, the shoulder/system buttons, and the guide bit. The previous
/// stub declared a single constant input, so the twin had no controls at
/// all and non-hidapi consumers (RetroArch's udev driver) saw an empty
/// pad. Faces use individual usages so they land on the standard evdev
/// buttons despite the wire's square/cross/circle/triangle bit order.
pub const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x05, // Usage (Gamepad)
    0xA1, 0x01, // Collection (Application)
    0x85, 0x01, //   Report ID (1)
    0x09, 0x30, //   Usage (X)      — left stick x
    0x09, 0x31, //   Usage (Y)      — left stick y
    0x09, 0x32, //   Usage (Z)      — right stick x
    0x09, 0x35, //   Usage (Rz)     — right stick y
    0x09, 0x33, //   Usage (Rx)     — L2 analog
    0x09, 0x34, //   Usage (Ry)     — R2 analog
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xFF, 0x00, // Logical Maximum (255)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x06, //   Report Count (6)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x01, //   Report Count (1)
    0x81, 0x03, //   Input (Constant) — reserved report byte
    0x05, 0x01, //   Usage Page (Generic Desktop)
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
    0x09, 0x05, //   Usage (West) — square
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x01, //   Report Count (1)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x09, 0x01, //   Usage (South) — cross
    0x81, 0x02, //   Input
    0x09, 0x02, //   Usage (East) — circle
    0x81, 0x02, //   Input
    0x09, 0x04, //   Usage (North) — triangle
    0x81, 0x02, //   Input
    0x19, 0x07, //   Usage Minimum (TL)     — L1
    0x29, 0x08, //   Usage Maximum (TR)     — R1
    0x95, 0x02, //   Report Count (2)
    0x81, 0x02, //   Input
    0x19, 0x09, //   Usage Minimum (TL2)    — digital L2
    0x29, 0x0A, //   Usage Maximum (TR2)    — digital R2
    0x81, 0x02, //   Input
    0x19, 0x0B, //   Usage Minimum (Select) — create
    0x29, 0x0C, //   Usage Maximum (Start)  — options
    0x81, 0x02, //   Input
    0x19, 0x0E, //   Usage Minimum (Thumb L)
    0x29, 0x0F, //   Usage Maximum (Thumb R)
    0x81, 0x02, //   Input
    0x09, 0x0D, //   Usage (Mode) — guide
    0x81, 0x02, //   Input
    0x75, 0x01, //   Report Size (1)
    0x95, 0x07, //   Report Count (7)
    0x81, 0x03, //   Input (Constant) — touchpad click and padding
    0xC0, // End Collection
];

/// One live virtual DualSense: owns the uhid device and forwards whole
/// controller states as USB input reports.
pub struct DualsenseUhidDevice {
    device: UhidDevice,
    sequence: u32,
    /// Rumble the game played on this pad, drained by the daemon for replay
    /// on the physical controller.
    pending_rumble: Vec<RumbleCommand>,
    /// Last sent report with the volatile fields (sequence, timestamp,
    /// touchpad counters) zeroed, so idle ticks skip the wire write.
    last_state: Option<[u8; USB_STATE_REPORT_LEN]>,
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
            pending_rumble: Vec::new(),
            last_state: None,
        })
    }

    /// Pushes one whole-controller state and drains kernel events, serving
    /// the feature reports SDL's third-party probe asks for.
    pub fn send_state(&mut self, pad: &PadState, sample: &MotionSample) -> io::Result<()> {
        self.sequence = self.sequence.wrapping_add(1);
        let mut report = usb_state_report(pad, sample);
        report[12..16].copy_from_slice(&self.sequence.to_le_bytes());
        let mut stable = report.clone();
        stable[12..16].fill(0); // sequence
        stable[28..30].fill(0); // sensor timestamp
        stable[32] = 0; // touchpad counters count up
        stable[36] = 0;
        let stable: [u8; USB_STATE_REPORT_LEN] =
            stable.try_into().expect("fixed report length");
        if self.last_state == Some(stable) {
            return self.service();
        }
        self.last_state = Some(stable);
        self.device.send_input_report(&report)?;
        self.service()
    }

    /// Drains kernel events and serves SDL's feature probes without sending
    /// a state report. Run every daemon pass: probes arrive whenever a
    /// reader opens the pad, including while the daemon considers itself
    /// paused or between report-rate ticks.
    pub fn service(&mut self) -> io::Result<()> {
        for event in self.device.poll()? {
            match event {
                UhidEvent::GetReport { id, number, kind } => {
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
                UhidEvent::OutputReport { data } => {
                    if let Some(command) = rumble_command_from_output_report(&data) {
                        self.pending_rumble.push(command);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Rumble commands decoded since the last drain, in arrival order. A
    /// zero-magnitude command stops the motors.
    pub fn take_rumble(&mut self) -> Vec<RumbleCommand> {
        std::mem::take(&mut self.pending_rumble)
    }
}

/// DualSense output reports carry no duration; drivers stop the motors with
/// an explicit zeroed report, so the replayed effect only needs to outlive
/// the gaps between reports.
const DECODED_RUMBLE_MS: u16 = 250;

/// The effects-report enable bit that arms the emulated rumble motors.
const RUMBLE_EMULATION_ENABLE: u8 = 0x01;

/// Extracts the rumble request from a DualSense output report. SDL's USB
/// effects report 0x02 gates both motors behind enableBits1 bit 0, with the
/// right/weak magnitude at [3] and left/strong at [4]; Bluetooth drivers
/// use the tagged 0x31 report where byte 1 flags each motor separately
/// (bit 0 right/weak, bit 1 left/strong), bytes 2 and 3 the magnitudes.
pub(crate) fn rumble_command_from_output_report(report: &[u8]) -> Option<RumbleCommand> {
    match report.first()? {
        0x02 if report.len() >= 5 => Some(RumbleCommand {
            strong: dualsense_motor(report[1] & RUMBLE_EMULATION_ENABLE, report[4]),
            weak: dualsense_motor(report[1] & RUMBLE_EMULATION_ENABLE, report[3]),
            duration_ms: DECODED_RUMBLE_MS,
        }),
        0x31 if report.len() >= 4 => Some(RumbleCommand {
            strong: dualsense_motor(report[1] & 0x02, report[3]),
            weak: dualsense_motor(report[1] & 0x01, report[2]),
            duration_ms: DECODED_RUMBLE_MS,
        }),
        _ => None,
    }
}

/// HID motor byte (0..=255) to the evdev magnitude scale (0..=65535);
/// unflagged motors read as off.
fn dualsense_motor(flagged: u8, byte: u8) -> u16 {
    if flagged == 0 {
        return 0;
    }
    u16::from(byte) * 257
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
    // Third-party pads use SDL's alternate state layout: a 16-bit counter
    // in plain microseconds here, sample spacing derived from its wrapped
    // deltas. Cemu drops samples whose timestamp does not advance, so
    // plain wrapping matches what real licensed hardware emits.
    r[28..30].copy_from_slice(&(sample.timestamp_us as u16).to_le_bytes());

    // Alternate-layout status bytes: the touchpad counters live at wire
    // 32/36 (their high bit set means "finger up", so a zero there reads as
    // a permanently pressed phantom finger), and the battery byte at 30
    // packs status in the high nibble (2 = fully charged) with the level
    // below it.
    r[30] = 0x20 | 10;
    r[32] = 0x80;
    r[36] = 0x80;
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
        let tick = u16::from_le_bytes([r[28], r[29]]);
        assert_eq!(tick, 30_000); // microseconds, straight into the u16
        // Alternate layout: touchpad counters at 32/36 with the released
        // bit set, battery at 30 reading fully charged (status 2).
        assert_eq!(r[32], 0x80);
        assert_eq!(r[36], 0x80);
        assert_eq!(r[33], 0);
        assert_eq!(r[30] >> 4, 2);
    }

    #[test]
    fn test_hat_values_cover_the_compass() {
        let mut pad = PadState::default();
        assert_eq!(hat_value(&pad), 8);
        pad.dpad_left = true;
        assert_eq!(hat_value(&pad), 6);
    }

    #[test]
    fn test_rumble_decoded_from_usb_effects_report() {
        // SDL's USB effects packet: enableBits1 bit 0 arms both motors,
        // [3] is right/weak, [4] left/strong.
        let report = [0x02, 0x01, 0x00, 0x40, 0xFF, 0x00];
        let command = super::rumble_command_from_output_report(&report).expect("effects");
        assert_eq!(command.strong, 255 * 257);
        assert_eq!(command.weak, 64 * 257);
        // Without the emulation bit nothing vibrates despite magnitudes.
        let disabled = super::rumble_command_from_output_report(&[0x02, 0x02, 0, 0x40, 0xFF])
            .expect("effects packet still parses");
        assert_eq!((disabled.strong, disabled.weak), (0, 0));
    }

    #[test]
    fn test_rumble_decoded_from_output_report() {
        // Report 0x31: flag bit 0 right/weak at [2], bit 1 left/strong at [3].
        let report = [0x31, 0x03, 0x40, 0xFF, 0x00];
        let command = super::rumble_command_from_output_report(&report).expect("rumble report");
        assert_eq!(command.strong, 255 * 257);
        assert_eq!(command.weak, 64 * 257);
        // An unflagged motor reads as off even with a nonzero magnitude.
        let one_sided = [0x31, 0x01, 0x40, 0xFF, 0x00];
        let command = super::rumble_command_from_output_report(&one_sided).expect("rumble report");
        assert_eq!((command.strong, command.weak), (0, 64 * 257));
        // Zeroed motors are a stop command; other reports carry no motors.
        let stop = super::rumble_command_from_output_report(&[0x31, 0x00, 0, 0]).expect("stop");
        assert_eq!((stop.strong, stop.weak), (0, 0));
        assert!(super::rumble_command_from_output_report(&[0x01, 0, 0, 0]).is_none());
        assert!(super::rumble_command_from_output_report(&[0x31, 0x00]).is_none());
    }
}
