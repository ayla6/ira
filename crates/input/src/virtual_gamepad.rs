use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    UinputAbsSetup,
};

use crate::{GamepadAxis, GamepadButton, OutputEvent};

const VIRTUAL_VENDOR: u16 = 0x045e;
const VIRTUAL_PRODUCT: u16 = 0x028e;
const VIRTUAL_VERSION: u16 = 0x0114;

pub struct VirtualGamepad {
    device: VirtualDevice,
}

impl VirtualGamepad {
    pub fn create() -> io::Result<Self> {
        let buttons = gamepad_buttons();
        let mut builder = VirtualDevice::builder()?
            .name("Ira Virtual Xbox Controller")
            .input_id(InputId::new(
                BusType::BUS_USB,
                VIRTUAL_VENDOR,
                VIRTUAL_PRODUCT,
                VIRTUAL_VERSION,
            ))
            .with_keys(&buttons)?;
        for setup in axis_setups() {
            builder = builder.with_absolute_axis(&setup)?;
        }
        let mut device = builder.build()?;
        device.enumerate_dev_nodes_blocking()?;
        Ok(Self { device })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        let input = match event {
            OutputEvent::GamepadButton { button, pressed } => {
                let Some(code) = button_code(*button) else {
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
}

fn gamepad_buttons() -> AttributeSet<KeyCode> {
    [
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
    .collect()
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

fn button_code(button: GamepadButton) -> Option<KeyCode> {
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
    use super::{axis_value, button_code};
    use crate::{GamepadAxis, GamepadButton};
    use evdev::KeyCode;

    #[test]
    fn test_button_code_uses_virtual_xbox_positions() {
        assert_eq!(button_code(GamepadButton::X), Some(KeyCode::BTN_WEST));
        assert_eq!(button_code(GamepadButton::Y), Some(KeyCode::BTN_NORTH));
        assert_eq!(button_code(GamepadButton::Paddle1), None);
    }

    #[test]
    fn test_axis_value_maps_sticks_and_triggers() {
        assert_eq!(axis_value(GamepadAxis::LeftX, -1.0), -32767);
        assert_eq!(axis_value(GamepadAxis::LeftX, 1.0), 32767);
        assert_eq!(axis_value(GamepadAxis::LeftTrigger, 0.5), 128);
        assert_eq!(axis_value(GamepadAxis::LeftTrigger, -1.0), 0);
    }
}
