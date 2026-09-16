use std::collections::HashMap;
use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventSummary, EventType, FFEffectCode,
    InputEvent, InputId, KeyCode, UInputCode, UinputAbsSetup,
};

use crate::rumble::RumbleCommand;
use crate::{GamepadAxis, GamepadButton, OutputEvent, VirtualGamepadBackend};

/// Effect slots the virtual pad advertises. Games only ever need a couple of
/// rumble effects alive; four covers SDL's cached rumble plus headroom.
const RUMBLE_EFFECT_SLOTS: u32 = 4;

const VIRTUAL_VENDOR: u16 = 0x045e;
const VIRTUAL_PRODUCT: u16 = 0x028e;
const VIRTUAL_VERSION: u16 = 0x0114;
// Ira's private evdev identity: BUS_VIRTUAL plus the ASCII tag "IR" as VID.
// This is not a USB allocation and must not be presented as one.
const DIRECT_INPUT_VENDOR: u16 = 0x4952;
const DIRECT_INPUT_PRODUCT: u16 = 0x0001;
const DIRECT_INPUT_VERSION: u16 = 0x0001;
const DIRECT_INPUT_NAME: &str = "Ira Virtual DirectInput Controller";
const DIRECT_INPUT_SDL_BINDINGS: &str = "a:b0,b:b1,x:b2,y:b3,leftshoulder:b4,rightshoulder:b5,lefttrigger:a2,righttrigger:a5,back:b8,start:b9,guide:b10,leftstick:b11,rightstick:b12,dpup:b13,dpdown:b14,dpleft:b15,dpright:b16,leftx:a0,lefty:a1,rightx:a3,righty:a4,paddle1:b17,paddle2:b18,paddle3:b19,paddle4:b20";
const SWITCH_PRO_VENDOR: u16 = 0x057e;
const SWITCH_PRO_PRODUCT: u16 = 0x2009;
const SWITCH_PRO_VERSION: u16 = 0x8111;
const SWITCH_PRO_NAME: &str = "Ira Virtual Nintendo Switch Pro Controller";
const SWITCH_PRO_SDL_BINDINGS: &str = "a:b0,b:b1,back:b9,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b11,leftshoulder:b5,leftstick:b12,lefttrigger:b7,leftx:a0,lefty:a1,misc1:b4,rightshoulder:b6,rightstick:b13,righttrigger:b8,rightx:a2,righty:a3,start:b10,x:b2,y:b3,platform:Linux";
// Sony ids copied from the kernel drivers' hardware: hid-sony exposes the
// DualShock 4 as 054c:09cc rev 0x0001, hid-playstation the DualSense as
// 054c:0ce6. SDL ships evdev mappings for those GUIDs in its built-in
// controller database, so emulators recognize the pads without extra config.
//
// Both Sony kernel drivers use the same quirky evdev layout: the right stick
// sits on ABS_Z/ABS_RZ, the analog triggers on ABS_RX/ABS_RY, the d-pad is
// hat 0, and square is BTN_C rather than BTN_WEST.
const DUAL_SHOCK_4_VENDOR: u16 = 0x054c;
const DUAL_SHOCK_4_PRODUCT: u16 = 0x09cc;
const DUAL_SHOCK_4_VERSION: u16 = 0x0001;
const DUAL_SHOCK_4_NAME: &str = "Sony Interactive Entertainment Wireless Controller";
const DUAL_SHOCK_4_GUID: &str = "030000004c050000cc09000000010000";
const DUAL_SENSE_VENDOR: u16 = 0x054c;
const DUAL_SENSE_PRODUCT: u16 = 0x0ce6;
const DUAL_SENSE_VERSION: u16 = 0x0111;
const DUAL_SENSE_NAME: &str = "Sony Interactive Entertainment DualSense Wireless Controller";
const DUAL_SENSE_GUID: &str = "030000004c050000e60c000011010000";
// Valve's Steam Input output identity: the 28de:11ff pair SDL special-cases
// as USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD, and the name games see when a
// remapper stands between them and the hardware. The evdev node keeps an
// Ira-prefixed name (like every backend) so the hub never routes our own
// pad as a physical controller; the mapping string below is what renames it
// "Steam Virtual Gamepad" inside SDL games. The version matches the
// controller database entry for this identity so browser consumers GUID-
// match it instead of falling back to positional button binding.
const STEAM_INPUT_VENDOR: u16 = 0x28de;
const STEAM_INPUT_PRODUCT: u16 = 0x11ff;
const STEAM_INPUT_VERSION: u16 = 0x0100;
const STEAM_INPUT_NAME: &str = "Ira Virtual Steam Input Controller";
const STEAM_INPUT_GUID: &str = "03000000de280000ff11000000010000";
/// The name the SDL mapping carries: games ask SDL for the controller's
/// name, and SDL answers with the mapping's.
const STEAM_INPUT_SDL_NAME: &str = "Steam Virtual Gamepad";
const STEAM_INPUT_SDL_BINDINGS: &str = "a:b0,b:b1,x:b2,y:b3,leftshoulder:b4,rightshoulder:b5,lefttrigger:a2,righttrigger:a5,back:b6,start:b7,guide:b8,leftstick:b9,rightstick:b10,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,leftx:a0,lefty:a1,rightx:a3,righty:a4,platform:Linux";

fn sony_sdl_bindings() -> &'static str {
    "a:b0,b:b1,x:b2,y:b3,back:b8,start:b9,guide:b12,leftstick:b10,rightstick:b11,leftshoulder:b4,rightshoulder:b5,dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,leftx:a0,lefty:a1,rightx:a2,righty:a5,lefttrigger:a3,righttrigger:a4,misc1:b13,platform:Linux"
}

/// SDL bindings for the Sony uhid twins' evdev layout. Their descriptors
/// declare the face buttons as individual Button-page usages (west/south/
/// east/north), so the evdev indices differ from the kernel Sony drivers
/// the mapping above was written against: square lands on b3 and triangle
/// on b2, the guide on b10, and the stick clicks on b11/b12. Sticks and
/// analog triggers sit on the same axes (right stick ABS_Z/ABS_RZ, L2/R2
/// on ABS_RX/ABS_RY).
pub(crate) const SONY_TWIN_SDL_BINDINGS: &str = "a:b0,b:b1,x:b3,y:b2,back:b8,start:b9,guide:b10,leftstick:b11,rightstick:b12,leftshoulder:b4,rightshoulder:b5,lefttrigger:a3,righttrigger:a4,dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,leftx:a0,lefty:a1,rightx:a2,righty:a5,platform:Linux";

