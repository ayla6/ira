//! Cemuhook / DSU motion server: presents a complete virtual controller —
//! buttons, analog sticks, triggers and the physical controller's raw
//! gyroscope/accelerometer — over UDP, so emulators with a cemuhook client
//! (Cemu, Ryujinx, Yuzu, Dolphin) can use Ira's pad as ONE device for input
//! and motion. Sandboxed emulators cannot read uinput sensors or hidraw,
//! which makes this the only path that carries everything.
//!
//! Protocol: https://v1993.github.io/cemuhook-protocol/ — "DSUS"/"DSUC"
//! magic, protocol version 1001, CRC32 over the whole packet with the
//! checksum field zeroed, accelerometer in m/s^2 (the unit Cemu's client
//! expects, matching what SDL provides), gyroscope in degrees/second.

use crate::{GamepadAxis, GamepadButton, OutputEvent};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const MOTION_PORT: u16 = 26760;
const PROTOCOL_VERSION: u16 = 1001;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

const MESSAGE_VERSION: u32 = 0x100000;
const MESSAGE_INFO: u32 = 0x100001;
const MESSAGE_DATA: u32 = 0x100002;

/// The DualShock-shaped button set the DSU protocol carries, tracked as a
/// shadow of the virtual gamepad's output so the stream always matches what
/// the mapped controller is doing regardless of backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadState {
    pub cross: bool,
    pub circle: bool,
    pub square: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub back: bool,
    pub start: bool,
    pub l3: bool,
    pub r3: bool,
    pub guide: bool,
    /// Analog triggers, 0..1.
    pub l2: f32,
    pub r2: f32,
    /// Analog sticks, -1..1 with +1 meaning right/down.
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
}

impl Default for PadState {
    fn default() -> Self {
        Self {
            cross: false,
            circle: false,
            square: false,
            triangle: false,
            l1: false,
            r1: false,
            dpad_up: false,
            dpad_down: false,
            dpad_left: false,
            dpad_right: false,
            back: false,
            start: false,
            l3: false,
            r3: false,
            guide: false,
            l2: 0.0,
            r2: 0.0,
            lx: 0.0,
            ly: 0.0,
            rx: 0.0,
            ry: 0.0,
        }
    }
}

impl PadState {
    /// Fold one virtual gamepad output event into the shadow state.
    /// Buttons use positional DualShock semantics: A/cross is south,
    /// B/circle east, X/square west, Y/triangle north.
    pub fn apply_output(&mut self, event: &OutputEvent) {
        match event {
            OutputEvent::GamepadButton { button, pressed } => {
                let pressed = *pressed;
                match button {
                    GamepadButton::A => self.cross = pressed,
                    GamepadButton::B => self.circle = pressed,
                    GamepadButton::X => self.square = pressed,
                    GamepadButton::Y => self.triangle = pressed,
                    GamepadButton::LeftShoulder => self.l1 = pressed,
                    GamepadButton::RightShoulder => self.r1 = pressed,
                    GamepadButton::DpadUp => self.dpad_up = pressed,
                    GamepadButton::DpadDown => self.dpad_down = pressed,
                    GamepadButton::DpadLeft => self.dpad_left = pressed,
                    GamepadButton::DpadRight => self.dpad_right = pressed,
                    GamepadButton::Back => self.back = pressed,
                    GamepadButton::Start => self.start = pressed,
                    GamepadButton::LeftStick => self.l3 = pressed,
                    GamepadButton::RightStick => self.r3 = pressed,
                    GamepadButton::Guide => self.guide = pressed,
                    _ => {}
                }
            }
            OutputEvent::GamepadAxis { axis, value } => match axis {
                GamepadAxis::LeftX => self.lx = value.clamp(-1.0, 1.0),
                GamepadAxis::LeftY => self.ly = value.clamp(-1.0, 1.0),
                GamepadAxis::RightX => self.rx = value.clamp(-1.0, 1.0),
                GamepadAxis::RightY => self.ry = value.clamp(-1.0, 1.0),
                GamepadAxis::LeftTrigger => self.l2 = value.clamp(0.0, 1.0),
                GamepadAxis::RightTrigger => self.r2 = value.clamp(0.0, 1.0),
            },
            _ => {}
        }
    }
}

