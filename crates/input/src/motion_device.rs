//! Native motion passthrough: a companion uinput node exposing the physical
//! controller's accelerometer and gyroscope as standard Linux sensor axes,
//! next to the virtual gamepad.
//!
//! SDL 2.24+/3.x pairs an "accelerometer class" evdev node with a gamepad by
//! comparing the two nodes' EVIOCGUNIQ strings (uinput cannot set UNIQ, so
//! both are empty and any single pad/sensor pair matches); emulators then see
//! hardware-style motion without cemuhook. Axis conventions follow the
//! kernel's HID sensor providers: acceleration on ABS_X/Y/Z in g units and
//! angular velocity on ABS_RX/RY/RZ in degrees per second. SDL converts those
//! raw values by dividing each axis' `resolution` (1 here, since the values
//! are emitted already scaled) before applying its own SI conversions.

use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, PropType, UinputAbsSetup,
};

use crate::VirtualGamepadBackend;

const MOTION_NAME: &str = "Ira Virtual Motion Sensors";
/// Full-scale accelerometer range in g.
const ACCEL_RANGE_G: i32 = 8;
/// Full-scale gyroscope range in degrees per second.
const GYRO_RANGE_DPS: i32 = 2048;

pub struct VirtualMotionSensor {
    device: VirtualDevice,
}

impl VirtualMotionSensor {
    pub fn create(_backend: VirtualGamepadBackend) -> io::Result<Self> {
        let props: AttributeSet<PropType> = [PropType::ACCELEROMETER].into_iter().collect();
        let mut builder = VirtualDevice::builder()?
            .name(MOTION_NAME)
            .with_properties(&props)?;
        // No key bits at all: together with the accelerometer property this
        // keeps udev and SDL from classifying the node as a joystick.
        for setup in axis_setups() {
            builder = builder.with_absolute_axis(&setup)?;
        }
        let mut device = builder.build()?;
        device.enumerate_dev_nodes_blocking()?;
        Ok(Self { device })
    }

    /// Forward one raw sample: `gyro` in rad/s (our sensor pipeline units),
    /// `accel` in g. Emitted unscaled on the wire as deg/s and g.
    pub fn emit_sample(&mut self, gyro: [f32; 3], accel: [f32; 3]) -> io::Result<()> {
        const RAD_TO_DEG: f32 = 180.0 / std::f32::consts::PI;
        let events = [
            axis_event(AbsoluteAxisCode::ABS_X, accel[0], -ACCEL_RANGE_G, ACCEL_RANGE_G),
            axis_event(AbsoluteAxisCode::ABS_Y, accel[1], -ACCEL_RANGE_G, ACCEL_RANGE_G),
            axis_event(AbsoluteAxisCode::ABS_Z, accel[2], -ACCEL_RANGE_G, ACCEL_RANGE_G),
            axis_event(
                AbsoluteAxisCode::ABS_RX,
                gyro[0] * RAD_TO_DEG,
                -GYRO_RANGE_DPS,
                GYRO_RANGE_DPS,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_RY,
                gyro[1] * RAD_TO_DEG,
                -GYRO_RANGE_DPS,
                GYRO_RANGE_DPS,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_RZ,
                gyro[2] * RAD_TO_DEG,
                -GYRO_RANGE_DPS,
                GYRO_RANGE_DPS,
            ),
        ];
        self.device.emit(&events)
    }
}

fn axis_setups() -> Vec<UinputAbsSetup> {
    let mut setups = Vec::new();
    let mut push = |code: AbsoluteAxisCode, min: i32, max: i32| {
        setups.push(UinputAbsSetup::new(
            code,
            // Resolution 1: SDL derives physical units from value/resolution.
            AbsInfo::new(0, min, max, 0, 0, 1),
        ));
    };
    for code in [
        AbsoluteAxisCode::ABS_X,
        AbsoluteAxisCode::ABS_Y,
        AbsoluteAxisCode::ABS_Z,
    ] {
        push(code, -ACCEL_RANGE_G, ACCEL_RANGE_G);
    }
    for code in [
        AbsoluteAxisCode::ABS_RX,
        AbsoluteAxisCode::ABS_RY,
        AbsoluteAxisCode::ABS_RZ,
    ] {
        push(code, -GYRO_RANGE_DPS, GYRO_RANGE_DPS);
    }
    setups
}

fn axis_event(code: AbsoluteAxisCode, value: f32, min: i32, max: i32) -> InputEvent {
    let clamped = (value.round() as i32).clamp(min, max);
    InputEvent::new(evdev::EventType::ABSOLUTE.0, code.0, clamped)
}

#[cfg(test)]
mod tests {
    use super::{axis_event, axis_setups};
    use evdev::AbsoluteAxisCode;

    #[test]
    fn test_axis_event_rounds_and_clamps_to_range() {
        let event = axis_event(AbsoluteAxisCode::ABS_RY, 91.4, -2048, 2048);
        assert_eq!(event.value(), 91);
        let clamped = axis_event(AbsoluteAxisCode::ABS_X, 12.5, -8, 8);
        assert_eq!(clamped.value(), 8);
        let negative = axis_event(AbsoluteAxisCode::ABS_Z, -100.0, -8, 8);
        assert_eq!(negative.value(), -8);
    }

    #[test]
    fn test_axis_setups_cover_accel_and_gyro_with_unit_resolution() {
        let setups = axis_setups();
        assert_eq!(setups.len(), 6);
        let accel_codes = [
            AbsoluteAxisCode::ABS_X.0,
            AbsoluteAxisCode::ABS_Y.0,
            AbsoluteAxisCode::ABS_Z.0,
        ];
        let gyro_codes = [
            AbsoluteAxisCode::ABS_RX.0,
            AbsoluteAxisCode::ABS_RY.0,
            AbsoluteAxisCode::ABS_RZ.0,
        ];
        for setup in &setups {
            assert_eq!(setup.absinfo().resolution(), 1);
            if accel_codes.contains(&setup.code()) {
                assert_eq!(setup.absinfo().minimum(), -8);
                assert_eq!(setup.absinfo().maximum(), 8);
            } else {
                assert!(gyro_codes.contains(&setup.code()));
                assert_eq!(setup.absinfo().minimum(), -2048);
                assert_eq!(setup.absinfo().maximum(), 2048);
            }
        }
    }
}