/// Builds the SDL_GAMECONTROLLERCONFIG mapping a Sony uhid twin ships in
/// the game's environment. SDL matches mappings by GUID, and no SDL
/// database maps the twins' third-party identities — without this, the
/// twins show up as raw joysticks no gamecontroller-based game can open.
pub(crate) fn sony_twin_sdl_mapping(guid: &str, name: &str) -> String {
    format!("{guid},{name},{SONY_TWIN_SDL_BINDINGS}")
}

pub struct VirtualGamepad {
    /// `None` for the DSU backend: it creates no kernel device and exists
    /// only so the output pipeline has a uniform sink; the real carrier is
    /// the cemuhook stream.
    device: Option<VirtualDevice>,
    backend: VirtualGamepadBackend,
    hat_dpad: [bool; 4],
    /// Events collected by `emit`, written to the kernel in one syscall by
    /// `flush`. The pipeline runs at the controller's report rate, where one
    /// write per output event dominated the daemon's idle cost.
    pending: Vec<InputEvent>,
    /// Rumble effects the game uploaded (EVIOCSFF), keyed by kernel effect
    /// id, waiting for the playback event that runs them.
    rumble_effects: HashMap<i16, RumbleCommand>,
}

/// Turns one playback event (EV_FF, `value` 1 = run, 0 = stop) for `id`
/// into the command to replay. Starting an effect the pad never saw is
/// ignored; any stop always stops, whether or not its effect is known.
fn playback_command(
    effects: &HashMap<i16, RumbleCommand>,
    id: FFEffectCode,
    value: i32,
) -> Option<RumbleCommand> {
    if value == 0 {
        return Some(crate::rumble::stop_command());
    }
    effects.get(&(id.0 as i16)).copied()
}

impl VirtualGamepad {
    pub fn create() -> io::Result<Self> {
        Self::create_for_backend(VirtualGamepadBackend::XInput)
    }

    /// A backend with no kernel device: outputs land only in the pad shadow
    /// that whole-controller carriers (the cemuhook stream, the uhid DS4)
    /// read from, so games see a single controller.
    pub fn shadow_only(backend: VirtualGamepadBackend) -> Self {
        Self {
            device: None,
            backend,
            hat_dpad: [false; 4],
            pending: Vec::new(),
            rumble_effects: HashMap::new(),
        }
    }

    pub fn create_for_backend(backend: VirtualGamepadBackend) -> io::Result<Self> {
        if backend == VirtualGamepadBackend::Dsu {
            return Ok(Self::shadow_only(backend));
        }
        let buttons = gamepad_buttons(backend);
        let rumble_effects: AttributeSet<FFEffectCode> =
            [FFEffectCode::FF_RUMBLE].into_iter().collect();
        let mut builder = VirtualDevice::builder()?
            .name(device_name(backend))
            .input_id(device_id(backend))
            .with_keys(&buttons)?
            .with_ff(&rumble_effects)?
            .with_ff_effects_max(RUMBLE_EFFECT_SLOTS);
        for setup in axis_setups(backend) {
            builder = builder.with_absolute_axis(&setup)?;
        }
        let mut device = builder.build()?;
        device.enumerate_dev_nodes_blocking()?;
        // Upload notifications are drained by poll_rumble between loop
        // passes; without O_NONBLOCK that read would stall the daemon.
        enable_nonblocking(&device);
        Ok(Self {
            device: Some(device),
            backend,
            hat_dpad: [false; 4],
            pending: Vec::new(),
            rumble_effects: HashMap::new(),
        })
    }

    /// Drains force-feedback traffic the game produced on this pad, turned
    /// into replay commands. Uploads (EVIOCSFF) only *register* an effect's
    /// motor strengths under its kernel effect id; the playback events that
    /// actually drive the motors arrive as ordinary EV_FF events — value 1
    /// runs the stored effect, value 0 is an explicit stop. Chrome-style
    /// consumers upload with a safety-max length and always stop through a
    /// play event; SDL-style consumers upload with the real length and rely
    /// on the pad's own timer, so the uploaded length stays the safety cap.
    /// Erased effects (haptic close, gone consumer) stop the motors too.
    pub fn poll_rumble(&mut self) -> Vec<RumbleCommand> {
        let Some(device) = self.device.as_mut() else {
            return Vec::new();
        };
        let events: Vec<InputEvent> = match device.fetch_events() {
            Ok(events) => events.collect(),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Vec::new(),
            Err(error) => {
                eprintln!("ira-input: reading virtual pad events failed: {error}");
                return Vec::new();
            }
        };
        let mut commands = Vec::new();
        for event in events {
            match event.destructure() {
                EventSummary::UInput(upload, code, _) if code == UInputCode::UI_FF_UPLOAD => {
                    match device.process_ff_upload(upload) {
                        Ok(mut request) => {
                            request.set_retval(0);
                            if let Some(command) =
                                crate::rumble::rumble_command_from_effect(&request.effect())
                            {
                                self.rumble_effects.insert(request.effect_id(), command);
                            }
                        }
                        Err(error) => {
                            eprintln!("ira-input: answering rumble upload failed: {error}")
                        }
                    }
                }
                EventSummary::UInput(erase, code, _) if code == UInputCode::UI_FF_ERASE => {
                    if let Err(error) = device.process_ff_erase(erase) {
                        eprintln!("ira-input: answering rumble erase failed: {error}");
                    }
                    self.rumble_effects.clear();
                    commands.push(crate::rumble::stop_command());
                }
                EventSummary::ForceFeedback(_, code, value) => {
                    if let Some(command) = playback_command(&self.rumble_effects, code, value) {
                        commands.push(command);
                    }
                }
                _ => {}
            }
        }
        commands
    }

