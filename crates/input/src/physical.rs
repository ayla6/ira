use std::collections::{HashMap, HashSet};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::UNIX_EPOCH;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

use crate::{GamepadAxis, GamepadButton, InputEvent, InputSource};

const AXIS_MIN: f32 = -1.0;
const AXIS_MAX: f32 = 1.0;
const EIGHTBITDO_VENDOR: u16 = 0x2dc8;
const ULTIMATE_2_PRODUCT: u16 = 0x6012;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub has_evdev_gyro: bool,
    pub supported_buttons: Vec<GamepadButton>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControllerFamily {
    #[default]
    Generic,
    Xbox,
    PlayStation,
    Nintendo,
    EightBitDo,
    Steam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportedInputMode {
    XInput,
    DirectInput,
    PlayStation,
    Switch,
    Generic,
}

impl ReportedInputMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::XInput => "XInput-compatible",
            Self::DirectInput => "DirectInput-compatible",
            Self::PlayStation => "PlayStation controller",
            Self::Switch => "Switch controller",
            Self::Generic => "Generic gamepad",
        }
    }
}

impl DeviceInfo {
    pub fn family(&self) -> ControllerFamily {
        let name = self.name.to_ascii_lowercase();
        if self.vendor == EIGHTBITDO_VENDOR || name.contains("8bitdo") {
            ControllerFamily::EightBitDo
        } else if self.vendor == 0x045e || name.contains("xbox") || name.contains("x-input") {
            ControllerFamily::Xbox
        } else if name.contains("playstation")
            || name.contains("dualshock")
            || name.contains("dualsense")
            || name.contains("sony")
            || self.vendor == 0x054c
        {
            ControllerFamily::PlayStation
        } else if self.vendor == 0x057e
            || name.contains("nintendo")
            || name.contains("switch")
            || name.contains("joy-con")
        {
            ControllerFamily::Nintendo
        } else if self.vendor == 0x28de || name.contains("steam controller") {
            ControllerFamily::Steam
        } else {
            ControllerFamily::Generic
        }
    }

    /// Whether this pad should use the Nintendo face-button layout before
    /// the user has chosen explicitly: Nintendo-family controllers default
    /// to swapped A/B and X/Y, everything else to the positional standard.
    pub fn prefers_nintendo_layout(&self) -> bool {
        self.family() == ControllerFamily::Nintendo
    }

    /// Linux does not expose a controller's physical mode switch directly.
    /// This is the input layout identified from the device it currently reports.
    pub fn reported_input_mode(&self) -> ReportedInputMode {
        let name = self.name.to_ascii_lowercase();
        if name.contains("dinput") || name.contains("directinput") {
            ReportedInputMode::DirectInput
        } else if self.vendor == 0x045e
            || name.contains("xinput")
            || name.contains("x-input")
            || name.contains("xbox")
            || is_ultimate_2(self.vendor, self.product)
        {
            ReportedInputMode::XInput
        } else {
            match self.family() {
                ControllerFamily::PlayStation => ReportedInputMode::PlayStation,
                ControllerFamily::Nintendo => ReportedInputMode::Switch,
                _ => ReportedInputMode::Generic,
            }
        }
    }
}

pub fn discover_gamepads() -> Vec<DeviceInfo> {
    let mut devices: Vec<_> = evdev::enumerate()
        .filter_map(|(path, device)| device_info(path, &device))
        .collect();
    devices.sort_by(|left, right| left.path.cmp(&right.path));
    devices
}