/// One raw motion frame to broadcast, already in protocol units.
pub struct MotionSample {
    pub accel_ms2: [f32; 3],
    pub gyro_dps: [f32; 3],
    pub timestamp_us: u64,
}

pub struct MotionServer {
    socket: UdpSocket,
    clients: Vec<(SocketAddr, Instant)>,
    packet_counter: u32,
    server_id: u32,
}

impl MotionServer {
    /// Bind the default motion port. `None` when the port is taken (another
    /// motion server, e.g. an emulator's own helper, already listens).
    pub fn bind() -> Option<Self> {
        Self::bind_on(MOTION_PORT)
    }

    /// Bind a specific port so launches can move or disable the stream
    /// (`--motion-port 0` means off).
    pub fn bind_on(port: u16) -> Option<Self> {
        let socket = UdpSocket::bind(("127.0.0.1", port)).ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self {
            socket,
            clients: Vec::new(),
            packet_counter: 0,
            server_id: 0x1A1A_C0DE,
        })
    }

    /// Drain pending client requests, replying to version and information
    /// requests and (re)subscribing data requesters. Call every loop pass.
    pub fn poll_clients(&mut self, connected: bool) {
        let mut buf = [0u8; 64];
        while let Ok((read, from)) = self.socket.recv_from(&mut buf) {
            if read < 20 || &buf[0..4] != b"DSUC" {
                continue;
            }
            let message = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
            match message {
                MESSAGE_VERSION => {
                    let payload = PROTOCOL_VERSION.to_le_bytes();
                    let _ = self.socket.send_to(&build_packet(self.server_id, MESSAGE_VERSION, &payload), from);
                }
                MESSAGE_INFO => {
                    let payload = info_payload(connected);
                    let _ = self.socket.send_to(&build_packet(self.server_id, MESSAGE_INFO, &payload), from);
                }
                MESSAGE_DATA => self.subscribe(from),
                _ => {}
            }
        }
        self.clients
            .retain(|(_, seen)| seen.elapsed() < CLIENT_TIMEOUT);
    }

    /// Broadcast the latest sample — motion plus the controller shadow
    /// state — to every subscribed client.
    pub fn send_sample(&mut self, sample: &MotionSample, pad: &PadState) {
        if self.clients.is_empty() {
            return;
        }
        self.packet_counter = self.packet_counter.wrapping_add(1);
        let payload = data_payload(self.packet_counter, sample, pad);
        let packet = build_packet(self.server_id, MESSAGE_DATA, &payload);
        self.clients
            .retain(|(client, seen)| {
                if seen.elapsed() >= CLIENT_TIMEOUT {
                    return false;
                }
                self.socket.send_to(&packet, client).is_ok()
            });
    }

    fn subscribe(&mut self, client: SocketAddr) {
        match self.clients.iter_mut().find(|(known, _)| *known == client) {
            Some(entry) => entry.1 = Instant::now(),
            None => self.clients.push((client, Instant::now())),
        }
    }
}

/// Controller info block: slot 0, full-gyro model, Bluetooth connection,
/// healthy battery. Padded to 12 bytes as the information response payload.
fn info_payload(connected: bool) -> [u8; 12] {
    let mut payload = [0u8; 12];
    payload[1] = u8::from(connected) * 2; // 0 disconnected, 2 connected
    payload[2] = 2; // full gyro
    payload[3] = 2; // bluetooth
    payload[4..10].copy_from_slice(&[0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6]);
    payload[10] = 0x05; // full battery
    payload
}