    /// Queues one output event. The kernel write happens in `flush`, once
    /// per loop pass, so a full report batch costs a single syscall.
    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        match event {
            OutputEvent::GamepadButton { button, pressed } => {
                if let Some(input) = self.hat_dpad_event(*button, *pressed) {
                    // Switch Pro and Sony pads report the d-pad only as
                    // hat 0.
                    self.pending.push(input);
                    return Ok(());
                }
                let Some(code) = button_code(self.backend, *button) else {
                    return Ok(());
                };
                self.pending
                    .push(InputEvent::new(EventType::KEY.0, code.0, i32::from(*pressed)));
                if let Some(input) = self.mirrored_hat_event(*button, *pressed) {
                    // The Xbox and DirectInput identities ship the d-pad
                    // keys AND mirror it as hat 0: every controller database
                    // entry for those identities binds the d-pad to the hat
                    // (dpup:h0.1), while games that read keys directly get
                    // the keys.
                    self.pending.push(input);
                }
            }
            OutputEvent::GamepadAxis { axis, value } => {
                let Some(code) = axis_code(self.backend, *axis) else {
                    return Ok(());
                };
                let input = InputEvent::new(EventType::ABSOLUTE.0, code.0, axis_value(*axis, *value));
                self.pending.push(input);
            }
            _ => {}
        }
        Ok(())
    }

    /// Writes the events queued by `emit` as one report. A failed write
    /// drops the batch exactly as the per-event writes dropped the failing
    /// event; the error surfaces so the session can decide to shut down.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let events = std::mem::take(&mut self.pending);
        let Some(device) = self.device.as_mut() else {
            return Ok(());
        };
        device.emit(&events)
    }

    pub fn direct_input_sdl_mapping() -> String {
        format!(
            "{},{},{}",
            direct_input_sdl_guid(),
            DIRECT_INPUT_NAME,
            DIRECT_INPUT_SDL_BINDINGS
        )
    }

    pub fn switch_pro_sdl_mapping() -> String {
        format!(
            "030000007e0500000920000011810000,Nintendo Switch Pro Controller,{}",
            SWITCH_PRO_SDL_BINDINGS
        )
    }

    pub fn dual_shock_4_sdl_mapping() -> String {
        format!(
            "{},{},{}",
            DUAL_SHOCK_4_GUID,
            DUAL_SHOCK_4_NAME,
            sony_sdl_bindings()
        )
    }

    pub fn dual_sense_sdl_mapping() -> String {
        format!(
            "{},{},{}",
            DUAL_SENSE_GUID,
            DUAL_SENSE_NAME,
            sony_sdl_bindings()
        )
    }

    pub fn steam_input_sdl_mapping() -> String {
        format!(
            "{},{},{}",
            STEAM_INPUT_GUID,
            STEAM_INPUT_SDL_NAME,
            STEAM_INPUT_SDL_BINDINGS
        )
    }

    /// Backends whose d-pad is reported as hat 0 movements instead of
    /// BTN_DPAD_* keys (Nintendo Switch Pro and both Sony pads).
    fn hat_dpad_event(&mut self, button: GamepadButton, pressed: bool) -> Option<InputEvent> {
        if !matches!(
            self.backend,
            VirtualGamepadBackend::SwitchPro
                | VirtualGamepadBackend::DualShock4
                | VirtualGamepadBackend::DualSense
        ) {
            return None;
        }
        let (code, value) = self.update_hat(button, pressed)?;
        Some(InputEvent::new(EventType::ABSOLUTE.0, code.0, value))
    }

    /// The hat 0 half mirrored beside the d-pad keys for the backends that
    /// report the d-pad both ways (Xbox, DirectInput, Steam Input).
    fn mirrored_hat_event(&mut self, button: GamepadButton, pressed: bool) -> Option<InputEvent> {
        if !matches!(
            self.backend,
            VirtualGamepadBackend::XInput
                | VirtualGamepadBackend::DirectInput
                | VirtualGamepadBackend::SteamInput
        ) {
            return None;
        }
        let (code, value) = self.update_hat(button, pressed)?;
        Some(InputEvent::new(EventType::ABSOLUTE.0, code.0, value))
    }

    /// Records one d-pad direction in the hat state and returns the axis
    /// event that reflects it.
    fn update_hat(
        &mut self,
        button: GamepadButton,
        pressed: bool,
    ) -> Option<(AbsoluteAxisCode, i32)> {
        let index = match button {
            GamepadButton::DpadUp => 0,
            GamepadButton::DpadDown => 1,
            GamepadButton::DpadLeft => 2,
            GamepadButton::DpadRight => 3,
            _ => return None,
        };
        self.hat_dpad[index] = pressed;
        let horizontal = matches!(button, GamepadButton::DpadLeft | GamepadButton::DpadRight);
        let value = hat_value(self.hat_dpad, horizontal);
        let code = match button {
            GamepadButton::DpadUp | GamepadButton::DpadDown => AbsoluteAxisCode::ABS_HAT0Y,
            _ => AbsoluteAxisCode::ABS_HAT0X,
        };
        Some((code, value))
    }
}

fn hat_value(state: [bool; 4], horizontal: bool) -> i32 {
    let (negative, positive) = if horizontal {
        (state[2], state[3])
    } else {
        (state[0], state[1])
    };
    match (negative, positive) {
        (true, false) => -1,
        (false, true) => 1,
        _ => 0,
    }
}

fn direct_input_sdl_guid() -> String {
    format!(
        "0600{:02x}{:02x}{:02x}{:02x}0000{:02x}{:02x}0000{:02x}{:02x}0000",
        sdl_crc16(DIRECT_INPUT_NAME.as_bytes()) as u8,
        (sdl_crc16(DIRECT_INPUT_NAME.as_bytes()) >> 8) as u8,
        DIRECT_INPUT_VENDOR as u8,
        (DIRECT_INPUT_VENDOR >> 8) as u8,
        DIRECT_INPUT_PRODUCT as u8,
        (DIRECT_INPUT_PRODUCT >> 8) as u8,
        DIRECT_INPUT_VERSION as u8,
        (DIRECT_INPUT_VERSION >> 8) as u8,
    )
}

// SDL3's SDL_CreateJoystickGUID uses this CRC16 for the Linux product name.
fn sdl_crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |crc, byte| {
        let mut input = crc ^ u16::from(*byte);
        let mut value = 0;
        for _ in 0..8 {
            value = if (value ^ input) & 1 != 0 {
                0xa001 ^ (value >> 1)
            } else {
                value >> 1
            };
            input >>= 1;
        }
        value ^ (crc >> 8)
    })
}

