//! A virtual *real* Nintendo Switch Pro Controller built on
//! [`super::uhid`]: unlike every other flavor, this one is claimed by the
//! kernel's own `hid-nintendo` driver, so games and flatpaks see exactly
//! the hardware a real pad produces — including the driver-generated IMU
//! input node SDL pairs by serial.
//!
//! hid-nintendo talks to the device instead of parsing a descriptor: on
//! connect it handshakes over USB ([`0x80, cmd`] answered by
//! [`0x81, cmd`]), then issues subcommands (report 0x01) it expects
//! answered by 0x21 replies — device info, SPI flash reads for stick and
//! IMU calibration, report mode, IMU enable. This module answers all of
//! them and then streams 0x30 standard reports.
//!
//! The crafted calibration makes the driver's math an identity: every
//! cal scale equals its divisor, so accelerometer samples arrive as
//! g × 4096 and gyroscope samples as deg/s × 14247/1000 — the exact
//! units behind the resolutions the driver publishes
//! (`JC_IMU_ACCEL_RES_PER_G`, `JC_IMU_GYRO_RES_PER_DPS`).

use std::io;

use crate::motion_udp::PadState;
use crate::uhid::{UhidDevice, UhidEvent, BUS_USB};

pub const VENDOR_ID: u32 = 0x057e;
pub const PRODUCT_ID: u32 = 0x2009;
pub const DEVICE_NAME: &str = "Ira Virtual Switch Pro Controller";

/// Switch Pro buttons, positional (A/B are Nintendo labels).
const BTN_WEST_Y: u32 = 1 << 0;
const BTN_NORTH_X: u32 = 1 << 1;
const BTN_SOUTH_B: u32 = 1 << 2;
const BTN_EAST_A: u32 = 1 << 3;
const BTN_R: u32 = 1 << 6;
const BTN_ZR: u32 = 1 << 7;
const BTN_MINUS: u32 = 1 << 8;
const BTN_PLUS: u32 = 1 << 9;
const BTN_RSTICK: u32 = 1 << 10;
const BTN_LSTICK: u32 = 1 << 11;
const BTN_HOME: u32 = 1 << 12;
const BTN_DOWN: u32 = 1 << 16;
const BTN_UP: u32 = 1 << 17;
const BTN_RIGHT: u32 = 1 << 18;
const BTN_LEFT: u32 = 1 << 19;
const BTN_L: u32 = 1 << 22;
const BTN_ZL: u32 = 1 << 23;

const STICK_CENTER: u16 = 2048;
const STICK_RANGE: u16 = 1500;
/// A battery/connection byte reading "full, wired".
const BAT_FULL_USB: u8 = 0x8E;

