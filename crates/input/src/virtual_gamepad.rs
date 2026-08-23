use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    UinputAbsSetup,
};

use crate::{GamepadAxis, GamepadButton, OutputEvent, VirtualGamepadBackend};

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

fn sony_sdl_bindings() -> &'static str {
    "a:b0,b:b1,x:b2,y:b3,back:b8,start:b9,guide:b12,leftstick:b10,rightstick:b11,leftshoulder:b4,rightshoulder:b5,dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,leftx:a0,lefty:a1,rightx:a2,righty:a5,lefttrigger:a3,righttrigger:a4,misc1:b13,platform:Linux"
}

pub struct VirtualGamepad {
    /// `None` for the DSU backend: it creates no kernel device and exists
    /// only so the output pipeline has a uniform sink; the real carrier is
    /// the cemuhook stream.
    device: Option<VirtualDevice>,
    backend: VirtualGamepadBackend,
    hat_dpad: [bool; 4],
}

impl VirtualGamepad {
    pub fn create() -> io::Result<Self> {
        Self::create_for_backend(VirtualGamepadBackend::XInput)
    }

    pub fn create_for_backend(backend: VirtualGamepadBackend) -> io::Result<Self> {
        if backend == VirtualGamepadBackend::Dsu {
            return Ok(Self {
                device: None,
                backend,
                hat_dpad: [false; 4],
            });
        }
        let buttons = gamepad_buttons(backend);
        let mut builder = VirtualDevice::builder()?
            .name(device_name(backend))
            .input_id(device_id(backend))
            .with_keys(&buttons)?;
        for setup in axis_setups(backend) {
            builder = builder.with_absolute_axis(&setup)?;
        }
        let mut device = builder.build()?;
        device.enumerate_dev_nodes_blocking()?;
        Ok(Self {
            device: Some(device),
            backend,
            hat_dpad: [false; 4],
        })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        let input = match event {
            OutputEvent::GamepadButton { button, pressed } => {
                if let Some(input) = self.hat_dpad_event(*button, *pressed) {
                    input
                } else {
                    let Some(code) = button_code(self.backend, *button) else {
                        return Ok(());
                    };
                    InputEvent::new(EventType::KEY.0, code.0, i32::from(*pressed))
                }
            }
            OutputEvent::GamepadAxis { axis, value } => {
                let Some(code) = axis_code(self.backend, *axis) else {
                    return Ok(());
                };
                InputEvent::new(EventType::ABSOLUTE.0, code.0, axis_value(*axis, *value))
            }
            _ => return Ok(()),
        };
        let Some(device) = self.device.as_mut() else {
            return Ok(());
        };
        device.emit(&[input])
    }

    pub fn emit_all(&mut self, events: &[OutputEvent]) -> io::Result<()> {
        for event in events {
            self.emit(event)?;
        }
        Ok(())
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
            GamepadButton::DpadLeft | GamepadButton::DpadRight => AbsoluteAxisCode::ABS_HAT0X,
            _ => unreachable!(),
        };
        Some(InputEvent::new(EventType::ABSOLUTE.0, code.0, value))
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

fn gamepad_buttons(backend: VirtualGamepadBackend) -> AttributeSet<KeyCode> {
    let mut buttons: AttributeSet<KeyCode> = [
        KeyCode::BTN_SOUTH,
        KeyCode::BTN_EAST,
        KeyCode::BTN_NORTH,
        KeyCode::BTN_WEST,
        KeyCode::BTN_TL,
        KeyCode::BTN_TR,
        KeyCode::BTN_TL2,
        KeyCode::BTN_TR2,
        KeyCode::BTN_SELECT,
        KeyCode::BTN_START,
        KeyCode::BTN_MODE,
        KeyCode::BTN_THUMBL,
        KeyCode::BTN_THUMBR,
    ]
    .into_iter()
    .collect();
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
        GamepadButton::A if backend == VirtualGamepadBackend::SwitchPro => KeyCode::BTN_EAST,
        GamepadButton::A => KeyCode::BTN_SOUTH,
        GamepadButton::B if backend == VirtualGamepadBackend::SwitchPro => KeyCode::BTN_SOUTH,
        GamepadButton::B => KeyCode::BTN_EAST,
        GamepadButton::X if sony_layout(backend) => KeyCode::BTN_C,
        GamepadButton::X => KeyCode::BTN_NORTH,
        GamepadButton::Y if sony_layout(backend) => KeyCode::BTN_NORTH,
        GamepadButton::Y => KeyCode::BTN_WEST,
        GamepadButton::LeftShoulder => KeyCode::BTN_TL,
        GamepadButton::RightShoulder => KeyCode::BTN_TR,
        GamepadButton::LeftTrigger => KeyCode::BTN_TL2,
        GamepadButton::RightTrigger => KeyCode::BTN_TR2,
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
        _ => (value * 32767.0).round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        axis_code, axis_value, button_code, device_id, device_name, gamepad_buttons, hat_value,
        sony_layout, VirtualGamepad, DIRECT_INPUT_NAME, DIRECT_INPUT_PRODUCT, DIRECT_INPUT_VENDOR,
        DIRECT_INPUT_VERSION,
    };
    use crate::VirtualGamepadBackend::{DirectInput, DualSense, DualShock4, SwitchPro, XInput};
    use crate::{GamepadAxis, GamepadButton};
    use evdev::{InputId, KeyCode};

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
    fn test_switch_pro_uses_nintendo_button_positions() {
        assert_eq!(
            button_code(SwitchPro, GamepadButton::A),
            Some(KeyCode::BTN_EAST)
        );
        assert_eq!(
            button_code(SwitchPro, GamepadButton::B),
            Some(KeyCode::BTN_SOUTH)
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
            assert_eq!(
                button_code(backend, GamepadButton::X),
                Some(KeyCode::BTN_C)
            );
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
    fn test_axis_value_maps_sticks_and_triggers() {
        assert_eq!(axis_value(GamepadAxis::LeftX, -1.0), -32767);
        assert_eq!(axis_value(GamepadAxis::LeftX, 1.0), 32767);
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