/// Best-effort O_NONBLOCK flip so event draining never blocks the daemon.
fn enable_nonblocking(device: &VirtualDevice) {
    use std::os::fd::AsRawFd;
    let fd = device.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
}

fn gamepad_buttons(backend: VirtualGamepadBackend) -> AttributeSet<KeyCode> {
    let mut buttons: AttributeSet<KeyCode> = [
        KeyCode::BTN_SOUTH,
        KeyCode::BTN_EAST,
        KeyCode::BTN_NORTH,
        KeyCode::BTN_WEST,
        KeyCode::BTN_TL,
        KeyCode::BTN_TR,
        KeyCode::BTN_SELECT,
        KeyCode::BTN_START,
        KeyCode::BTN_MODE,
        KeyCode::BTN_THUMBL,
        KeyCode::BTN_THUMBR,
    ]
    .into_iter()
    .collect();
    // The Xbox identities must mirror the real xpad key set, which has no
    // digital trigger keys (triggers are analog only): consumers enumerate
    // buttons by key code and every database entry for those identities
    // binds back/start/guide/stick clicks to slots 6-10, which only line up
    // without BTN_TL2/BTN_TR2 shifting SELECT and everything after it two
    // places down.
    if !matches!(
        backend,
        VirtualGamepadBackend::XInput | VirtualGamepadBackend::SteamInput
    ) {
        buttons.insert(KeyCode::BTN_TL2);
        buttons.insert(KeyCode::BTN_TR2);
    }
    if backend == VirtualGamepadBackend::SwitchPro {
        buttons.insert(KeyCode::BTN_Z);
    } else if sony_layout(backend) {
        // Sony pads report square on BTN_C and keep the d-pad on hat 0;
        // BTN_WEST stays unused so the button indexes match the kernel
        // drivers that SDL's built-in mappings were written against.
        buttons.remove(KeyCode::BTN_WEST);
        buttons.insert(KeyCode::BTN_C);
    } else {
        for code in [
            KeyCode::BTN_DPAD_UP,
            KeyCode::BTN_DPAD_DOWN,
            KeyCode::BTN_DPAD_LEFT,
            KeyCode::BTN_DPAD_RIGHT,
        ] {
            buttons.insert(code);
        }
    }
    if backend == VirtualGamepadBackend::DirectInput {
        for code in [
            KeyCode::BTN_TRIGGER_HAPPY1,
            KeyCode::BTN_TRIGGER_HAPPY2,
            KeyCode::BTN_TRIGGER_HAPPY3,
            KeyCode::BTN_TRIGGER_HAPPY4,
            KeyCode::BTN_TRIGGER_HAPPY5,
            KeyCode::BTN_TRIGGER_HAPPY6,
            KeyCode::BTN_TRIGGER_HAPPY7,
            KeyCode::BTN_TRIGGER_HAPPY8,
        ] {
            buttons.insert(code);
        }
    }
    buttons
}

fn sony_layout(backend: VirtualGamepadBackend) -> bool {
    matches!(
        backend,
        VirtualGamepadBackend::DualShock4 | VirtualGamepadBackend::DualSense
    )
}

fn device_name(backend: VirtualGamepadBackend) -> &'static str {
    match backend {
        VirtualGamepadBackend::XInput => "Ira Virtual Xbox Controller",
        VirtualGamepadBackend::DirectInput => DIRECT_INPUT_NAME,
        VirtualGamepadBackend::SwitchPro => SWITCH_PRO_NAME,
        VirtualGamepadBackend::DualShock4 => DUAL_SHOCK_4_NAME,
        VirtualGamepadBackend::DualSense => DUAL_SENSE_NAME,
        VirtualGamepadBackend::SteamInput => STEAM_INPUT_NAME,
        VirtualGamepadBackend::Dsu => "Ira DSU Controller",
    }
}

fn device_id(backend: VirtualGamepadBackend) -> InputId {
    match backend {
        VirtualGamepadBackend::XInput => InputId::new(
            BusType::BUS_USB,
            VIRTUAL_VENDOR,
            VIRTUAL_PRODUCT,
            VIRTUAL_VERSION,
        ),
        VirtualGamepadBackend::DirectInput => InputId::new(
            BusType::BUS_VIRTUAL,
            DIRECT_INPUT_VENDOR,
            DIRECT_INPUT_PRODUCT,
            DIRECT_INPUT_VERSION,
        ),
        VirtualGamepadBackend::SwitchPro => InputId::new(
            BusType::BUS_USB,
            SWITCH_PRO_VENDOR,
            SWITCH_PRO_PRODUCT,
            SWITCH_PRO_VERSION,
        ),
        VirtualGamepadBackend::DualShock4 => InputId::new(
            BusType::BUS_USB,
            DUAL_SHOCK_4_VENDOR,
            DUAL_SHOCK_4_PRODUCT,
            DUAL_SHOCK_4_VERSION,
        ),
        VirtualGamepadBackend::DualSense => InputId::new(
            BusType::BUS_USB,
            DUAL_SENSE_VENDOR,
            DUAL_SENSE_PRODUCT,
            DUAL_SENSE_VERSION,
        ),
        VirtualGamepadBackend::SteamInput => InputId::new(
            BusType::BUS_USB,
            STEAM_INPUT_VENDOR,
            STEAM_INPUT_PRODUCT,
            STEAM_INPUT_VERSION,
        ),
        VirtualGamepadBackend::Dsu => InputId::new(BusType::BUS_VIRTUAL, 0, 0, 0),
    }
}