/// 80-byte data payload. Layout follows cemuhook's DataResponse: the port
/// info block, packet counter, DS4-shaped button/stick bytes, two touch
/// points, then the motion block. Buttons are reported twice — as the
/// state1/state2 bitfields clients like Cemu read, and as the per-button
/// pressure bytes other clients expect (0xFF pressed).
fn data_payload(counter: u32, sample: &MotionSample, pad: &PadState) -> [u8; 8 * 10] {
    let mut payload = [0u8; 80];
    payload[..12].copy_from_slice(&info_payload(true));
    payload[11] = 1; // connected
    payload[12..16].copy_from_slice(&counter.to_le_bytes());

    // state1: share, l3, r3, options | dpad up, right, down, left
    let state1 = u8::from(pad.back)
        | u8::from(pad.l3) << 1
        | u8::from(pad.r3) << 2
        | u8::from(pad.start) << 3
        | u8::from(pad.dpad_up) << 4
        | u8::from(pad.dpad_right) << 5
        | u8::from(pad.dpad_down) << 6
        | u8::from(pad.dpad_left) << 7;
    // state2: l2, r2, l1, r1 | triangle, circle, cross, square
    let state2 = u8::from(pad.l2 > 0.0)
        | u8::from(pad.r2 > 0.0) << 1
        | u8::from(pad.l1) << 2
        | u8::from(pad.r1) << 3
        | u8::from(pad.triangle) << 4
        | u8::from(pad.circle) << 5
        | u8::from(pad.cross) << 6
        | u8::from(pad.square) << 7;
    payload[16] = state1;
    payload[17] = state2;
    payload[18] = u8::from(pad.guide); // ps

    let axis_byte = |value: f32| (((value + 1.0) / 2.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    payload[20] = axis_byte(pad.lx);
    payload[21] = axis_byte(pad.ly);
    payload[22] = axis_byte(pad.rx);
    payload[23] = axis_byte(pad.ry);

    let pressed = |on: bool| if on { 0xFF } else { 0x00 };
    payload[24] = pressed(pad.dpad_left);
    payload[25] = pressed(pad.dpad_down);
    payload[26] = pressed(pad.dpad_right);
    payload[27] = pressed(pad.dpad_up);
    payload[28] = pressed(pad.square);
    payload[29] = pressed(pad.cross);
    payload[30] = pressed(pad.circle);
    payload[31] = pressed(pad.triangle);
    payload[32] = pressed(pad.r1);
    payload[33] = pressed(pad.l1);
    payload[34] = (pad.r2.clamp(0.0, 1.0) * 255.0).round() as u8;
    payload[35] = (pad.l2.clamp(0.0, 1.0) * 255.0).round() as u8;

    payload[48..56].copy_from_slice(&sample.timestamp_us.to_le_bytes());
    for (offset, value) in payload[56..80]
        .chunks_exact_mut(4)
        .zip(sample.accel_ms2.iter().chain(sample.gyro_dps.iter()))
    {
        offset.copy_from_slice(&value.to_le_bytes());
    }
    payload
}

/// Whole packet: header + payload, CRC32 computed with the checksum bytes
/// zeroed and then written back.
fn build_packet(server_id: u32, message: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(20 + payload.len());
    packet.extend_from_slice(b"DSUS");
    packet.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    packet.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    packet.extend_from_slice(&[0; 4]); // CRC placeholder
    packet.extend_from_slice(&server_id.to_le_bytes());
    packet.extend_from_slice(&message.to_le_bytes());
    packet.extend_from_slice(payload);
    let crc = crc32(&packet).to_le_bytes();
    packet[8..12].copy_from_slice(&crc);
    packet
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Convert SDL sensor units to protocol units: gyro rad/s → deg/s with
/// pitch/yaw/roll on x/y/z; accelerometer already reports g.
pub fn sensor_to_motion(gyro_rads: [f32; 3], accel_ms2: [f32; 3], timestamp_us: u64) -> MotionSample {
    const RAD_TO_DEG: f32 = 180.0 / std::f32::consts::PI;
    MotionSample {
        accel_ms2,
        gyro_dps: [
            gyro_rads[0] * RAD_TO_DEG,
            gyro_rads[1] * RAD_TO_DEG,
            gyro_rads[2] * RAD_TO_DEG,
        ],
        timestamp_us,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_packet, crc32, data_payload, info_payload, sensor_to_motion, PadState,
    };
    use crate::{GamepadAxis, GamepadButton, OutputEvent};

    #[test]
    fn test_crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_packet_header_layout_and_crc_roundtrip() {
        let packet = build_packet(0x1122_3344, 0x100002, &[1, 2, 3]);
        assert_eq!(&packet[0..4], b"DSUS");
        assert_eq!(u16::from_le_bytes([packet[4], packet[5]]), 1001);
        assert_eq!(u16::from_le_bytes([packet[6], packet[7]]), 3);
        assert_eq!(u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]), 0x1122_3344);
        assert_eq!(u32::from_le_bytes([packet[16], packet[17], packet[18], packet[19]]), 0x100002);
        // Zeroing the CRC and recomputing must reproduce the stored value.
        let mut zeroed = packet.clone();
        zeroed[8..12].copy_from_slice(&[0; 4]);
        assert_eq!(crc32(&zeroed), u32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]));
    }

    #[test]
    fn test_data_payload_places_motion_fields() {
        let sample = sensor_to_motion([1.0, 0.0, -2.0], [0.1, 0.9, 0.2], 12_345);
        let payload = data_payload(7, &sample, &PadState::default());
        assert_eq!(payload.len(), 80);
        assert_eq!(payload[11], 1);
        assert_eq!(u32::from_le_bytes(payload[12..16].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(payload[48..56].try_into().unwrap()), 12_345);
        let accel_x = f32::from_le_bytes(payload[56..60].try_into().unwrap());
        let accel_y = f32::from_le_bytes(payload[60..64].try_into().unwrap());
        let accel_z = f32::from_le_bytes(payload[64..68].try_into().unwrap());
        assert_eq!(accel_x, 0.1);
        assert_eq!(accel_y, 0.9);
        assert_eq!(accel_z, 0.2);
        let pitch = f32::from_le_bytes(payload[68..72].try_into().unwrap());
        let yaw = f32::from_le_bytes(payload[72..76].try_into().unwrap());
        let roll = f32::from_le_bytes(payload[76..80].try_into().unwrap());
        assert!(pitch > 57.2 && pitch < 57.4, "pitch {pitch}");
        assert_eq!(yaw, 0.0);
        assert!((roll + 114.6).abs() < 0.1, "roll {roll}");
    }

    #[test]
    fn test_data_payload_encodes_full_controller_state() {
        let mut pad = PadState {
            cross: true,
            circle: true,
            back: true,
            dpad_up: true,
            r1: true,
            guide: true,
            l2: 0.5,
            lx: -1.0,
            ry: 1.0,
            ..PadState::default()
        };
        let sample = sensor_to_motion([0.0; 3], [0.0; 3], 1);
        let payload = data_payload(1, &sample, &pad);

        // state1: bit0 share, bit4 dpad up.
        assert_eq!(payload[16], 0b0001_0001);
        // state2: bit3 r1, bit5 circle, bit6 cross, bit0 l2 pressed.
        assert_eq!(payload[17], 0b0110_1001);
        assert_eq!(payload[18], 1, "ps button");
        // Sticks: -1 -> 0, +1 -> 255, center -> 128.
        assert_eq!(payload[20], 0);
        assert_eq!(payload[23], 255);
        assert_eq!(payload[21], 128);
        // Pressure bytes: face pressed = 0xFF, l2 analog = 128.
        assert_eq!(payload[29], 0xFF, "cross");
        assert_eq!(payload[30], 0xFF, "circle");
        assert_eq!(payload[31], 0, "triangle released");
        assert_eq!(payload[32], 0xFF, "r1");
        assert_eq!(payload[35], 128, "l2 half");

        // The shadow clears when outputs release.
        for event in [
            OutputEvent::GamepadButton {
                button: GamepadButton::A,
                pressed: false,
            },
            OutputEvent::GamepadAxis {
                axis: GamepadAxis::LeftTrigger,
                value: 0.0,
            },
        ] {
            pad.apply_output(&event);
        }
        assert!(!pad.cross);
        assert_eq!(pad.l2, 0.0);
    }

    #[test]
    fn test_pad_state_applies_output_events_positionally() {
        let mut pad = PadState::default();
        // A is south → cross; X is west → square (DS4 positions).
        for button in [GamepadButton::A, GamepadButton::X, GamepadButton::LeftStick] {
            pad.apply_output(&OutputEvent::GamepadButton {
                button,
                pressed: true,
            });
        }
        pad.apply_output(&OutputEvent::GamepadAxis {
            axis: GamepadAxis::RightTrigger,
            value: 0.75,
        });
        assert!(pad.cross);
        assert!(pad.square);
        assert!(pad.l3);
        assert!(!pad.circle);
        assert_eq!(pad.r2, 0.75);
    }

    #[test]
    fn test_info_payload_reports_connected_state() {
        assert_eq!(info_payload(false)[1], 0);
        assert_eq!(info_payload(true)[1], 2);
        assert_eq!(info_payload(true)[2], 2, "full gyro model");
    }
}