fn device_info(path: PathBuf, device: &Device) -> Option<DeviceInfo> {
    let keys = device.supported_keys()?;
    if !keys.contains(KeyCode::BTN_SOUTH) {
        return None;
    }
    if device.name().is_some_and(is_ira_virtual_device) {
        return None;
    }
    let id = device.input_id();
    let layout = button_layout(id.vendor(), id.product(), device.name().unwrap_or_default());
    let mut supported_buttons = [
        KeyCode::BTN_SOUTH,
        KeyCode::BTN_EAST,
        KeyCode::BTN_WEST,
        KeyCode::BTN_NORTH,
        KeyCode::BTN_TL,
        KeyCode::BTN_TR,
        KeyCode::BTN_TL2,
        KeyCode::BTN_TR2,
        KeyCode::BTN_C,
        KeyCode::BTN_Z,
        KeyCode::BTN_SELECT,
        KeyCode::BTN_START,
        KeyCode::BTN_MODE,
        KeyCode::BTN_THUMBL,
        KeyCode::BTN_THUMBR,
        KeyCode::BTN_DPAD_UP,
        KeyCode::BTN_DPAD_DOWN,
        KeyCode::BTN_DPAD_LEFT,
        KeyCode::BTN_DPAD_RIGHT,
        KeyCode::BTN_TRIGGER_HAPPY1,
        KeyCode::BTN_TRIGGER_HAPPY2,
        KeyCode::BTN_TRIGGER_HAPPY3,
        KeyCode::BTN_TRIGGER_HAPPY4,
        KeyCode::BTN_TRIGGER_HAPPY5,
        KeyCode::BTN_TRIGGER_HAPPY6,
        KeyCode::BTN_TRIGGER_HAPPY7,
        KeyCode::BTN_TRIGGER_HAPPY8,
    ]
    .into_iter()
    .filter(|code| keys.contains(*code))
    .filter_map(|code| map_button_for_device(code, layout))
    .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    supported_buttons.retain(|button| seen.insert(*button));
    if device.supported_absolute_axes().is_some_and(|axes| {
        axes.contains(AbsoluteAxisCode::ABS_HAT0X) || axes.contains(AbsoluteAxisCode::ABS_HAT0Y)
    }) {
        for button in [
            GamepadButton::DpadUp,
            GamepadButton::DpadDown,
            GamepadButton::DpadLeft,
            GamepadButton::DpadRight,
        ] {
            if !supported_buttons.contains(&button) {
                supported_buttons.push(button);
            }
        }
    }
    let has_evdev_gyro = device.supported_absolute_axes().is_some_and(|axes| {
        [0x40, 0x41, 0x42]
            .into_iter()
            .all(|code| axes.contains(AbsoluteAxisCode(code)))
    });
    Some(DeviceInfo {
        path,
        name: device.name().unwrap_or("Unknown gamepad").to_string(),
        vendor: id.vendor(),
        product: id.product(),
        version: id.version(),
        has_evdev_gyro,
        supported_buttons,
    })
}

pub struct PhysicalGamepad {
    info: DeviceInfo,
    device: Option<Device>,
    hat_x: i32,
    hat_y: i32,
    axis_ranges: HashMap<AbsoluteAxisCode, (i32, i32)>,
    /// Last raw value seen per axis. Wireless pads re-send their full state
    /// every report even when nothing moved; suppressing repeats keeps the
    /// mapping engine and the virtual pad from churning on identical data.
    /// Cleared with the device on reconnect.
    last_axis_values: HashMap<AbsoluteAxisCode, i32>,
    z_axes_are_right_stick: bool,
    left_trigger_clicked: bool,
    right_trigger_clicked: bool,
    /// Controller-level Nintendo layout: face buttons swap (A↔B, X↔Y) as
    /// they enter the mapping engine, mirroring Steam's per-controller
    /// toggle.
    nintendo_layout: bool,
}

