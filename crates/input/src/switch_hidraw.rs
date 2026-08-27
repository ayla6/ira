//! Talks the Nintendo Switch controller protocol straight to a physical
//! pad's hidraw node — the same approach SDL's hidapi driver takes, and
//! the intended motion source for Switch-mode pads the kernel's
//! `hid-nintendo` driver has not claimed: 8BitDo dongles in Switch mode
//! keep their own vendor id, so generic HID maps their buttons but
//! nothing ever parses the IMU.
//!
//! KNOWN LIMITATION (as of 2026-08): gyro on the 8BitDo Ultimate 2
//! dongle's Switch mode does not work yet. Three approaches were built
//! and verified against a real kernel; none delivered motion on the real
//! dongle:
//! 1. The kernel's IMU companion node — never exists, because
//!    hid-nintendo does not bind this dongle's Switch-mode id.
//! 2. In-process SDL3 — SDL has no driver for the dongle's Switch-mode
//!    id either (its 8BitDo driver only knows the DInput products).
//! 3. This driver — the handshake path works end to end against a
//!    virtual pad (see the switch_hidraw_driver probe test), but the
//!    physical dongle either stops answering after the handshake or
//!    never streams 0x30 reports to hidraw, so the fail-safes below
//!    hand the pad back to evdev (buttons keep working, motion does
//!    not). The startup log says which happened: "Switch protocol
//!    engaged" means this driver owns the pad, "streams no reports"
//!    means the gate rejected it. A usbmon capture of Steam reading
//!    motion from this exact dongle would close the gap — Steam does
//!    get gyro from it, so some incantation we are not sending
//!    (ForceUSB? a different report mode?) starts the stream.
//!
//! DInput mode has full gyro through SDL3; use that when motion matters.
//!
//! On open the driver handshakes like `joycon_init` does (USB handshake,
//! baudrate, device-info subcommand), then switches the pad to 0x30
//! standard reports with the IMU enabled. A 600ms first-report gate and
//! a daemon-side silence watchdog keep a non-streaming pad on the evdev
//! path instead of taking input away. From then on every input report
//! yields buttons, sticks and three motion samples; the newest sample is
//! exposed per service pass. Rumble replays through the 0x10 report
//! built by [`crate::switch_rumble`].
//!
//! Taking over the report mode can invalidate what generic HID parses from
//! the pad's descriptor, so while this driver is active the daemon must
//! feed the mapping engine from [`SwitchHidrawPad::take_events`] instead
//! of the pad's evdev node.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use crate::physical::swap_face_buttons;
use crate::rumble::{sibling_hidraw_nodes, RumbleCommand};
use crate::switch_rumble::rumble_output_report;
use crate::{DeviceInfo, GamepadAxis, GamepadButton, InputEvent, InputSource, SensorSample};

/// Standard input report id: buttons, sticks and three IMU samples.
const REPORT_ID_INPUT: u8 = 0x30;
/// Subcommand reply report id (acks during init).
const REPORT_ID_SUBCMD_REPLY: u8 = 0x21;
/// Subcommand output report id.
const REPORT_ID_SUBCOMMAND: u8 = 0x01;
const SUBCMD_DEV_INFO: u8 = 0x02;
const SUBCMD_SET_REPORT_MODE: u8 = 0x03;
const SUBCMD_ENABLE_IMU: u8 = 0x40;
const SUBCMD_ENABLE_VIBRATION: u8 = 0x48;

/// SDL's uncalibrated IMU scales: accelerometer counts per g, gyroscope
/// counts per degree per second.
const ACCEL_COUNTS_PER_G: f32 = 4096.0;
const GYRO_COUNTS_PER_DPS: f32 = 14.2842;
const GRAVITY_MS2: f32 = 9.80665;

/// Button bits of the 0x30 report's three button bytes, by position.
const BTN_WEST: u32 = 1 << 0; // Y label
const BTN_NORTH: u32 = 1 << 1; // X label
const BTN_SOUTH: u32 = 1 << 2; // B label
const BTN_EAST: u32 = 1 << 3; // A label
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

