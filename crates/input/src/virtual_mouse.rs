use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode, RelativeAxisCode};

use crate::{MouseAxis, MouseButton, OutputEvent};

const VIRTUAL_VENDOR: u16 = 0x1a1a;
const VIRTUAL_PRODUCT: u16 = 0x0002;
const VIRTUAL_VERSION: u16 = 0x0001;

pub struct VirtualMouse {
    device: VirtualDevice,
    fractional_x: f32,
    fractional_y: f32,
    fractional_wheel: f32,
}

impl VirtualMouse {
    pub fn create() -> io::Result<Self> {
        let buttons: AttributeSet<KeyCode> = [
            KeyCode::BTN_LEFT,
            KeyCode::BTN_RIGHT,
            KeyCode::BTN_MIDDLE,
            KeyCode::BTN_SIDE,
            KeyCode::BTN_EXTRA,
        ]
        .into_iter()
        .collect();
        let axes: AttributeSet<RelativeAxisCode> = [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
        ]
        .into_iter()
        .collect();
        let device = VirtualDevice::builder()?
            .name("Ira Virtual Mouse")
            .input_id(InputId::new(
                BusType::BUS_VIRTUAL,
                VIRTUAL_VENDOR,
                VIRTUAL_PRODUCT,
                VIRTUAL_VERSION,
            ))
            .with_keys(&buttons)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self {
            device,
            fractional_x: 0.0,
            fractional_y: 0.0,
            fractional_wheel: 0.0,
        })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        match event {
            OutputEvent::MouseButton { button, pressed } => {
                let code = button_code(*button);
                let input = InputEvent::new(EventType::KEY.0, code.0, i32::from(*pressed));
                self.device.emit(&[input])
            }
            OutputEvent::MouseMotion { axis, value } => self.emit_motion(*axis, *value),
            _ => Ok(()),
        }
    }

    fn emit_motion(&mut self, axis: MouseAxis, value: f32) -> io::Result<()> {
        let (code, delta) = match axis {
            MouseAxis::X => (
                RelativeAxisCode::REL_X,
                take_delta(&mut self.fractional_x, value),
            ),
            MouseAxis::Y => (
                RelativeAxisCode::REL_Y,
                take_delta(&mut self.fractional_y, value),
            ),
            MouseAxis::Wheel => (
                RelativeAxisCode::REL_WHEEL,
                take_delta(&mut self.fractional_wheel, value),
            ),
        };
        if delta == 0 {
            return Ok(());
        }
        self.device
            .emit(&[InputEvent::new(EventType::RELATIVE.0, code.0, delta)])
    }
}

fn take_delta(remainder: &mut f32, value: f32) -> i32 {
    *remainder += value;
    let delta = remainder.trunc() as i32;
    *remainder -= delta as f32;
    delta
}

fn button_code(button: MouseButton) -> KeyCode {
    match button {
        MouseButton::Left => KeyCode::BTN_LEFT,
        MouseButton::Right => KeyCode::BTN_RIGHT,
        MouseButton::Middle => KeyCode::BTN_MIDDLE,
        MouseButton::Side => KeyCode::BTN_SIDE,
        MouseButton::Extra => KeyCode::BTN_EXTRA,
    }
}

#[cfg(test)]
mod tests {
    use super::take_delta;

    #[test]
    fn test_take_delta_preserves_fractional_motion() {
        let mut remainder = 0.0;
        assert_eq!(take_delta(&mut remainder, 0.4), 0);
        assert_eq!(take_delta(&mut remainder, 0.7), 1);
        assert!((remainder - 0.1).abs() < 0.001);
    }
}