fn axis_setups(backend: VirtualGamepadBackend) -> Vec<UinputAbsSetup> {
    let mut setups = vec![
        axis_setup(AbsoluteAxisCode::ABS_X, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_Y, -32768, 32767),
    ];
    if sony_layout(backend) {
        // Kernel Sony layout: right stick on ABS_Z/ABS_RZ (full range),
        // analog triggers on ABS_RX/ABS_RY (0..255).
        setups.extend([
            axis_setup(AbsoluteAxisCode::ABS_Z, -32768, 32767),
            axis_setup(AbsoluteAxisCode::ABS_RZ, -32768, 32767),
            axis_setup(AbsoluteAxisCode::ABS_RX, 0, 255),
            axis_setup(AbsoluteAxisCode::ABS_RY, 0, 255),
        ]);
        setups.extend([
            axis_setup(AbsoluteAxisCode::ABS_HAT0X, -1, 1),
            axis_setup(AbsoluteAxisCode::ABS_HAT0Y, -1, 1),
        ]);
        return setups;
    }
    setups.extend([
        axis_setup(AbsoluteAxisCode::ABS_RX, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_RY, -32768, 32767),
    ]);
    if matches!(
        backend,
        VirtualGamepadBackend::DirectInput
            | VirtualGamepadBackend::XInput
            | VirtualGamepadBackend::SteamInput
    ) {
        // Hat 0 beside the d-pad keys: the databases and auto-mappings for
        // these identities bind the d-pad to the hat.
        setups.extend([
            axis_setup(AbsoluteAxisCode::ABS_HAT0X, -1, 1),
            axis_setup(AbsoluteAxisCode::ABS_HAT0Y, -1, 1),
        ]);
    }
    if backend != VirtualGamepadBackend::SwitchPro {
        setups.extend([
            axis_setup(AbsoluteAxisCode::ABS_Z, 0, 255),
            axis_setup(AbsoluteAxisCode::ABS_RZ, 0, 255),
        ]);
    } else {
        setups.extend([
            axis_setup(AbsoluteAxisCode::ABS_HAT0X, -1, 1),
            axis_setup(AbsoluteAxisCode::ABS_HAT0Y, -1, 1),
        ]);
    }
    setups
}

fn axis_setup(code: AbsoluteAxisCode, minimum: i32, maximum: i32) -> UinputAbsSetup {
    UinputAbsSetup::new(code, AbsInfo::new(0, minimum, maximum, 0, 0, 0))
}

fn button_code(backend: VirtualGamepadBackend, button: GamepadButton) -> Option<KeyCode> {
    Some(match button {
        // Face buttons are positional for every backend (A = south, B =
        // east): a virtual pad must identify exactly like the real
        // controller SDL names after, and Nintendo lettering is purely
        // cosmetic on the hardware.
        GamepadButton::A => KeyCode::BTN_SOUTH,
        GamepadButton::B => KeyCode::BTN_EAST,
        GamepadButton::X if sony_layout(backend) => KeyCode::BTN_C,
        GamepadButton::X => KeyCode::BTN_NORTH,
        GamepadButton::Y if sony_layout(backend) => KeyCode::BTN_NORTH,
        GamepadButton::Y => KeyCode::BTN_WEST,
        GamepadButton::LeftShoulder => KeyCode::BTN_TL,
        GamepadButton::RightShoulder => KeyCode::BTN_TR,
        // The Xbox identities carry no digital trigger keys (analog only);
        // trigger outputs there ride the axes.
        GamepadButton::LeftTrigger
            if !matches!(
                backend,
                VirtualGamepadBackend::XInput | VirtualGamepadBackend::SteamInput
            ) =>
        {
            KeyCode::BTN_TL2
        }
        GamepadButton::RightTrigger
            if !matches!(
                backend,
                VirtualGamepadBackend::XInput | VirtualGamepadBackend::SteamInput
            ) =>
        {
            KeyCode::BTN_TR2
        }
        GamepadButton::Back => KeyCode::BTN_SELECT,
        GamepadButton::Start => KeyCode::BTN_START,
        GamepadButton::Guide => KeyCode::BTN_MODE,
        GamepadButton::LeftStick => KeyCode::BTN_THUMBL,
        GamepadButton::RightStick => KeyCode::BTN_THUMBR,
        GamepadButton::DpadUp
            if backend != VirtualGamepadBackend::SwitchPro && !sony_layout(backend) =>
        {
            KeyCode::BTN_DPAD_UP
        }
        GamepadButton::DpadDown
            if backend != VirtualGamepadBackend::SwitchPro && !sony_layout(backend) =>
        {
            KeyCode::BTN_DPAD_DOWN
        }
        GamepadButton::DpadLeft
            if backend != VirtualGamepadBackend::SwitchPro && !sony_layout(backend) =>
        {
            KeyCode::BTN_DPAD_LEFT
        }
        GamepadButton::DpadRight
            if backend != VirtualGamepadBackend::SwitchPro && !sony_layout(backend) =>
        {
            KeyCode::BTN_DPAD_RIGHT
        }
        GamepadButton::Paddle1 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY1
        }
        GamepadButton::Paddle2 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY2
        }
        GamepadButton::Paddle3 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY3
        }
        GamepadButton::Paddle4 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY4
        }
        GamepadButton::Paddle5 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY5
        }
        GamepadButton::Paddle6 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY6
        }
        GamepadButton::Paddle7 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY7
        }
        GamepadButton::Paddle8 if backend == VirtualGamepadBackend::DirectInput => {
            KeyCode::BTN_TRIGGER_HAPPY8
        }
        _ => return None,
    })
}

fn axis_code(backend: VirtualGamepadBackend, axis: GamepadAxis) -> Option<AbsoluteAxisCode> {
    Some(match axis {
        GamepadAxis::LeftX => AbsoluteAxisCode::ABS_X,
        GamepadAxis::LeftY => AbsoluteAxisCode::ABS_Y,
        GamepadAxis::RightX if sony_layout(backend) => AbsoluteAxisCode::ABS_Z,
        GamepadAxis::RightX => AbsoluteAxisCode::ABS_RX,
        GamepadAxis::RightY if sony_layout(backend) => AbsoluteAxisCode::ABS_RZ,
        GamepadAxis::RightY => AbsoluteAxisCode::ABS_RY,
        GamepadAxis::LeftTrigger if backend != VirtualGamepadBackend::SwitchPro => {
            if sony_layout(backend) {
                AbsoluteAxisCode::ABS_RX
            } else {
                AbsoluteAxisCode::ABS_Z
            }
        }
        GamepadAxis::RightTrigger if backend != VirtualGamepadBackend::SwitchPro => {
            if sony_layout(backend) {
                AbsoluteAxisCode::ABS_RY
            } else {
                AbsoluteAxisCode::ABS_RZ
            }
        }
        _ => return None,
    })
}