/// (bit, positional button) pairs; capture is skipped deliberately, as on
/// the hid-nintendo path.
const BUTTON_MAP: [(u32, GamepadButton); 17] = [
    (BTN_WEST, GamepadButton::X),
    (BTN_NORTH, GamepadButton::Y),
    (BTN_SOUTH, GamepadButton::A),
    (BTN_EAST, GamepadButton::B),
    (BTN_R, GamepadButton::RightShoulder),
    (BTN_ZR, GamepadButton::RightTrigger),
    (BTN_MINUS, GamepadButton::Back),
    (BTN_PLUS, GamepadButton::Start),
    (BTN_RSTICK, GamepadButton::RightStick),
    (BTN_LSTICK, GamepadButton::LeftStick),
    (BTN_HOME, GamepadButton::Guide),
    (BTN_DOWN, GamepadButton::DpadDown),
    (BTN_UP, GamepadButton::DpadUp),
    (BTN_RIGHT, GamepadButton::DpadRight),
    (BTN_LEFT, GamepadButton::DpadLeft),
    (BTN_L, GamepadButton::LeftShoulder),
    (BTN_ZL, GamepadButton::LeftTrigger),
];

/// One live Switch-protocol pad driven directly over hidraw.
pub struct SwitchHidrawPad {
    file: File,
    out_timer: u8,
    nintendo_layout: bool,
    prev_buttons: u32,
    prev_axes: [f32; 4],
    accel_ms2: [f32; 3],
    gyro_rad: [f32; 3],
    fresh_motion: bool,
    pending_events: Vec<InputEvent>,
    rumble_magnitudes: (u16, u16),
    rumble_stop_at: Option<Instant>,
    rumble_last_sent: Option<Instant>,
    /// When the last 0x30 report arrived; Switch pads stream continuously,
    /// so a stale value means the takeover must be abandoned.
    last_report: Instant,
}