/// Anything hid parse accepts; hid-nintendo consumes raw reports only.
const REPORT_DESCRIPTOR: &[u8] = &[
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

pub struct SwitchProUhidDevice {
    device: UhidDevice,
    timer: u8,
}

impl SwitchProUhidDevice {
    pub fn create() -> io::Result<Self> {
        let device = UhidDevice::create(
            DEVICE_NAME,
            "",
            REPORT_DESCRIPTOR,
            BUS_USB,
            VENDOR_ID,
            PRODUCT_ID,
        )?;
        Ok(Self {
            device,
            timer: 0,
        })
    }

    /// Answers kernel requests (the hid-nintendo handshake) and streams one
    /// 0x30 standard report. Call per tick; motion uses the SDL frame.
    pub fn tick(
        &mut self,
        pad: &PadState,
        accel_g: [f32; 3],
        gyro_dps: [f32; 3],
    ) -> io::Result<()> {
        for event in self.device.poll()? {
            match event {
                UhidEvent::OutputReport { data } => {
                    eprintln!("ira-input: switch pro request {data:?}");
                    let reply = handshake_reply(&data);
                    if !reply.is_empty() {
                        self.device.send_input_report(&reply)?;
                    }
                }
                UhidEvent::GetReport { id, .. } => {
                    let _ = self.device.reply_get_report(id, 61, &[]);
                }
                UhidEvent::Start => {
                    eprintln!("ira-input: virtual Switch Pro started by the kernel");
                }
                UhidEvent::Stop => {
                    eprintln!("ira-input: virtual Switch Pro stopped by the kernel");
                }
                other => {
                    eprintln!("ira-input: virtual Switch Pro event {other:?}");
                }
            }
        }
        let report = standard_report(self.timer, pad, accel_g, gyro_dps);
        self.timer = self.timer.wrapping_add(1);
        self.device.send_input_report(&report)
    }
}

/// Builds the input report answering one kernel output report: USB
/// commands get [`0x81, cmd`]; subcommands get a 0x21 ack carrying data
/// for device info and SPI flash reads. Empty for reports we ignore.
pub fn handshake_reply(request: &[u8]) -> Vec<u8> {
    match request.first() {
        Some(&0x80) => vec![0x81, *request.get(1).unwrap_or(&0)],
        Some(&0x01) => {
            let subcmd = request.get(10).copied().unwrap_or(0);
            let payload = subcmd_payload(subcmd, request.get(11..).unwrap_or(&[]));
            let mut reply = vec![
                0x21, // subcommand reply
                0x00, // timer (patched by caller? no: kernel ignores)
                BAT_FULL_USB,
                0, 0, 0,                // buttons
                0x00, 0x08, 0x80,       // left stick centered
                0x00, 0x08, 0x80,       // right stick centered
                0,                      // vibrator
                0x80 | subcmd,          // ack
                subcmd,
            ];
            reply.extend_from_slice(&payload);
            // The kernel drops replies smaller than a full input report.
            reply.resize(49, 0);
            reply
        }
        _ => Vec::new(),
    }
}

fn subcmd_payload(subcmd: u8, args: &[u8]) -> Vec<u8> {
    match subcmd {
        // REQ_DEV_INFO: firmware pair, controller type 0x03 (Pro), filler,
        // then the six MAC bytes the driver prints and uses as identity.
        0x02 => vec![0x03, 0x00, 0x03, 0x02, 0x49, 0x52, 0x41, 0x56, 0x50, 0x52],
        // SPI_FLASH_READ: echo address+size, then the flash bytes.
        0x10 if args.len() >= 5 => {
            let addr = u32::from_le_bytes([args[0], args[1], args[2], args[3]]);
            let size = args[4] as usize;
            let mut payload = args[..5].to_vec();
            payload.extend_from_slice(&spi_flash(addr, size.min(64)));
            payload
        }
        _ => Vec::new(),
    }
}

/// The fake SPI flash: user-calibration magics read as absent (0xFF) so the
/// driver takes the factory tables; those carry identity calibration and
/// centered sticks.
fn spi_flash(addr: u32, size: usize) -> Vec<u8> {
    let mut data = vec![0xFFu8; size];
    match addr {
        0x6020 => {
            // IMU calibration: zero offsets, 16384 scales -> divisor equals
            // scale, so the driver's conversion collapses to identity.
            if size >= 24 {
                data = vec![0u8; size];
                for axis in 0..3 {
                    data[6 + axis * 2..8 + axis * 2]
                        .copy_from_slice(&16384u16.to_le_bytes());
                    data[18 + axis * 2..20 + axis * 2]
                        .copy_from_slice(&16384u16.to_le_bytes());
                }
            }
        }
        0x603d => data = stick_calibration(true),
        0x6046 => data = stick_calibration(false),
        _ => {}
    }
    data
}

/// Stick factory calibration: centered sticks with a ±1500 range, packed
/// as six 12-bit fields MSB-first. Field order differs per stick to match
/// the driver's parser.
fn stick_calibration(left: bool) -> Vec<u8> {
    let mut raw = [0u8; 9];
    let order: [(usize, u16, u16); 6] = if left {
        // byte, bit shift, value: x_max, y_max, x_center, y_center,
        // x_min, y_min (max above center, min below center).
        [
            (0, 0, STICK_RANGE),
            (1, 4, STICK_RANGE),
            (3, 0, STICK_CENTER),
            (4, 4, STICK_CENTER),
            (6, 0, STICK_RANGE),
            (7, 4, STICK_RANGE),
        ]
    } else {
        [
            (0, 0, STICK_CENTER),
            (1, 4, STICK_CENTER),
            (3, 0, STICK_RANGE),
            (4, 4, STICK_RANGE),
            (6, 0, STICK_RANGE),
            (7, 4, STICK_RANGE),
        ]
    };
    for (byte, shift, value) in order {
        pack_field(&mut raw, byte, shift, value);
    }
    raw.to_vec()
}

/// Writes a 12-bit field MSB-first (HID bit order).
fn pack_field(buf: &mut [u8], byte: usize, shift: u16, value: u16) {
    for (i, bit) in (0..12).rev().zip(byte * 8 + shift as usize..) {
        if (value >> i) & 1 == 1 {
            buf[bit / 8] |= 1 << (7 - bit % 8);
        }
    }
}

/// One 0x30 standard report: buttons, sticks, three IMU samples.
pub fn standard_report(
    timer: u8,
    pad: &PadState,
    accel_g: [f32; 3],
    gyro_dps: [f32; 3],
) -> Vec<u8> {
    let mut buttons = 0u32;
    if pad.cross {
        buttons |= BTN_SOUTH_B;
    }
    if pad.circle {
        buttons |= BTN_EAST_A;
    }
    if pad.square {
        buttons |= BTN_WEST_Y;
    }
    if pad.triangle {
        buttons |= BTN_NORTH_X;
    }
    if pad.l1 {
        buttons |= BTN_L;
    }
    if pad.r1 {
        buttons |= BTN_R;
    }
    if pad.back {
        buttons |= BTN_MINUS;
    }
    if pad.start {
        buttons |= BTN_PLUS;
    }
    if pad.l3 {
        buttons |= BTN_LSTICK;
    }
    if pad.r3 {
        buttons |= BTN_RSTICK;
    }
    if pad.guide {
        buttons |= BTN_HOME;
    }
    if pad.dpad_up {
        buttons |= BTN_UP;
    }
    if pad.dpad_down {
        buttons |= BTN_DOWN;
    }
    if pad.dpad_left {
        buttons |= BTN_LEFT;
    }
    if pad.dpad_right {
        buttons |= BTN_RIGHT;
    }
    if pad.l2 >= 0.5 {
        buttons |= BTN_ZL;
    }
    if pad.r2 >= 0.5 {
        buttons |= BTN_ZR;
    }

    let mut report = Vec::with_capacity(49);
    report.push(0x30);
    report.push(timer);
    report.push(BAT_FULL_USB);
    report.extend_from_slice(&buttons.to_le_bytes()[..3]);
    report.extend_from_slice(&stick_bytes(pad.lx, pad.ly));
    report.extend_from_slice(&stick_bytes(pad.rx, pad.ry));
    report.push(0); // vibrator
    for _ in 0..3 {
        for value in accel_g {
            let raw = (value * 4096.0).round().clamp(-32768.0, 32767.0) as i16;
            report.extend_from_slice(&raw.to_le_bytes());
        }
        for value in gyro_dps {
            let raw = (value * 14.247).round().clamp(-32768.0, 32767.0) as i16;
            report.extend_from_slice(&raw.to_le_bytes());
        }
    }
    report
}

/// Two 12-bit stick axes in three bytes, LSB-first as the driver decodes
/// report data.
fn stick_bytes(x: f32, y: f32) -> [u8; 3] {
    let x = (STICK_CENTER as f32 + x * STICK_RANGE as f32).round().clamp(0.0, 4095.0) as u16;
    let y = (STICK_CENTER as f32 + y * STICK_RANGE as f32).round().clamp(0.0, 4095.0) as u16;
    [
        (x & 0xFF) as u8,
        (((y & 0xF) << 4) | (x >> 8)) as u8,
        (y >> 4) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        handshake_reply, spi_flash, standard_report, stick_bytes, PadState,
    };

    #[test]
    fn test_usb_commands_ack_with_matching_reply() {
        assert_eq!(handshake_reply(&[0x80, 0x02]), vec![0x81, 0x02]);
        assert_eq!(handshake_reply(&[0x80, 0x03]), vec![0x81, 0x03]);
        assert!(handshake_reply(&[0x10]).is_empty());
    }

    #[test]
    fn test_device_info_reply_carries_pro_type_and_mac() {
        let mut request = vec![0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02];
        request.extend_from_slice(&[0; 0]);
        let reply = handshake_reply(&request);
        assert_eq!(reply[0], 0x21);
        assert_eq!(reply[13], 0x82); // ack 0x80 | 0x02
        assert_eq!(reply[14], 0x02); // subcmd id
        assert_eq!(reply[17], 0x03); // controller type: Pro
        assert_eq!(&reply[19..25], b"IRAVPR"); // MAC
    }

    #[test]
    fn test_spi_replies_serve_calibration_tables() {
        // User-cal magic addresses read as absent.
        assert!(spi_flash(0x8026, 2).iter().all(|&b| b == 0xFF));
        // IMU calibration: zero offsets, 16384 scales, identity divisors.
        let imu = spi_flash(0x6020, 24);
        assert_eq!(imu.len(), 24);
        assert_eq!(u16::from_le_bytes([imu[6], imu[7]]), 16384);
        assert_eq!(u16::from_le_bytes([imu[18], imu[19]]), 16384);
        // Stick calibration blocks exist for both sticks.
        assert_eq!(spi_flash(0x603d, 9).len(), 9);
        assert_eq!(spi_flash(0x6046, 9).len(), 9);
    }

    #[test]
    fn test_standard_report_layout_and_button_bits() {
        let pad = PadState {
            cross: true,
            l2: 1.0,
            ..PadState::default()
        };
        let report = standard_report(7, &pad, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]);
        assert_eq!(report[0], 0x30);
        assert_eq!(report[1], 7);
        assert_eq!(report[3], 1 << 2); // south (B) in the first button byte
        assert_eq!(report[5], 1 << 7); // ZL in the third button byte
        // Third IMU sample's accel Z lands as 1g x 4096.
        let offset = 13 + 2 * 12 + 4; // skip 2 samples, accel z slot
        let raw = i16::from_le_bytes([report[offset], report[offset + 1]]);
        assert_eq!(raw, 4096);
    }

    #[test]
    fn test_stick_bytes_center_and_extremes() {
        assert_eq!(stick_bytes(0.0, 0.0), [0x00, 0x08, 0x80]);
        let right = stick_bytes(1.0, 0.0);
        let x = u16::from(right[0]) | (u16::from(right[1] & 0x0F) << 8);
        assert_eq!(x, 2048 + 1500);
        let down = stick_bytes(0.0, 1.0);
        let y = u16::from(down[1] >> 4) | (u16::from(down[2]) << 4);
        assert_eq!(y, 2048 + 1500);
    }
}
