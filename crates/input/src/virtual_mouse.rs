use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode, RelativeAxisCode};

use crate::{MouseAxis, MouseButton, OutputEvent};

const VIRTUAL_VENDOR: u16 = 0x1a1a;
const VIRTUAL_PRODUCT: u16 = 0x0002;
const VIRTUAL_VERSION: u16 = 0x0001;

pub struct VirtualMouse {
    device: VirtualDevice,
    /// Fractional motion accumulated between integer uinput reports.
    pending_x: f32,
    pending_y: f32,
    pending_wheel: f32,
    pending_wheel_x: f32,
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
            RelativeAxisCode::REL_HWHEEL,
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
            pending_x: 0.0,
            pending_y: 0.0,
            pending_wheel: 0.0,
            pending_wheel_x: 0.0,
        })
    }

    pub fn emit(&mut self, event: &OutputEvent) -> io::Result<()> {
        match event {
            OutputEvent::MouseButton { button, pressed } => {
                let code = button_code(*button);
                let input = InputEvent::new(EventType::KEY.0, code.0, i32::from(*pressed));
                self.device.emit(&[input])
            }
            OutputEvent::MouseMotion { axis, value } => {
                self.accumulate(*axis, *value);
                Ok(())
            }
            OutputEvent::WheelClick { axis, amount } => {
                self.accumulate(*axis, *amount as f32);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Write accumulated motion as one uinput report so X/Y/wheel deltas that
    /// arrived in the same tick reach the game as a single synchronized
    /// batch instead of one SYN_REPORT per axis.
    pub fn flush(&mut self) -> io::Result<()> {
        let mut reports = Vec::with_capacity(4);
        let x = take_delta(&mut self.pending_x);
        if x != 0 {
            reports.push(relative(RelativeAxisCode::REL_X, x));
        }
        let y = take_delta(&mut self.pending_y);
        if y != 0 {
            reports.push(relative(RelativeAxisCode::REL_Y, y));
        }
        let wheel = take_delta(&mut self.pending_wheel);
        if wheel != 0 {
            reports.push(relative(RelativeAxisCode::REL_WHEEL, wheel));
        }
        let wheel_x = take_delta(&mut self.pending_wheel_x);
        if wheel_x != 0 {
            reports.push(relative(RelativeAxisCode::REL_HWHEEL, wheel_x));
        }
        if reports.is_empty() {
            return Ok(());
        }
        self.device.emit(&reports)
    }

    fn accumulate(&mut self, axis: MouseAxis, value: f32) {
        match axis {
            MouseAxis::X => self.pending_x += value,
            MouseAxis::Y => self.pending_y += value,
            MouseAxis::Wheel => self.pending_wheel += value,
            MouseAxis::WheelX => self.pending_wheel_x += value,
        }
    }
}

fn relative(code: RelativeAxisCode, delta: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, code.0, delta)
}

fn take_delta(remainder: &mut f32) -> i32 {
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
        remainder += 0.4;
        assert_eq!(take_delta(&mut remainder), 0);
        remainder += 0.7;
        assert_eq!(take_delta(&mut remainder), 1);
        assert!((remainder - 0.1).abs() < 0.001);
    }
}