impl SwitchHidrawPad {
    /// Probes every hidraw node beside the pad's event node for a
    /// Switch-speaking controller and switches it to full 0x30 mode.
    /// `None` when none answers the handshake (DInput mode, non-Switch
    /// pads, or a hidraw the user cannot open) — or when a pad answers
    /// but then pushes no reports, which would leave the session with no
    /// input source at all; the simple report mode is restored and the
    /// evdev path keeps the pad.
    pub fn open(pad: &DeviceInfo) -> Option<Self> {
        for node in sibling_hidraw_nodes(&pad.path) {
            let node = host_visible(node);
            let Ok(mut file) = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&node)
            else {
                continue;
            };
            let mut out_timer = 0;
            if !init_switch(&mut file, &mut out_timer) {
                continue;
            }
            if !wait_for(&mut file, 600, |report| {
                report.first() == Some(&REPORT_ID_INPUT)
            }) {
                eprintln!(
                    "ira-input: '{}' answered the Switch handshake but streams no \
                     reports; restoring simple mode and keeping the evdev input path",
                    pad.name
                );
                let _ = send_subcommand(&mut file, &mut out_timer, SUBCMD_SET_REPORT_MODE, &[0x3F]);
                continue;
            }
            eprintln!(
                "ira-input: Switch protocol engaged on {} for '{}'",
                node.display(),
                pad.name
            );
            return Some(Self {
                file,
                out_timer,
                nintendo_layout: false,
                prev_buttons: 0,
                prev_axes: [0.0; 4],
                accel_ms2: [0.0; 3],
                gyro_rad: [0.0; 3],
                fresh_motion: false,
                pending_events: Vec::new(),
                rumble_magnitudes: (0, 0),
                rumble_stop_at: None,
                rumble_last_sent: None,
                last_report: Instant::now(),
            });
        }
        None
    }

    /// Controller-level Nintendo layout, mirroring the evdev path.
    pub fn set_nintendo_layout(&mut self, on: bool) {
        self.nintendo_layout = on;
    }

    /// Drains pending input reports and refreshes rumble. Run every daemon
    /// pass: the pad pushes reports on its own schedule.
    pub fn service(&mut self) {
        let mut buffer = [0u8; 64];
        for _ in 0..64 {
            match self.file.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => self.ingest_report(&buffer[..size]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("ira-input: Switch hidraw read failed: {error}");
                    break;
                }
            }
        }
        self.refresh_rumble();
    }

    /// How long since the last 0x30 report; Switch pads stream
    /// continuously, so a growing value means the pad went silent.
    pub fn silent_for(&self) -> Duration {
        self.last_report.elapsed()
    }

    /// Mapping-engine events decoded since the last drain.
    pub fn take_events(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// The newest motion sample since the last drain, if any.
    pub fn take_sample(&mut self, fallback_timestamp_us: u64) -> Option<SensorSample> {
        if !self.fresh_motion {
            return None;
        }
        self.fresh_motion = false;
        Some(SensorSample {
            gyro: self.gyro_rad,
            accel: Some(self.accel_ms2),
            timestamp_us: fallback_timestamp_us,
        })
    }

    /// Runs the motors; the wire has no duration, so a deadline stops them
    /// like the other self-timed backends.
    pub fn play_rumble(&mut self, command: RumbleCommand) {
        let duration = command.duration_ms.clamp(20, 5_000);
        self.rumble_magnitudes = (command.strong, command.weak);
        self.rumble_stop_at = Some(Instant::now() + Duration::from_millis(u64::from(duration)));
        self.send_rumble();
    }

    /// Stops the motors immediately.
    pub fn stop_rumble(&mut self) {
        if self.rumble_stop_at.is_none() && self.rumble_magnitudes == (0, 0) {
            return;
        }
        self.rumble_magnitudes = (0, 0);
        self.rumble_stop_at = None;
        self.send_rumble();
    }

    fn send_rumble(&mut self) {
        let report = rumble_output_report(
            self.out_timer,
            self.rumble_magnitudes.0,
            self.rumble_magnitudes.1,
        );
        self.out_timer = self.out_timer.wrapping_add(1);
        if let Err(error) = self.file.write_all(&report) {
            eprintln!("ira-input: Switch rumble write failed: {error}");
        }
        self.rumble_last_sent = Some(Instant::now());
    }

    /// Long-running rumble needs periodic re-sends (SDL refreshes every
    /// 50ms); expired rumble needs one zeroed report.
    fn refresh_rumble(&mut self) {
        let Some(deadline) = self.rumble_stop_at else {
            return;
        };
        if Instant::now() >= deadline {
            self.stop_rumble();
        } else if self
            .rumble_last_sent
            .is_none_or(|sent| sent.elapsed() >= Duration::from_millis(40))
        {
            self.send_rumble();
        }
    }

    fn ingest_report(&mut self, report: &[u8]) {
        if report.first() != Some(&REPORT_ID_INPUT) || report.len() < 49 {
            return;
        }
        self.last_report = Instant::now();
        let timestamp_us = now_us();
        let buttons = decoded_buttons(report);
        if buttons != self.prev_buttons {
            for (bit, button) in BUTTON_MAP {
                let pressed = buttons & bit != 0;
                if (self.prev_buttons & bit != 0) == pressed {
                    continue;
                }
                let button = if self.nintendo_layout {
                    swap_face_buttons(button)
                } else {
                    button
                };
                self.pending_events.push(InputEvent {
                    source: InputSource::Button(button),
                    value: f32::from(pressed),
                    timestamp_us,
                });
            }
            self.prev_buttons = buttons;
        }
        let axes = [
            decoded_axis(GamepadAxis::LeftX, stick_raw(report, 6).0),
            decoded_axis(GamepadAxis::LeftY, stick_raw(report, 6).1),
            decoded_axis(GamepadAxis::RightX, stick_raw(report, 9).0),
            decoded_axis(GamepadAxis::RightY, stick_raw(report, 9).1),
        ];
        for (index, value) in axes.into_iter().enumerate() {
            if (value - self.prev_axes[index]).abs() > 0.001 {
                self.pending_events.push(InputEvent {
                    source: InputSource::Axis(match index {
                        0 => GamepadAxis::LeftX,
                        1 => GamepadAxis::LeftY,
                        2 => GamepadAxis::RightX,
                        _ => GamepadAxis::RightY,
                    }),
                    value,
                    timestamp_us,
                });
            }
        }
        self.prev_axes = axes;

        let (accel_ms2, gyro_rad) = motion_from_report(report);
        self.accel_ms2 = accel_ms2;
        self.gyro_rad = gyro_rad;
        self.fresh_motion = true;
    }
}

fn now_us() -> u64 {
    std::time::UNIX_EPOCH
        .elapsed()
        .map(|elapsed| elapsed.as_micros() as u64)
        .unwrap_or(0)
}