fn axis_value(axis: GamepadAxis, value: f32) -> i32 {
    let value = value.clamp(-1.0, 1.0);
    match axis {
        GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger => {
            ((value.max(0.0)) * 255.0).round() as i32
        }
        // The evdev stick range is the XInput one, -32768..32767: the
        // negative half is one step wider, so scaling both sides by 32767
        // would leave -1.0 one short of the axis minimum and every consumer
        // (SDL, Chrome) would read full deflection as -0.9999x. Scale each
        // half by its own width so both endpoints land exactly.
        _ if value < 0.0 => (value * 32768.0).round() as i32,
        _ => (value * 32767.0).round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        axis_code, axis_setups, axis_value, button_code, device_id, device_name, gamepad_buttons,
        hat_value, playback_command, sony_layout, VirtualGamepad, DIRECT_INPUT_NAME,
        DIRECT_INPUT_PRODUCT, DIRECT_INPUT_VENDOR, DIRECT_INPUT_VERSION,
    };
    use crate::rumble::{stop_command, RumbleCommand};
    use crate::VirtualGamepadBackend::{DirectInput, DualSense, DualShock4, SwitchPro, XInput};
    use crate::{GamepadAxis, GamepadButton, OutputEvent};
    use std::collections::HashMap;
    use evdev::{FFEffectCode, InputId, KeyCode};

    #[test]
    fn test_playback_start_replays_the_stored_effect() {
        let mut effects = HashMap::new();
        effects.insert(3, RumbleCommand {
            strong: 10,
            weak: 20,
            duration_ms: 300,
        });
        assert_eq!(
            playback_command(&effects, FFEffectCode(3), 1),
            Some(RumbleCommand {
                strong: 10,
                weak: 20,
                duration_ms: 300
            })
        );
    }

    #[test]
    fn test_playback_stop_stops_regardless_of_known_id() {
        let effects = HashMap::new();
        assert_eq!(
            playback_command(&effects, FFEffectCode(9), 0),
            Some(stop_command())
        );
    }

    #[test]
    fn test_playback_of_unknown_effect_is_ignored() {
        let effects = HashMap::new();
        assert_eq!(playback_command(&effects, FFEffectCode(4), 1), None);
    }

    #[test]
    fn test_emit_queues_and_flush_drains() {
        let mut pad = VirtualGamepad::shadow_only(XInput);
        pad.emit(&OutputEvent::GamepadAxis {
            axis: GamepadAxis::LeftX,
            value: 0.5,
        })
        .unwrap();
        pad.emit(&OutputEvent::GamepadAxis {
            axis: GamepadAxis::LeftX,
            value: 0.5,
        })
        .unwrap();
        assert_eq!(pad.pending.len(), 2, "emit queues instead of writing");
        pad.flush().unwrap();
        assert!(pad.pending.is_empty(), "flush drains the queue");
        // A second flush with nothing queued stays a no-op.
        pad.flush().unwrap();
    }

    #[test]
    fn test_xinput_enumerates_buttons_like_the_real_xpad_driver() {
        // Consumers enumerate buttons by ascending key code and controller
        // database entries for the Xbox identity bind back/start/guide/stick
        // clicks to slots 6..10, written against xpad's 11-key set. The
        // virtual pad must carry exactly those codes so the slots line up.
        let expected: Vec<KeyCode> = [
            KeyCode::BTN_SOUTH,   // b0
            KeyCode::BTN_EAST,    // b1
            KeyCode::BTN_NORTH,   // b2
            KeyCode::BTN_WEST,    // b3
            KeyCode::BTN_TL,      // b4
            KeyCode::BTN_TR,      // b5
            KeyCode::BTN_SELECT,  // b6 (back)
            KeyCode::BTN_START,   // b7
            KeyCode::BTN_MODE,    // b8 (guide)
            KeyCode::BTN_THUMBL,  // b9 (left stick click)
            KeyCode::BTN_THUMBR,  // b10 (right stick click)
        ]
        .into_iter()
        .chain([
            KeyCode::BTN_DPAD_UP,
            KeyCode::BTN_DPAD_DOWN,
            KeyCode::BTN_DPAD_LEFT,
            KeyCode::BTN_DPAD_RIGHT,
        ])
        .collect();
        let mut buttons: Vec<KeyCode> = gamepad_buttons(XInput).iter().collect();
        buttons.sort_by_key(|code| code.0);
        assert_eq!(buttons, expected);
        assert!(!gamepad_buttons(XInput).contains(KeyCode::BTN_TL2));
        assert!(!gamepad_buttons(XInput).contains(KeyCode::BTN_TR2));
        // Same for the Steam Input identity, whose database entries use the
        // same xpad-shaped slot table.
        assert!(!gamepad_buttons(crate::VirtualGamepadBackend::SteamInput)
            .contains(KeyCode::BTN_TL2));
        // Triggers are analog-only there: no digital trigger key events.
        assert_eq!(button_code(XInput, GamepadButton::LeftTrigger), None);
        assert_eq!(button_code(XInput, GamepadButton::RightTrigger), None);
        assert_eq!(
            button_code(
                crate::VirtualGamepadBackend::SteamInput,
                GamepadButton::LeftTrigger
            ),
            None
        );
        // DirectInput pads keep their digital trigger keys.
        assert!(gamepad_buttons(DirectInput).contains(KeyCode::BTN_TL2));
    }

    #[test]
    fn test_button_code_uses_virtual_xbox_positions() {
        assert_eq!(
            button_code(XInput, GamepadButton::X),
            Some(KeyCode::BTN_NORTH)
        );
        assert_eq!(
            button_code(XInput, GamepadButton::Y),
            Some(KeyCode::BTN_WEST)
        );
        assert_eq!(button_code(XInput, GamepadButton::Paddle1), None);
    }

    #[test]
    fn test_switch_pro_matches_sdl_positional_layout() {
        // A real Pro Controller through SDL is positional (south = a, east =
        // b); the virtual pad must identify exactly the same way instead of
        // following Nintendo's printed letters.
        assert_eq!(
            button_code(SwitchPro, GamepadButton::A),
            Some(KeyCode::BTN_SOUTH)
        );
        assert_eq!(
            button_code(SwitchPro, GamepadButton::B),
            Some(KeyCode::BTN_EAST)
        );
        assert_eq!(
            button_code(SwitchPro, GamepadButton::X),
            Some(KeyCode::BTN_NORTH)
        );
        assert_eq!(
            button_code(SwitchPro, GamepadButton::Y),
            Some(KeyCode::BTN_WEST)
        );
        assert_eq!(button_code(SwitchPro, GamepadButton::DpadUp), None);
        assert_eq!(button_code(SwitchPro, GamepadButton::Paddle1), None);
        assert!(gamepad_buttons(SwitchPro).contains(KeyCode::BTN_Z));
        assert!(!gamepad_buttons(SwitchPro).contains(KeyCode::BTN_DPAD_UP));
    }

    #[test]
    fn test_direct_input_maps_all_paddles_to_happy_buttons() {
        assert_eq!(
            button_code(DirectInput, GamepadButton::Paddle1),
            Some(KeyCode::BTN_TRIGGER_HAPPY1)
        );
        assert_eq!(
            button_code(DirectInput, GamepadButton::Paddle8),
            Some(KeyCode::BTN_TRIGGER_HAPPY8)
        );
        let buttons = gamepad_buttons(DirectInput);
        assert!(buttons.contains(KeyCode::BTN_TRIGGER_HAPPY1));
        assert!(buttons.contains(KeyCode::BTN_TRIGGER_HAPPY8));
    }

    #[test]
    fn test_backend_identity_is_stable_and_distinct() {
        assert_eq!(device_name(XInput), "Ira Virtual Xbox Controller");
        assert_eq!(
            device_name(DirectInput),
            "Ira Virtual DirectInput Controller"
        );
        assert_ne!(device_id(XInput), device_id(DirectInput));
        assert_eq!(
            device_id(XInput),
            InputId::new(evdev::BusType::BUS_USB, 0x045e, 0x028e, 0x0114)
        );
        assert_eq!(
            device_id(SwitchPro),
            InputId::new(evdev::BusType::BUS_USB, 0x057e, 0x2009, 0x8111)
        );
        assert_eq!(
            device_name(SwitchPro),
            "Ira Virtual Nintendo Switch Pro Controller"
        );
    }

    #[test]
    fn test_switch_pro_uses_hat_dpad_and_no_analog_triggers() {
        assert_eq!(
            axis_code(SwitchPro, GamepadAxis::LeftX),
            Some(evdev::AbsoluteAxisCode::ABS_X)
        );
        assert_eq!(axis_code(SwitchPro, GamepadAxis::LeftTrigger), None);
        assert!(VirtualGamepad::switch_pro_sdl_mapping()
            .starts_with("030000007e0500000920000011810000,Nintendo Switch Pro Controller"));
    }

    #[test]
    fn test_switch_pro_hat_values_handle_opposite_directions() {
        assert_eq!(hat_value([true, false, false, false], false), -1);
        assert_eq!(hat_value([false, true, false, false], false), 1);
        assert_eq!(hat_value([false, false, true, false], true), -1);
        assert_eq!(hat_value([false, false, true, true], true), 0);
    }

    #[test]
    fn test_sony_backends_use_kernel_layout() {
        for backend in [DualShock4, DualSense] {
            assert!(sony_layout(backend));
            // Square on BTN_C (not BTN_WEST) so button indexes match the
            // kernel drivers SDL's built-in mappings were written against.
            assert_eq!(button_code(backend, GamepadButton::X), Some(KeyCode::BTN_C));
            assert_eq!(
                button_code(backend, GamepadButton::Y),
                Some(KeyCode::BTN_NORTH)
            );
            assert_eq!(
                button_code(backend, GamepadButton::A),
                Some(KeyCode::BTN_SOUTH)
            );
            assert_eq!(
                button_code(backend, GamepadButton::B),
                Some(KeyCode::BTN_EAST)
            );
            assert_eq!(button_code(backend, GamepadButton::DpadUp), None);
            assert_eq!(button_code(backend, GamepadButton::Paddle1), None);
            assert!(!gamepad_buttons(backend).contains(KeyCode::BTN_WEST));
            assert!(!gamepad_buttons(backend).contains(KeyCode::BTN_DPAD_UP));
            assert!(gamepad_buttons(backend).contains(KeyCode::BTN_C));
            // Right stick lives on ABS_Z/ABS_RZ, triggers on ABS_RX/ABS_RY.
            assert_eq!(
                axis_code(backend, GamepadAxis::RightX),
                Some(evdev::AbsoluteAxisCode::ABS_Z)
            );
            assert_eq!(
                axis_code(backend, GamepadAxis::RightY),
                Some(evdev::AbsoluteAxisCode::ABS_RZ)
            );
            assert_eq!(
                axis_code(backend, GamepadAxis::LeftTrigger),
                Some(evdev::AbsoluteAxisCode::ABS_RX)
            );
            assert_eq!(
                axis_code(backend, GamepadAxis::RightTrigger),
                Some(evdev::AbsoluteAxisCode::ABS_RY)
            );
        }
    }

    #[test]
    fn test_sony_identity_matches_hardware() {
        assert_eq!(
            device_id(DualShock4),
            InputId::new(evdev::BusType::BUS_USB, 0x054c, 0x09cc, 0x0001)
        );
        assert_eq!(
            device_id(DualSense),
            InputId::new(evdev::BusType::BUS_USB, 0x054c, 0x0ce6, 0x0111)
        );
        assert_eq!(
            device_name(DualShock4),
            "Sony Interactive Entertainment Wireless Controller"
        );
        assert!(VirtualGamepad::dual_shock_4_sdl_mapping().starts_with(
            "030000004c050000cc09000000010000,Sony Interactive Entertainment Wireless Controller,"
        ));
        assert!(VirtualGamepad::dual_sense_sdl_mapping()
            .starts_with("030000004c050000e60c000011010000,Sony Interactive Entertainment DualSense Wireless Controller,"));
    }

    #[test]
    fn test_direct_input_emits_hat_beside_dpad_keys() {
        let mut pad = VirtualGamepad::shadow_only(DirectInput);
        pad.emit(&OutputEvent::GamepadButton {
            button: GamepadButton::DpadUp,
            pressed: true,
        })
        .unwrap();
        let queued = std::mem::take(&mut pad.pending);
        assert_eq!(
            queued.len(),
            2,
            "the d-up key and its hat 0 movement both ship"
        );
        assert!(
            queued.iter().any(|event| event.event_type() == evdev::EventType::ABSOLUTE
                && event.code() == evdev::AbsoluteAxisCode::ABS_HAT0Y.0
                && event.value() == -1),
            "hat 0 moves up"
        );
        assert!(
            queued.iter().any(|event| event.event_type() == evdev::EventType::KEY
                && event.code() == KeyCode::BTN_DPAD_UP.0
                && event.value() == 1),
            "the d-up key ships beside the hat"
        );
    }

    #[test]
    fn test_xinput_mirrors_the_dpad_as_hat0_for_database_mappings() {
        // Every controller database entry for the Xbox identity binds the
        // d-pad to hat 0 (dpup:h0.1); a keys-only d-pad reads as dead there.
        for backend in [XInput, crate::VirtualGamepadBackend::SteamInput] {
            let mut pad = VirtualGamepad::shadow_only(backend);
            pad.emit(&OutputEvent::GamepadButton {
                button: GamepadButton::DpadLeft,
                pressed: true,
            })
            .unwrap();
            let queued = std::mem::take(&mut pad.pending);
            assert_eq!(
                queued.len(),
                2,
                "{backend:?}: the d-left key and its hat 0 movement both ship"
            );
            assert!(queued.iter().any(|event| event.event_type() == evdev::EventType::KEY
                && event.code() == KeyCode::BTN_DPAD_LEFT.0
                && event.value() == 1));
            assert!(queued.iter().any(|event| event.event_type() == evdev::EventType::ABSOLUTE
                && event.code() == evdev::AbsoluteAxisCode::ABS_HAT0X.0
                && event.value() == -1));
            // The hat axes are declared so consumers discover them.
            let codes: Vec<_> = axis_setups(backend).iter().map(|setup| setup.code()).collect();
            assert!(codes.contains(&evdev::AbsoluteAxisCode::ABS_HAT0X.0));
            assert!(codes.contains(&evdev::AbsoluteAxisCode::ABS_HAT0Y.0));
        }
    }

    #[test]
    fn test_direct_input_sdl_mapping_matches_identity() {
        assert!(VirtualGamepad::direct_input_sdl_mapping()
            .starts_with("0600f799524900000100000001000000,Ira Virtual DirectInput Controller,"));
        assert_eq!(device_name(DirectInput), DIRECT_INPUT_NAME);
        assert_eq!(
            device_id(DirectInput),
            InputId::new(
                evdev::BusType::BUS_VIRTUAL,
                DIRECT_INPUT_VENDOR,
                DIRECT_INPUT_PRODUCT,
                DIRECT_INPUT_VERSION,
            )
        );
    }

    #[test]
    fn test_steam_input_matches_valve_identity_and_layout() {
        use crate::VirtualGamepadBackend::SteamInput;
        // The identity SDL special-cases as the Steam virtual gamepad; the
        // evdev name stays Ira-prefixed so the hub ignores our own pad.
        assert_eq!(device_name(SteamInput), "Ira Virtual Steam Input Controller");
        assert_eq!(
            device_id(SteamInput),
            InputId::new(evdev::BusType::BUS_USB, 0x28de, 0x11ff, 0x0100)
        );
        // XInput-style layout: d-pad keys, analog triggers on ABS_Z/ABS_RZ.
        assert_eq!(
            button_code(SteamInput, GamepadButton::DpadUp),
            Some(KeyCode::BTN_DPAD_UP)
        );
        assert_eq!(button_code(SteamInput, GamepadButton::Paddle1), None);
        assert_eq!(
            axis_code(SteamInput, GamepadAxis::LeftTrigger),
            Some(evdev::AbsoluteAxisCode::ABS_Z)
        );
        assert_eq!(
            device_id(SteamInput),
            InputId::new(evdev::BusType::BUS_USB, 0x28de, 0x11ff, 0x0100)
        );
        // The env mapping must carry the exact GUID the uinput node gets:
        // bus USB, vendor/product/version little-endian, zero CRC.
        let mapping = VirtualGamepad::steam_input_sdl_mapping();
        assert!(mapping.starts_with(
            "03000000de280000ff11000000010000,Steam Virtual Gamepad,a:b0,b:b1"
        ));
        assert!(mapping.contains("dpup:b11,dpdown:b12,dpleft:b13,dpright:b14"));
        assert!(mapping.contains("back:b6,start:b7,guide:b8,leftstick:b9,rightstick:b10"));
        assert!(mapping.contains("lefttrigger:a2,righttrigger:a5"));
    }

    #[test]
    fn test_axis_value_maps_sticks_and_triggers() {
        // Both deflection endpoints must land on the evdev range's own
        // endpoints (-32768..32767) or consumers read full pulls as
        // -0.9999x / +0.9999x.
        assert_eq!(axis_value(GamepadAxis::LeftX, -1.0), -32768);
        assert_eq!(axis_value(GamepadAxis::LeftX, 1.0), 32767);
        assert_eq!(axis_value(GamepadAxis::RightY, 0.0), 0);
        assert_eq!(axis_value(GamepadAxis::LeftTrigger, 0.5), 128);
        assert_eq!(axis_value(GamepadAxis::LeftTrigger, -1.0), 0);
    }

    #[test]
    fn test_direct_input_exposes_the_six_standard_axes() {
        let axes = [
            (GamepadAxis::LeftX, evdev::AbsoluteAxisCode::ABS_X),
            (GamepadAxis::LeftY, evdev::AbsoluteAxisCode::ABS_Y),
            (GamepadAxis::RightX, evdev::AbsoluteAxisCode::ABS_RX),
            (GamepadAxis::RightY, evdev::AbsoluteAxisCode::ABS_RY),
            (GamepadAxis::LeftTrigger, evdev::AbsoluteAxisCode::ABS_Z),
            (GamepadAxis::RightTrigger, evdev::AbsoluteAxisCode::ABS_RZ),
        ];
        for (axis, code) in axes {
            assert_eq!(axis_code(DirectInput, axis), Some(code));
        }
    }
}
