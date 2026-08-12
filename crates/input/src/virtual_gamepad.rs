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
const DIRECT_INPUT_SDL_BINDINGS: &str = "a:b0,b:b1,x:b3,y:b2,leftshoulder:b4,rightshoulder:b5,lefttrigger:a2,righttrigger:a5,back:b8,start:b9,guide:b10,leftstick:b11,rightstick:b12,dpup:b13,dpdown:b14,dpleft:b15,dpright:b16,leftx:a0,lefty:a1,rightx:a3,righty:a4,paddle1:b17,paddle2:b18,paddle3:b19,paddle4:b20";

pub struct VirtualGamepad {
    device: VirtualDevice,
    backend: VirtualGamepadBackend,
}

impl VirtualGamepad {
    pub fn create() -> io::Result<Self> {
        Self::create_for_backend(VirtualGamepadBackend::XInput)
    }

    pub fn create_for_backend(backend: VirtualGamepadBackend) -> io::Result<Self> {
        let buttons = gamepad_buttons(backend);
        let mut builder = VirtualDevice::builder()?
            .name(device_name(backend))
            .input_id(device_id(backend))
            .with_keys(&buttons)?;
        for setup in axis_setups() {
            builder = builder.with_absolute_axis(&setup)?;
        }
        let mut device = builder.build()?;
        device.enumerate_dev_nodes_blocking()?;
        Ok(Self { device, backend })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        let input = match event {
            OutputEvent::GamepadButton { button, pressed } => {
                let Some(code) = button_code(self.backend, *button) else {
                    return Ok(());
                };
                InputEvent::new(EventType::KEY.0, code.0, i32::from(*pressed))
            }
            OutputEvent::GamepadAxis { axis, value } => {
                let Some(code) = axis_code(*axis) else {
                    return Ok(());
                };
                InputEvent::new(EventType::ABSOLUTE.0, code.0, axis_value(*axis, *value))
            }
            _ => return Ok(()),
        };
        self.device.emit(&[input])
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
        KeyCode::BTN_DPAD_UP,
        KeyCode::BTN_DPAD_DOWN,
        KeyCode::BTN_DPAD_LEFT,
        KeyCode::BTN_DPAD_RIGHT,
    ]
    .into_iter()
    .collect();
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

fn device_name(backend: VirtualGamepadBackend) -> &'static str {
    match backend {
        VirtualGamepadBackend::XInput => "Ira Virtual Xbox Controller",
        VirtualGamepadBackend::DirectInput => DIRECT_INPUT_NAME,
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
    }
}

fn axis_setups() -> [UinputAbsSetup; 6] {
    [
        axis_setup(AbsoluteAxisCode::ABS_X, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_Y, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_RX, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_RY, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_Z, 0, 255),
        axis_setup(AbsoluteAxisCode::ABS_RZ, 0, 255),
    ]
}

fn axis_setup(code: AbsoluteAxisCode, minimum: i32, maximum: i32) -> UinputAbsSetup {
    UinputAbsSetup::new(code, AbsInfo::new(0, minimum, maximum, 0, 0, 0))
}

fn button_code(backend: VirtualGamepadBackend, button: GamepadButton) -> Option<KeyCode> {
    Some(match button {
        GamepadButton::A => KeyCode::BTN_SOUTH,
        GamepadButton::B => KeyCode::BTN_EAST,
        GamepadButton::X => KeyCode::BTN_WEST,
        GamepadButton::Y => KeyCode::BTN_NORTH,
        GamepadButton::LeftShoulder => KeyCode::BTN_TL,
        GamepadButton::RightShoulder => KeyCode::BTN_TR,
        GamepadButton::LeftTrigger => KeyCode::BTN_TL2,
        GamepadButton::RightTrigger => KeyCode::BTN_TR2,
        GamepadButton::Back => KeyCode::BTN_SELECT,
        GamepadButton::Start => KeyCode::BTN_START,
        GamepadButton::Guide => KeyCode::BTN_MODE,
        GamepadButton::LeftStick => KeyCode::BTN_THUMBL,
        GamepadButton::RightStick => KeyCode::BTN_THUMBR,
        GamepadButton::DpadUp => KeyCode::BTN_DPAD_UP,
        GamepadButton::DpadDown => KeyCode::BTN_DPAD_DOWN,
        GamepadButton::DpadLeft => KeyCode::BTN_DPAD_LEFT,
        GamepadButton::DpadRight => KeyCode::BTN_DPAD_RIGHT,
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

fn axis_code(axis: GamepadAxis) -> Option<AbsoluteAxisCode> {
    Some(match axis {
        GamepadAxis::LeftX => AbsoluteAxisCode::ABS_X,
        GamepadAxis::LeftY => AbsoluteAxisCode::ABS_Y,
        GamepadAxis::RightX => AbsoluteAxisCode::ABS_RX,
        GamepadAxis::RightY => AbsoluteAxisCode::ABS_RY,
        GamepadAxis::LeftTrigger => AbsoluteAxisCode::ABS_Z,
        GamepadAxis::RightTrigger => AbsoluteAxisCode::ABS_RZ,
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
        axis_code, axis_value, button_code, device_id, device_name, gamepad_buttons,
        VirtualGamepad, DIRECT_INPUT_NAME, DIRECT_INPUT_PRODUCT, DIRECT_INPUT_VENDOR,
        DIRECT_INPUT_VERSION,
    };
    use crate::VirtualGamepadBackend::{DirectInput, XInput};
    use crate::{GamepadAxis, GamepadButton};
    use evdev::{InputId, KeyCode};

    #[test]
    fn test_button_code_uses_virtual_xbox_positions() {
        assert_eq!(
            button_code(XInput, GamepadButton::X),
            Some(KeyCode::BTN_WEST)
        );
        assert_eq!(
            button_code(XInput, GamepadButton::Y),
            Some(KeyCode::BTN_NORTH)
        );
        assert_eq!(button_code(XInput, GamepadButton::Paddle1), None);
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
            assert_eq!(axis_code(axis), Some(code));
        }
    }
}