/// Sandboxed environments (distrobox) bind-mount /dev at container start,
/// so freshly created hidraw nodes exist only under the host's tree; prefer
/// it when the direct path is absent. On the host this never triggers.
fn host_visible(node: std::path::PathBuf) -> std::path::PathBuf {
    if node.exists() {
        return node;
    }
    let name = node.file_name().map(|name| name.to_owned());
    if let Some(name) = name {
        let candidate = std::path::Path::new("/run/host/dev").join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    node
}

/// The connect conversation, mirroring hid-nintendo's `joycon_init`:
/// USB handshake and baudrate first, then the device-info subcommand whose
/// 0x21 ack is the definitive proof the pad speaks this protocol, then the
/// mode/IMU/vibration subcommands that make reports worth reading.
fn init_switch(file: &mut File, out_timer: &mut u8) -> bool {
    if file.write_all(&[0x80, 0x02]).is_err() {
        return false;
    }
    if !wait_for(file, 300, |report| {
        report.len() >= 2 && report[0] == 0x81 && report[1] == 0x02
    }) {
        return false;
    }
    let _ = file.write_all(&[0x80, 0x03]);
    let _ = wait_for(file, 150, |report| {
        report.len() >= 2 && report[0] == 0x81 && report[1] == 0x03
    });
    if !send_subcommand(file, out_timer, SUBCMD_DEV_INFO, &[]) {
        return false;
    }
    if !wait_for(file, 300, |report| {
        report.len() >= 15
            && report[0] == REPORT_ID_SUBCMD_REPLY
            && report[13] == 0x80 | SUBCMD_DEV_INFO
            && report[14] == SUBCMD_DEV_INFO
    }) {
        return false;
    }
    for (subcmd, args) in [
        (SUBCMD_SET_REPORT_MODE, [REPORT_ID_INPUT].as_slice()),
        (SUBCMD_ENABLE_IMU, [0x01].as_slice()),
        (SUBCMD_ENABLE_VIBRATION, [0x01].as_slice()),
    ] {
        if !send_subcommand(file, out_timer, subcmd, args) {
            return false;
        }
        let _ = wait_for(file, 150, |report| {
            report.len() >= 15
                && report[0] == REPORT_ID_SUBCMD_REPLY
                && report[13] == 0x80 | subcmd
                && report[14] == subcmd
        });
    }
    true
}

/// Sends one subcommand report: id, timer, eight rumble placeholder bytes,
/// then the subcommand and its arguments — the layout the daemon's own
/// virtual Switch Pro decodes.
fn send_subcommand(file: &mut File, out_timer: &mut u8, subcmd: u8, args: &[u8]) -> bool {
    let mut packet = vec![REPORT_ID_SUBCOMMAND, *out_timer];
    packet.extend(std::iter::repeat_n(0u8, 8));
    packet.push(subcmd);
    packet.extend_from_slice(args);
    *out_timer = out_timer.wrapping_add(1);
    file.write_all(&packet).is_ok()
}

/// Reads reports until one matches, discarding everything else, bounded by
/// a poll timeout so a non-Switch device costs one short wait.
fn wait_for(file: &mut File, timeout_ms: i32, matches: impl Fn(&[u8]) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut buffer = [0u8; 64];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let mut descriptor = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                remaining.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if result <= 0 {
            return false;
        }
        match file.read(&mut buffer) {
            Ok(size) if matches(&buffer[..size]) => return true,
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
}

/// The 24 button bits across the report's three button bytes.
pub fn decoded_buttons(report: &[u8]) -> u32 {
    u32::from(report[3]) | u32::from(report[4]) << 8 | u32::from(report[5]) << 16
}

/// Unpacks one stick's two 12-bit fields from its three wire bytes.
pub fn stick_raw(report: &[u8], offset: usize) -> (u16, u16) {
    let x = u16::from(report[offset]) | (u16::from(report[offset + 1] & 0x0F) << 8);
    let y =
        (u16::from(report[offset + 1] >> 4) & 0x0F) | (u16::from(report[offset + 2]) << 4);
    (x, y)
}

/// 12-bit stick field to the engine's −1..=1 scale, centered at 2048. Y is
/// inverted: raw up is positive, the engine's down is positive — the same
/// negation hid-nintendo applies after mapping.
pub fn decoded_axis(axis: GamepadAxis, raw: u16) -> f32 {
    let value = (i32::from(raw) - 2048) as f32 / 2047.0;
    match axis {
        GamepadAxis::LeftY | GamepadAxis::RightY => -value,
        _ => value,
    }
    .clamp(-1.0, 1.0)
}

/// Motion from the newest of the report's three samples: accelerometer in
/// m/s² and gyroscope in rad/s, both already in the SDL sensor frame via
/// the Nintendo shuffle `[-y, z, -x]`.
pub fn motion_from_report(report: &[u8]) -> ([f32; 3], [f32; 3]) {
    let read_i16 = |offset: usize| {
        i16::from_le_bytes([report[offset], report[offset + 1]])
    };
    // Samples are oldest first; the newest is the last.
    let base = 13 + 2 * 12;
    let accel_g = [
        read_i16(base) as f32 / ACCEL_COUNTS_PER_G,
        read_i16(base + 2) as f32 / ACCEL_COUNTS_PER_G,
        read_i16(base + 4) as f32 / ACCEL_COUNTS_PER_G,
    ];
    let gyro_dps = [
        read_i16(base + 6) as f32 / GYRO_COUNTS_PER_DPS,
        read_i16(base + 8) as f32 / GYRO_COUNTS_PER_DPS,
        read_i16(base + 10) as f32 / GYRO_COUNTS_PER_DPS,
    ];
    const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
    (
        [
            -accel_g[1] * GRAVITY_MS2,
            accel_g[2] * GRAVITY_MS2,
            -accel_g[0] * GRAVITY_MS2,
        ],
        [
            -gyro_dps[1] * DEG_TO_RAD,
            gyro_dps[2] * DEG_TO_RAD,
            -gyro_dps[0] * DEG_TO_RAD,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{
        decoded_axis, decoded_buttons, motion_from_report, stick_raw, BTN_EAST, BTN_MINUS,
        BTN_SOUTH, BTN_WEST, BTN_ZL,
    };
    use crate::GamepadAxis;

    /// A resting 0x30 report.
    fn report() -> Vec<u8> {
        let mut report = vec![0u8; 49];
        report[0] = 0x30;
        report
    }

    #[test]
    fn test_buttons_decode_by_position() {
        let mut report = report();
        report[3] = (BTN_SOUTH | BTN_EAST) as u8;
        report[4] = (BTN_MINUS >> 8) as u8;
        report[5] = (BTN_ZL >> 16) as u8;
        let buttons = decoded_buttons(&report);
        assert_eq!(buttons & BTN_SOUTH, BTN_SOUTH);
        assert_eq!(buttons & BTN_EAST, BTN_EAST);
        assert_eq!(buttons & BTN_MINUS, BTN_MINUS);
        assert_eq!(buttons & BTN_ZL, BTN_ZL);
        assert_eq!(buttons & BTN_WEST, 0);
    }

    #[test]
    fn test_stick_unpacks_twelve_bit_fields() {
        let mut report = report();
        // x = 0xABC, y = 0x123 packed LSB-first.
        report[6] = 0xBC;
        report[7] = ((0xABC >> 8) as u8) | (((0x123 & 0x0F) << 4) as u8);
        report[8] = (0x123 >> 4) as u8;
        assert_eq!(stick_raw(&report, 6), (0x0ABC, 0x0123));
        // Right stick lives three bytes later.
        report[9] = 0x00;
        report[10] = 0x08;
        report[11] = 0x00;
        assert_eq!(stick_raw(&report, 9), (0x800, 0x000));
    }

    #[test]
    fn test_axis_center_extremes_and_y_inversion() {
        assert!((decoded_axis(GamepadAxis::LeftX, 2048) - 0.0).abs() < 1e-4);
        assert!((decoded_axis(GamepadAxis::LeftX, 4095) - 1.0).abs() < 1e-3);
        // Raw Y is up-positive; the engine's positive Y is down, so the
        // extremes flip — the same negation hid-nintendo applies.
        assert!((decoded_axis(GamepadAxis::LeftY, 4095) + 1.0).abs() < 1e-3);
        assert!((decoded_axis(GamepadAxis::LeftY, 0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn test_motion_shuffles_and_scales_like_sdl() {
        let mut report = report();
        // The newest sample is the last of the three (offset 13 + 24).
        let base = 13 + 24;
        report[base..base + 2].copy_from_slice(&4096i16.to_le_bytes()); // accel x = 1 g
        report[base + 2..base + 4].copy_from_slice(&(-8192i16).to_le_bytes()); // y = -2 g
        report[base + 4..base + 6].copy_from_slice(&16384i16.to_le_bytes()); // z = 4 g
        let gyro_x_raw = (14.2842_f32 * 90.0).round() as i16; // gyro x = 90 dps
        report[base + 6..base + 8].copy_from_slice(&gyro_x_raw.to_le_bytes());
        let (accel, gyro) = motion_from_report(&report);
        // SDL frame: [-y, z, -x].
        assert!((accel[0] - 2.0 * 9.80665).abs() < 0.02);
        assert!((accel[1] - 4.0 * 9.80665).abs() < 0.05);
        assert!((accel[2] + 1.0 * 9.80665).abs() < 0.02);
        assert!((gyro[2] + 90.0_f32.to_radians()).abs() < 0.01);
    }
}