impl PhysicalGamepad {
    pub fn open(path: impl AsRef<Path>, grab: bool) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let mut device = Device::open(&path).map_err(|error| format_open_error(&path, error))?;
        let info = device_info(path.clone(), &device)
            .ok_or_else(|| format!("{} is not a supported gamepad", path.display()))?;
        let axis_ranges = read_axis_ranges(&device);
        let z_axes_are_right_stick = device.supported_absolute_axes().is_none_or(|axes| {
            !axes.contains(AbsoluteAxisCode::ABS_RX) && !axes.contains(AbsoluteAxisCode::ABS_RY)
        });
        device
            .set_nonblocking(true)
            .map_err(|error| format!("failed to make {} nonblocking: {error}", path.display()))?;
        if grab {
            device.grab().map_err(|error| {
                format!("failed to grab {} exclusively: {error}", path.display())
            })?;
        }
        Ok(Self {
            info,
            device: Some(device),
            hat_x: 0,
            hat_y: 0,
            axis_ranges,
            last_axis_values: HashMap::new(),
            z_axes_are_right_stick,
            left_trigger_clicked: false,
            right_trigger_clicked: false,
            nintendo_layout: false,
        })
    }

    /// Toggles the controller-level Nintendo face-button layout. Applied at
    /// session start and after reconnects from the per-controller settings.
    pub fn set_nintendo_layout(&mut self, on: bool) {
        self.nintendo_layout = on;
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn grab(&mut self) -> Result<(), String> {
        let Some(device) = self.device.as_mut() else {
            return Ok(());
        };
        device.grab().map_err(|error| {
            format!(
                "failed to grab {} exclusively: {error}",
                self.info.path.display()
            )
        })
    }

    pub fn is_connected(&self) -> bool {
        self.device.is_some()
    }

    /// Blocks until the kernel has another evdev event or scheduled work is due.
    pub fn wait_for_event(&self, timeout: Option<Duration>) -> Result<(), String> {
        let Some(device) = self.device.as_ref() else {
            let result = unsafe { libc::poll(std::ptr::null_mut(), 0, poll_timeout_ms(timeout)) };
            if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return Err(format!(
                    "failed waiting to reconnect {}: {}",
                    self.info.path.display(),
                    std::io::Error::last_os_error()
                ));
            }
            return Ok(());
        };
        let timeout_ms = poll_timeout_ms(timeout);
        let mut descriptor = libc::pollfd {
            fd: device.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return Err(format!(
                "failed waiting for {}: {}",
                self.info.path.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn try_reconnect(&mut self) -> Result<bool, String> {
        if self.device.is_some() {
            return Ok(false);
        }
        let Some(path) = discover_gamepads()
            .into_iter()
            .find(|device| same_device(&self.info, device))
            .map(|device| device.path)
        else {
            return Ok(false);
        };
        let replacement = Self::open(path, false)?;
        *self = replacement;
        Ok(true)
    }

    pub fn fetch_events(&mut self) -> Result<Vec<InputEvent>, String> {
        // Taken so the event iterator can be drained while `convert_event`
        // mutates the pad state; every path puts the device back except the
        // one where the kernel says it is gone.
        let Some(mut device) = self.device.take() else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        // The event iterator borrows the device, so its lifetime must end
        // before the device is moved back or dropped — hence the two-step
        // match instead of putting the device back inside each arm.
        enum FetchOutcome {
            Drained,
            WouldBlock,
            Gone,
            Failed(String),
        }
        let outcome = match device.fetch_events() {
            Ok(events) => {
                for event in events {
                    self.convert_event(event, &mut result);
                }
                FetchOutcome::Drained
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                FetchOutcome::WouldBlock
            }
            Err(ref error) if device_gone(error) => FetchOutcome::Gone,
            Err(error) => FetchOutcome::Failed(error.to_string()),
        };
        match outcome {
            FetchOutcome::Drained | FetchOutcome::WouldBlock => self.device = Some(device),
            FetchOutcome::Gone => {
                eprintln!(
                    "ira-input: controller {} disconnected; continuing without physical input",
                    self.info.path.display()
                );
                close_disconnected_device(device);
            }
            FetchOutcome::Failed(error) => {
                self.device = Some(device);
                return Err(format!(
                    "failed reading {}: {error}",
                    self.info.path.display()
                ));
            }
        }
        Ok(result)
    }

    fn convert_event(&mut self, event: evdev::InputEvent, output: &mut Vec<InputEvent>) {
        let timestamp_us = event
            .timestamp()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_micros() as u64)
            .unwrap_or_default();
        match event.destructure() {
            EventSummary::Key(_, code, value) => {
                let layout = button_layout(
                    self.info.vendor,
                    self.info.product,
                    &self.info.name,
                );
                if let Some(mut button) = map_button_for_device(code, layout) {
                    if self.nintendo_layout {
                        button = swap_face_buttons(button);
                    }
                    output.push(InputEvent {
                        source: InputSource::Button(button),
                        value: if value == 0 { 0.0 } else { 1.0 },
                        timestamp_us,
                    });
                }
            }
            EventSummary::AbsoluteAxis(_, code, value) => {
                self.convert_axis(code, value, timestamp_us, output);
            }
            _ => {}
        }
    }

    fn convert_axis(
        &mut self,
        code: AbsoluteAxisCode,
        value: i32,
        timestamp_us: u64,
        output: &mut Vec<InputEvent>,
    ) {
        // Identical axis reports carry no information: the same raw value
        // normalizes to the same output and moves no trigger-click edge.
        // Skipping them keeps full-state report bursts from re-processing
        // (and re-emitting) unchanged sticks several hundred times a second.
        if self.last_axis_values.get(&code) == Some(&value) {
            return;
        }
        self.last_axis_values.insert(code, value);
        let source = match code {
            AbsoluteAxisCode::ABS_X => {
                Some((GamepadAxis::LeftX, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_Y => {
                Some((GamepadAxis::LeftY, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_RX => {
                Some((GamepadAxis::RightX, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_RY => {
                Some((GamepadAxis::RightY, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_Z if self.z_axes_are_right_stick => {
                Some((GamepadAxis::RightX, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_RZ if self.z_axes_are_right_stick => {
                Some((GamepadAxis::RightY, self.normalize_signed(code, value)))
            }
            AbsoluteAxisCode::ABS_Z => Some((
                GamepadAxis::LeftTrigger,
                self.normalize_trigger(code, value),
            )),
            AbsoluteAxisCode::ABS_RZ => Some((
                GamepadAxis::RightTrigger,
                self.normalize_trigger(code, value),
            )),
            AbsoluteAxisCode::ABS_GAS => Some((
                GamepadAxis::RightTrigger,
                self.normalize_trigger(code, value),
            )),
            AbsoluteAxisCode::ABS_BRAKE => Some((
                GamepadAxis::LeftTrigger,
                self.normalize_trigger(code, value),
            )),
            AbsoluteAxisCode::ABS_HAT0X => {
                self.hat_x = value;
                None
            }
            AbsoluteAxisCode::ABS_HAT0Y => {
                self.hat_y = value;
                None
            }
            _ => None,
        };
        if let Some((axis, normalized)) = source {
            output.push(InputEvent {
                source: InputSource::Axis(axis),
                value: normalized,
                timestamp_us,
            });
            self.synthesize_trigger_click(axis, normalized, timestamp_us, output);
        }
        if matches!(
            code,
            AbsoluteAxisCode::ABS_HAT0X | AbsoluteAxisCode::ABS_HAT0Y
        ) {
            self.emit_hat_events(timestamp_us, output);
        }
    }

    /// Many pads (DInput mode especially) report triggers only as analog
    /// axes with no BTN_TL2/BTN_TR2 keys, so button-style trigger bindings
    /// would never fire. Synthesize click edges from the axis the same way
    /// SDL does, with hysteresis so a noisy axis resting near the threshold
    /// cannot chatter. Pads that DO have digital trigger keys are left
    /// alone entirely: trusting the real keys avoids double edges and any
    /// chance of a synthesized click sticking when the axis goes quiet.
    fn synthesize_trigger_click(
        &mut self,
        axis: GamepadAxis,
        value: f32,
        timestamp_us: u64,
        output: &mut Vec<InputEvent>,
    ) {
        let (button, clicked) = match axis {
            GamepadAxis::LeftTrigger => {
                (GamepadButton::LeftTrigger, &mut self.left_trigger_clicked)
            }
            GamepadAxis::RightTrigger => {
                (GamepadButton::RightTrigger, &mut self.right_trigger_clicked)
            }
            _ => return,
        };
        if self.info.supported_buttons.contains(&button) {
            return;
        }
        let previous = *clicked;
        *clicked = synthesized_trigger_click(previous, value);
        if *clicked != previous {
            output.push(InputEvent {
                source: InputSource::Button(button),
                value: f32::from(*clicked),
                timestamp_us,
            });
        }
    }

    fn normalize_signed(&self, code: AbsoluteAxisCode, value: i32) -> f32 {
        let (minimum, maximum) = self.axis_ranges.get(&code).copied().unwrap_or((-1, 1));
        normalize_signed(value, minimum, maximum)
    }

    fn normalize_trigger(&self, code: AbsoluteAxisCode, value: i32) -> f32 {
        let (minimum, maximum) = self.axis_ranges.get(&code).copied().unwrap_or((0, 255));
        normalize_trigger(value, minimum, maximum)
    }

    fn emit_hat_events(&self, timestamp_us: u64, output: &mut Vec<InputEvent>) {
        output.extend(
            [
                (GamepadButton::DpadLeft, self.hat_x < 0),
                (GamepadButton::DpadRight, self.hat_x > 0),
                (GamepadButton::DpadUp, self.hat_y < 0),
                (GamepadButton::DpadDown, self.hat_y > 0),
            ]
            .into_iter()
            .map(|(button, pressed)| InputEvent {
                source: InputSource::Button(button),
                value: f32::from(pressed),
                timestamp_us,
            }),
        );
    }
}

fn poll_timeout_ms(timeout: Option<Duration>) -> libc::c_int {
    let Some(timeout) = timeout else {
        return -1;
    };
    timeout
        .as_nanos()
        .div_ceil(1_000_000)
        .min(libc::c_int::MAX as u128) as libc::c_int
}

fn device_gone(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENODEV | libc::ENXIO | libc::EBADF)
    )
}

fn same_device(left: &DeviceInfo, right: &DeviceInfo) -> bool {
    left.vendor == right.vendor && left.product == right.product && left.name == right.name
}

fn close_disconnected_device(device: Device) {
    let fd = device.as_raw_fd();
    std::mem::forget(device);
    unsafe {
        libc::close(fd);
    }
}

fn map_button(code: KeyCode) -> Option<GamepadButton> {
    Some(match code {
        KeyCode::BTN_SOUTH => GamepadButton::A,
        KeyCode::BTN_EAST => GamepadButton::B,
        KeyCode::BTN_NORTH => GamepadButton::Y,
        KeyCode::BTN_WEST => GamepadButton::X,
        KeyCode::BTN_TL => GamepadButton::LeftShoulder,
        KeyCode::BTN_TR => GamepadButton::RightShoulder,
        KeyCode::BTN_TL2 => GamepadButton::LeftTrigger,
        KeyCode::BTN_TR2 => GamepadButton::RightTrigger,
        KeyCode::BTN_C => GamepadButton::Paddle1,
        KeyCode::BTN_Z => GamepadButton::Paddle2,
        KeyCode::BTN_SELECT => GamepadButton::Back,
        KeyCode::BTN_START => GamepadButton::Start,
        KeyCode::BTN_MODE => GamepadButton::Guide,
        KeyCode::BTN_THUMBL => GamepadButton::LeftStick,
        KeyCode::BTN_THUMBR => GamepadButton::RightStick,
        KeyCode::BTN_DPAD_UP => GamepadButton::DpadUp,
        KeyCode::BTN_DPAD_DOWN => GamepadButton::DpadDown,
        KeyCode::BTN_DPAD_LEFT => GamepadButton::DpadLeft,
        KeyCode::BTN_DPAD_RIGHT => GamepadButton::DpadRight,
        KeyCode::BTN_TRIGGER_HAPPY1 => GamepadButton::Paddle1,
        KeyCode::BTN_TRIGGER_HAPPY2 => GamepadButton::Paddle2,
        KeyCode::BTN_TRIGGER_HAPPY3 => GamepadButton::Paddle3,
        KeyCode::BTN_TRIGGER_HAPPY4 => GamepadButton::Paddle4,
        KeyCode::BTN_TRIGGER_HAPPY5 => GamepadButton::Paddle5,
        KeyCode::BTN_TRIGGER_HAPPY6 => GamepadButton::Paddle6,
        KeyCode::BTN_TRIGGER_HAPPY7 => GamepadButton::Paddle7,
        KeyCode::BTN_TRIGGER_HAPPY8 => GamepadButton::Paddle8,
        _ => return None,
    })
}

/// Which wire layout a device's evdev codes follow, deciding how raw
/// key codes map to [`GamepadButton`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonLayout {
    /// evdev's positional standard shared by xpad, hid-sony, hid-nintendo.
    Standard,
    /// The 8BitDo Ultimate 2 in DInput mode, aligned with SDL's HIDAPI
    /// driver: P1=R4, P2=L4, P3=PR, P4=PL.
    Ultimate2DInput,
    /// A controller speaking Nintendo's positional layout through
    /// hid-nintendo, whose `BTN_Z` is the capture button, not a paddle.
    Nintendo,
}

/// Picks the layout from device identity: the Ultimate 2's DInput dongle
/// gets its paddle table, Nintendo-family pads get theirs, everything else
/// the positional standard.
fn button_layout(vendor: u16, product: u16, name: &str) -> ButtonLayout {
    if is_ultimate_2(vendor, product) {
        ButtonLayout::Ultimate2DInput
    } else if is_nintendo(vendor, name) {
        ButtonLayout::Nintendo
    } else {
        ButtonLayout::Standard
    }
}

/// The vendor check mirrors `DeviceInfo::family`'s Nintendo arm.
fn is_nintendo(vendor: u16, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    vendor == 0x057e
        || name.contains("nintendo")
        || name.contains("switch")
        || name.contains("joy-con")
}

/// Swaps face buttons for the controller-level Nintendo layout, matching
/// Steam's per-controller toggle: physical A reports as A and X as X on a
/// positional standard, i.e. A↔B and X↔Y trade places. Other buttons pass
/// through untouched.
pub fn swap_face_buttons(button: GamepadButton) -> GamepadButton {
    match button {
        GamepadButton::A => GamepadButton::B,
        GamepadButton::B => GamepadButton::A,
        GamepadButton::X => GamepadButton::Y,
        GamepadButton::Y => GamepadButton::X,
        other => other,
    }
}

fn map_button_for_device(code: KeyCode, layout: ButtonLayout) -> Option<GamepadButton> {
    match layout {
        ButtonLayout::Ultimate2DInput => Some(match code {
            // Keep the Ultimate 2 mapping aligned with SDL's HIDAPI driver:
            // P1=R4, P2=L4, P3=PR, P4=PL.
            KeyCode::BTN_C => GamepadButton::Paddle3,
            KeyCode::BTN_Z => GamepadButton::Paddle4,
            KeyCode::BTN_TRIGGER_HAPPY1 => GamepadButton::Paddle2,
            KeyCode::BTN_TRIGGER_HAPPY2 => GamepadButton::Paddle1,
            KeyCode::BTN_TRIGGER_HAPPY3
            | KeyCode::BTN_TRIGGER_HAPPY4
            | KeyCode::BTN_TRIGGER_HAPPY5
            | KeyCode::BTN_TRIGGER_HAPPY6
            | KeyCode::BTN_TRIGGER_HAPPY7
            | KeyCode::BTN_TRIGGER_HAPPY8 => return None,
            KeyCode::BTN_NORTH => GamepadButton::X,
            KeyCode::BTN_WEST => GamepadButton::Y,
            code => map_button(code)?,
        }),
        ButtonLayout::Nintendo => match code {
            // hid-nintendo reports the capture button as BTN_Z; there is no
            // paddle hardware behind it, so it must not masquerade as one.
            KeyCode::BTN_Z => None,
            code => map_button(code),
        },
        ButtonLayout::Standard => map_button(code),
    }
}

fn is_ultimate_2(vendor: u16, product: u16) -> bool {
    vendor == EIGHTBITDO_VENDOR && product == ULTIMATE_2_PRODUCT
}

fn read_axis_ranges(device: &Device) -> HashMap<AbsoluteAxisCode, (i32, i32)> {
    device
        .get_absinfo()
        .ok()
        .into_iter()
        .flatten()
        .map(|(code, info)| (code, (info.minimum(), info.maximum())))
        .collect()
}

fn normalize_signed(value: i32, minimum: i32, maximum: i32) -> f32 {
    if minimum >= maximum {
        return 0.0;
    }
    let normalized = ((value - minimum) as f32 / (maximum - minimum) as f32) * 2.0 - 1.0;
    normalized.clamp(AXIS_MIN, AXIS_MAX)
}

fn normalize_trigger(value: i32, minimum: i32, maximum: i32) -> f32 {
    if minimum >= maximum {
        return 0.0;
    }
    ((value - minimum) as f32 / (maximum - minimum) as f32).clamp(0.0, 1.0)
}

/// Click engages past the press threshold and releases only below a lower
/// threshold; between the two the previous state sticks.
const TRIGGER_CLICK_PRESS: f32 = 0.5;
const TRIGGER_CLICK_RELEASE: f32 = 0.35;

fn synthesized_trigger_click(previous: bool, value: f32) -> bool {
    if previous {
        value > TRIGGER_CLICK_RELEASE
    } else {
        value >= TRIGGER_CLICK_PRESS
    }
}

fn format_open_error(path: &Path, error: std::io::Error) -> String {
    format!(
        "failed to open {}: {} (kind={:?}, raw_os_error={:?})",
        path.display(),
        error,
        error.kind(),
        error.raw_os_error()
    )
}

fn is_ira_virtual_device(name: &str) -> bool {
    name.starts_with("Ira Virtual ")
}

#[cfg(test)]
mod tests {
    use super::{
        button_layout, device_gone, is_ira_virtual_device, is_ultimate_2, map_button,
        map_button_for_device, normalize_signed, normalize_trigger, poll_timeout_ms,
        same_device, swap_face_buttons, synthesized_trigger_click, ButtonLayout,
        ControllerFamily, DeviceInfo, ReportedInputMode, EIGHTBITDO_VENDOR, ULTIMATE_2_PRODUCT,
    };
    use crate::GamepadButton;
    use std::path::PathBuf;
    use std::time::Duration;

    fn test_pad() -> super::PhysicalGamepad {
        super::PhysicalGamepad {
            info: DeviceInfo {
                path: PathBuf::from("/dev/input/eventtest"),
                name: "Test Bench Pad".to_string(),
                vendor: 0x1234,
                product: 0x5678,
                version: 0,
                has_evdev_gyro: false,
                supported_buttons: Vec::new(),
            },
            device: None,
            hat_x: 0,
            hat_y: 0,
            axis_ranges: {
                let mut ranges = std::collections::HashMap::new();
                ranges.insert(evdev::AbsoluteAxisCode::ABS_X, (-32768, 32767));
                ranges.insert(evdev::AbsoluteAxisCode::ABS_Y, (-32768, 32767));
                ranges.insert(evdev::AbsoluteAxisCode::ABS_Z, (0, 255));
                ranges
            },
            last_axis_values: std::collections::HashMap::new(),
            z_axes_are_right_stick: false,
            left_trigger_clicked: false,
            right_trigger_clicked: false,
            nintendo_layout: false,
        }
    }

    #[test]
    fn test_convert_axis_suppresses_repeated_values() {
        let mut pad = test_pad();
        let mut out = Vec::new();
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_X, 1000, 1, &mut out);
        assert_eq!(out.len(), 1, "first report emits");
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_X, 1000, 2, &mut out);
        assert_eq!(out.len(), 1, "identical report is suppressed");
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_X, -2000, 3, &mut out);
        assert_eq!(out.len(), 2, "a real change emits again");
        // Other axes keep their own last-seen value.
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_Y, 1000, 4, &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn test_convert_axis_suppression_does_not_block_trigger_clicks() {
        let mut pad = test_pad();
        let mut out = Vec::new();
        // Full pull, hold (suppressed repeat), release: the click edge is
        // synthesized from the values that DO change, so the hold must not
        // swallow the release edge.
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_Z, 255, 1, &mut out);
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_Z, 255, 2, &mut out);
        pad.convert_axis(evdev::AbsoluteAxisCode::ABS_Z, 0, 3, &mut out);
        let buttons = out
            .iter()
            .filter(|event| matches!(event.source, crate::InputSource::Button(_)))
            .count();
        assert_eq!(buttons, 2, "press and release click edges both fire");
    }

    #[test]
    fn test_map_extra_grip_buttons() {
        assert_eq!(
            map_button(evdev::KeyCode::BTN_C),
            Some(GamepadButton::Paddle1)
        );
        assert_eq!(
            map_button(evdev::KeyCode::BTN_Z),
            Some(GamepadButton::Paddle2)
        );
    }

    #[test]
    fn test_map_standard_face_button_positions() {
        assert_eq!(
            map_button(evdev::KeyCode::BTN_NORTH),
            Some(GamepadButton::Y)
        );
        assert_eq!(map_button(evdev::KeyCode::BTN_WEST), Some(GamepadButton::X));
    }

    #[test]
    fn test_map_ultimate_2_buttons() {
        assert!(is_ultimate_2(0x2dc8, 0x6012));
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_NORTH, ButtonLayout::Ultimate2DInput),
            Some(GamepadButton::X)
        );
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_WEST, ButtonLayout::Ultimate2DInput),
            Some(GamepadButton::Y)
        );
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_Z, ButtonLayout::Ultimate2DInput),
            Some(GamepadButton::Paddle4)
        );
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_C, ButtonLayout::Ultimate2DInput),
            Some(GamepadButton::Paddle3)
        );
        assert_eq!(
            map_button_for_device(
                evdev::KeyCode::BTN_TRIGGER_HAPPY1,
                ButtonLayout::Ultimate2DInput
            ),
            Some(GamepadButton::Paddle2)
        );
        assert_eq!(
            map_button_for_device(
                evdev::KeyCode::BTN_TRIGGER_HAPPY2,
                ButtonLayout::Ultimate2DInput
            ),
            Some(GamepadButton::Paddle1)
        );
        assert_eq!(
            map_button_for_device(
                evdev::KeyCode::BTN_TRIGGER_HAPPY8,
                ButtonLayout::Ultimate2DInput
            ),
            None
        );
        assert!(!is_ultimate_2(0x2dc8, 0x310b));
    }

    #[test]
    fn test_nintendo_layout_drops_capture_instead_of_faking_a_paddle() {
        // hid-nintendo reports capture as BTN_Z; on Nintendo pads that must
        // not surface as a phantom Paddle2 the hardware does not have.
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_Z, ButtonLayout::Nintendo),
            None
        );
        // Standard positional buttons keep their usual meaning.
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_SOUTH, ButtonLayout::Nintendo),
            Some(GamepadButton::A)
        );
        assert_eq!(
            map_button_for_device(evdev::KeyCode::BTN_EAST, ButtonLayout::Nintendo),
            Some(GamepadButton::B)
        );
    }

    #[test]
    fn test_button_layout_identifies_device_families() {
        assert_eq!(
            button_layout(0x2dc8, 0x6012, "8BitDo Ultimate 2 Wireless Controller"),
            ButtonLayout::Ultimate2DInput
        );
        assert_eq!(
            button_layout(0x057e, 0x2009, "Nintendo Switch Pro Controller"),
            ButtonLayout::Nintendo
        );
        assert_eq!(
            button_layout(0x045e, 0x028e, "Xbox 360 Controller"),
            ButtonLayout::Standard
        );
        // A Switch-mode 8BitDo dongle impersonating a Pro Controller.
        assert_eq!(
            button_layout(0x057e, 0x2009, "8BitDo Ultimate 2 Wireless Controller"),
            ButtonLayout::Nintendo
        );
    }

    #[test]
    fn test_swap_face_buttons_trades_ab_and_xy_only() {
        assert_eq!(swap_face_buttons(GamepadButton::A), GamepadButton::B);
        assert_eq!(swap_face_buttons(GamepadButton::B), GamepadButton::A);
        assert_eq!(swap_face_buttons(GamepadButton::X), GamepadButton::Y);
        assert_eq!(swap_face_buttons(GamepadButton::Y), GamepadButton::X);
        assert_eq!(swap_face_buttons(GamepadButton::Start), GamepadButton::Start);
        assert_eq!(
            swap_face_buttons(GamepadButton::Paddle3),
            GamepadButton::Paddle3
        );
        // Swapping twice restores the positional meaning.
        for button in [
            GamepadButton::A,
            GamepadButton::B,
            GamepadButton::X,
            GamepadButton::Y,
        ] {
            assert_eq!(swap_face_buttons(swap_face_buttons(button)), button);
        }
    }

    #[test]
    fn test_controller_family_identifies_reported_brand() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "8BitDo Ultimate Controller".to_string(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        assert_eq!(device.family(), ControllerFamily::EightBitDo);
    }

    #[test]
    fn test_controller_family_identifies_vendor() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Wireless Gamepad".to_string(),
            vendor: 0x2dc8,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        assert_eq!(device.family(), ControllerFamily::EightBitDo);
    }

    #[test]
    fn test_reported_input_mode_prefers_explicit_direct_input_name() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "8BitDo Controller (DInput)".to_string(),
            vendor: EIGHTBITDO_VENDOR,
            product: ULTIMATE_2_PRODUCT,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        assert_eq!(device.reported_input_mode(), ReportedInputMode::DirectInput);
    }

    #[test]
    fn test_reported_input_mode_identifies_ultimate_2_xinput_layout() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "8BitDo Ultimate 2 Wireless Controller for PC".to_string(),
            vendor: EIGHTBITDO_VENDOR,
            product: ULTIMATE_2_PRODUCT,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        assert_eq!(device.reported_input_mode(), ReportedInputMode::XInput);
    }

    #[test]
    fn test_poll_timeout_ms_preserves_short_deadlines() {
        assert_eq!(poll_timeout_ms(None), -1);
        assert_eq!(poll_timeout_ms(Some(Duration::ZERO)), 0);
        assert_eq!(poll_timeout_ms(Some(Duration::from_nanos(1))), 1);
        assert_eq!(poll_timeout_ms(Some(Duration::from_millis(4))), 4);
        assert_eq!(poll_timeout_ms(Some(Duration::MAX)), libc::c_int::MAX);
    }

    #[test]
    fn test_same_device_matches_identity() {
        let left = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Controller".to_string(),
            vendor: 1,
            product: 2,
            version: 3,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        };
        let right = DeviceInfo {
            path: PathBuf::from("/dev/input/event9"),
            ..left.clone()
        };
        assert!(same_device(&left, &right));
    }

    #[test]
    fn test_device_gone_matches_removed_evdev_errors() {
        assert!(device_gone(&std::io::Error::from_raw_os_error(
            libc::ENODEV
        )));
        assert!(!device_gone(&std::io::Error::from_raw_os_error(libc::EIO)));
    }

    #[test]
    fn test_ira_virtual_devices_are_not_physical_inputs() {
        assert!(is_ira_virtual_device("Ira Virtual Xbox Controller"));
        assert!(!is_ira_virtual_device("8BitDo Ultimate Controller"));
    }

    #[test]
    fn test_normalize_signed_controller_axis() {
        assert!((normalize_signed(127, 0, 255) + 0.0039).abs() < 0.01);
        assert_eq!(normalize_signed(0, 0, 255), -1.0);
        assert_eq!(normalize_signed(255, 0, 255), 1.0);
        assert_eq!(normalize_signed(10, 10, 10), 0.0);
    }

    #[test]
    fn test_normalize_trigger_controller_axis() {
        assert_eq!(normalize_trigger(0, 0, 255), 0.0);
        assert_eq!(normalize_trigger(255, 0, 255), 1.0);
        assert_eq!(normalize_trigger(300, 0, 255), 1.0);
        assert_eq!(normalize_trigger(10, 10, 10), 0.0);
    }

    #[test]
    fn test_synthesized_trigger_click_presses_and_releases_with_hysteresis() {
        // Resting and mid-travel: no click.
        assert!(!synthesized_trigger_click(false, 0.0));
        assert!(!synthesized_trigger_click(false, 0.49));
        // Crossing the press threshold clicks.
        assert!(synthesized_trigger_click(false, 0.5));
        assert!(synthesized_trigger_click(true, 1.0));
        // Between the thresholds the click holds; below release it drops.
        assert!(synthesized_trigger_click(true, 0.4));
        assert!(!synthesized_trigger_click(true, 0.35));
        // A noisy axis hovering in the band cannot re-click on its own.
        assert!(!synthesized_trigger_click(false, 0.4));
    }
}
