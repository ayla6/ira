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
use crate::rumble::RumbleCommand;
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
    /// Rumble the game played on this pad, decoded from output reports and
    /// drained by the daemon for replay on the physical controller.
    pending_rumble: Vec<RumbleCommand>,
    /// The last report sent, minus the always-different timer byte: a tick
    /// whose state is identical to the last one skips the wire write. Pads
    /// stream at up to 1000 Hz and idle state resends are pure waste.
    last_report: Option<[u8; 49]>,
}

impl SwitchProUhidDevice {
    /// The uniq joins the pad to its paired IMU for SDL's evdev sensor
    /// pairing; hid-nintendo's own IMU node cannot carry a usable serial.
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
            timer: 0,
            pending_rumble: Vec::new(),
            last_report: None,
        })
    }

    /// Answers kernel requests (the hid-nintendo handshake) and streams one
    /// 0x30 standard report. Call per tick; motion arrives in the SDL
    /// frame and leaves in the Nintendo device frame.
    pub fn tick(
        &mut self,
        pad: &PadState,
        accel_g: [f32; 3],
        gyro_dps: [f32; 3],
    ) -> io::Result<()> {
        // Consumers of Nintendo devices apply SDL's Nintendo axis shuffle
        // (sdl = [-dev_y, dev_z, -dev_x]); feeding SDL-frame data through
        // a Nintendo device would be shuffled twice. Pre-shuffle into the
        // device frame so the consumer's shuffle lands on the SDL frame.
        let accel_g = nintendo_device_frame(accel_g);
        let gyro_dps = nintendo_device_frame(gyro_dps);
        self.service()?;
        let report = standard_report(self.timer, pad, accel_g, gyro_dps);
        self.timer = self.timer.wrapping_add(1);
        // The timer byte (offset 1) differs on every tick; everything else
        // equal means nothing changed on the pad or the sensors.
        let unchanged = self
            .last_report
            .is_some_and(|last| {
                last[0] == report[0]
                    && last[2..] == report[2..]
            });
        if unchanged {
            return Ok(());
        }
        self.last_report = Some(report.clone().try_into().expect("49 bytes"));
        self.device.send_input_report(&report)
    }

    /// Drains kernel events and answers them without sending a state
    /// report. Must run every daemon pass regardless of pause or tick
    /// cadence: hid-nintendo's connect handshake waits at most one or two
    /// scheduling periods per step, and an unanswered step fails the probe
    /// and takes force feedback down with it.
    pub fn service(&mut self) -> io::Result<()> {
        for event in self.device.poll()? {
            match event {
                UhidEvent::OutputReport { data } => {
                    if let Some(command) = rumble_command_from_output_report(&data) {
                        self.pending_rumble.push(command);
                    }
                    let reply = handshake_reply(&data);
                    if !reply.is_empty() {
                        self.device.send_input_report(&reply)?;
                    }
                }
                UhidEvent::GetReport { id, .. } => {
                    let _ = self.device.reply_get_report(id, 61, &[]);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Rumble commands decoded since the last drain, in arrival order.
    /// Zero-magnitude commands are real: the kernel stops motors by sending
    /// zeroed rumble packets, and replaying that on the physical pad stops
    /// it in turn.
    pub fn take_rumble(&mut self) -> Vec<RumbleCommand> {
        std::mem::take(&mut self.pending_rumble)
    }
}

/// Nominal duration for decoded rumble: the wire format carries amplitudes
/// only, and hid-nintendo signals the end with explicit zeroed packets, so
/// the replayed effect just needs to outlive the gaps between packets.
const DECODED_RUMBLE_MS: u16 = 250;

/// Largest amplitude code the firmware tables define (`0xC8`), the scale the
/// kernel maps the 0..=65535 evdev magnitude onto.
const MAX_AMPLITUDE_CODE: u16 = 200;

/// Extracts the rumble request from a kernel output report (subcommand
/// report 0x01 and rumble-only 0x10 both carry it: id, packet counter, then
/// two 4-byte motor commands — left/strong first, right/weak second). Each
/// command packs its amplitude code in the second byte's upper 7 bits
/// (`freq_low_byte + amp`, the freq byte being 0 or 1); the kernel scales
/// evdev's 0..=65535 magnitude onto 0..=200. Reports without rumble data
/// return `None`; zero amplitude still yields a zero command, which is the
/// wire's way of saying "stop the motors".
pub(crate) fn rumble_command_from_output_report(report: &[u8]) -> Option<RumbleCommand> {
    match report.first()? {
        0x01 | 0x10 if report.len() >= 10 => {}
        _ => return None,
    }
    Some(RumbleCommand {
        strong: decoded_amplitude(report[3]),
        weak: decoded_amplitude(report[7]),
        duration_ms: DECODED_RUMBLE_MS,
    })
}

/// Amplitude code (even byte) back to the evdev magnitude the kernel
/// received: `code / 200 * 65535`.
fn decoded_amplitude(byte: u8) -> u16 {
    let code = u16::from(byte & 0xFE);
    ((code as u32) * 65_535 / (MAX_AMPLITUDE_CODE as u32)) as u16
}

/// SDL reshapes Nintendo sensor axes as `sdl = [-dev_y, dev_z, -dev_x]`;
/// the inverse mapping turns SDL-frame data into device-frame values.
fn nintendo_device_frame(values: [f32; 3]) -> [f32; 3] {
    [-values[2], -values[0], values[1]]
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
                0,
                0,
                0, // buttons
                0x00,
                0x08,
                0x80, // left stick centered
                0x00,
                0x08,
                0x80,          // right stick centered
                0,             // vibrator
                0x80 | subcmd, // ack
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
    /// Factory stick calibration spans 0x603D..0x604E: the kernel reads each
    /// stick's 9 bytes separately, while SDL's hidapi driver reads both
    /// sticks in a single 18-byte request here, so the region serves one
    /// flat left+right image.
    const STICK_CAL_START: u32 = 0x603d;
    const STICK_CAL_LEN: usize = 18;

    let mut data = vec![0xFFu8; size];
    if addr == 0x6020 {
        // IMU calibration: zero offsets, 16384 scales -> divisor equals
        // scale, so the driver's conversion collapses to identity.
        if size >= 24 {
            data = vec![0u8; size];
            for axis in 0..3 {
                data[6 + axis * 2..8 + axis * 2].copy_from_slice(&16384u16.to_le_bytes());
                data[18 + axis * 2..20 + axis * 2].copy_from_slice(&16384u16.to_le_bytes());
            }
        }
        return data;
    }
    if (STICK_CAL_START..STICK_CAL_START + STICK_CAL_LEN as u32).contains(&addr) {
        let blob: Vec<u8> = stick_calibration(true)
            .into_iter()
            .chain(stick_calibration(false))
            .collect();
        let start = (addr - STICK_CAL_START) as usize;
        let end = (start + size).min(STICK_CAL_LEN);
        data[..end - start].copy_from_slice(&blob[start..end]);
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

/// Writes a 12-bit field LSB-first, the bit order hid-nintendo's
/// `hid_field_extract` uses for both report sticks and calibration.
fn pack_field(buf: &mut [u8], byte: usize, shift: u16, value: u16) {
    for (i, bit) in (0..12).zip(byte * 8 + shift as usize..) {
        if (value >> i) & 1 == 1 {
            buf[bit / 8] |= 1 << (bit % 8);
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

/// Two 12-bit stick axes packed LSB-first across three bytes, matching
/// hid-nintendo's `hid_field_extract(.., 0, 12)` / `(..+1, 4, 12)` decode.
/// Y encodes inverted because the driver negates it after mapping
/// (`y = -joycon_map_stick_val(..)`), so straight-through PlayStation-style
/// down-positive values would surface flipped sticks.
fn stick_bytes(x: f32, y: f32) -> [u8; 3] {
    let x = (STICK_CENTER as f32 + x * STICK_RANGE as f32)
        .round()
        .clamp(0.0, 4095.0) as u16;
    let y = (STICK_CENTER as f32 - y * STICK_RANGE as f32)
        .round()
        .clamp(0.0, 4095.0) as u16;
    [
        (x & 0xFF) as u8,
        (((y & 0xF) << 4) | (x >> 8)) as u8,
        (y >> 4) as u8,
    ]
}

/// Decodes two 12-bit fields packed LSB-first across three bytes; mirrors
/// the kernel's extraction so tests can assert what the driver will see.
#[cfg(test)]
fn unpack_fields(bytes: [u8; 3]) -> (u16, u16) {
    let first = u16::from(bytes[0]) | (u16::from(bytes[1] & 0x0F) << 8);
    let second = u16::from(bytes[1] >> 4) | (u16::from(bytes[2]) << 4);
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::{
        handshake_reply, spi_flash, standard_report, stick_bytes, unpack_fields, PadState,
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
        // SDL's hidapi reads both sticks in one 18-byte request at 0x603D;
        // its second half must equal the separate right-stick read.
        let combined = spi_flash(0x603d, 18);
        assert_eq!(combined.len(), 18);
        assert_eq!(combined[9..], spi_flash(0x6046, 9)[..]);
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
    fn test_device_frame_round_trips_sdl_shuffle() {
        let device = super::nintendo_device_frame([1.0, 2.0, 3.0]);
        // SDL's Nintendo shuffle: [-dev_y, dev_z, -dev_x]
        let sdl = [-device[1], device[2], -device[0]];
        assert_eq!(sdl, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_rumble_decoded_from_rumble_only_report() {
        // Kernel encoding for strong=65535 (left, code 200), weak=65535/2
        // (right, code 100): id, counter, then two 4-byte halves.
        let mut report = vec![0x10, 0x05, 0x20, 0xC8, 0x00, 0x40, 0x20, 0x64, 0x00, 0x40];
        let command = super::rumble_command_from_output_report(&report).expect("rumble report");
        assert_eq!(command.strong, 65_535);
        assert_eq!(command.weak, 32_767);
        assert!(command.duration_ms > 0);

        // Everything zeroed is a stop command, not the absence of one.
        report[3] = 0x00;
        report[7] = 0x00;
        let stop = super::rumble_command_from_output_report(&report).expect("stop report");
        assert_eq!((stop.strong, stop.weak), (0, 0));
    }

    #[test]
    fn test_rumble_ignores_non_rumble_reports() {
        // USB handshake and short/unknown reports carry no motor data.
        assert!(super::rumble_command_from_output_report(&[0x80, 0x02]).is_none());
        assert!(super::rumble_command_from_output_report(&[0x01, 0x00]).is_none());
        assert!(super::rumble_command_from_output_report(&[0x21, 0x00]).is_none());
    }

    #[test]
    fn test_stick_bytes_center_and_extremes() {
        assert_eq!(unpack_fields(stick_bytes(0.0, 0.0)), (2048, 2048));
        assert_eq!(unpack_fields(stick_bytes(1.0, 0.0)), (2048 + 1500, 2048));
        // Down-positive pad values encode below center: the driver negates
        // the mapped Y, so below-center raws surface as positive ABS_Y.
        assert_eq!(unpack_fields(stick_bytes(0.0, 1.0)), (2048, 2048 - 1500));
        assert_eq!(unpack_fields(stick_bytes(0.0, -1.0)), (2048, 2048 + 1500));
    }

    #[test]
    fn test_stick_calibration_decodes_as_driver_will_read_it() {
        for (addr, left) in [(0x603d, true), (0x6046, false)] {
            let raw = spi_flash(addr, 9);
            let extract = |byte: usize| -> u16 {
                u16::from(raw[byte]) | (u16::from(raw[byte + 1] & 0x0F) << 8)
            };
            let extract_shifted = |byte: usize| -> u16 {
                u16::from(raw[byte] >> 4) | (u16::from(raw[byte + 1]) << 4)
            };
            let (first, second, third) = (extract(0), extract_shifted(1), extract(3));
            let fourth = extract_shifted(4);
            let (fifth, sixth) = (extract(6), extract_shifted(7));
            // Offsets from center for the extremes, absolute for the center.
            let expected: [(u16, u16, u16, u16, u16, u16); 2] = [
                (1500, 1500, 2048, 2048, 1500, 1500), // left: max,max,cen,cen,min,min
                (2048, 2048, 1500, 1500, 1500, 1500), // right: cen,cen,min,min,max,max
            ];
            let e = expected[if left { 0 } else { 1 }];
            assert_eq!(
                (first, second, third, fourth, fifth, sixth),
                e,
                "addr {addr:#x}"
            );
        }
    }
}
