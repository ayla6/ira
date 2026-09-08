//! Native motion passthrough: a companion uinput node exposing the physical
//! controller's accelerometer and gyroscope as standard Linux sensor axes,
//! next to the virtual gamepad.
//!
//! SDL 2.24+/3.x pairs an "accelerometer class" evdev node with a gamepad by
//! comparing the two nodes' EVIOCGUNIQ strings (uinput cannot set UNIQ, so
//! both are empty and any single pad/sensor pair matches); emulators then see
//! hardware-style motion without cemuhook. Axis conventions follow the
//! kernel's HID sensor providers: acceleration on ABS_X/Y/Z and angular
//! velocity on ABS_RX/RY/RZ, where SDL recovers physical units by dividing
//! each raw value by the axis' `resolution` (milli-g and milli-dps here, so
//! a one-degree tilt moves the accelerometer by ~17 report units instead of
//! rounding a 0.02 g change to zero) before applying its own SI conversions.

use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, PropType, UinputAbsSetup};

use crate::VirtualGamepadBackend;

const MOTION_NAME: &str = "Ira Virtual Motion Sensors";
/// Full-scale accelerometer range, in the report's milli-g units (±8 g).
const ACCEL_RANGE_MG: i32 = 8_000;
/// Full-scale gyroscope range, in the report's milli-dps units (±2048 dps).
const GYRO_RANGE_MDPS: i32 = 2_048_000;
/// Report resolution: SDL divides each raw value by its axis resolution
/// to recover g and degrees per second, so 1000 units per physical unit
/// keeps three decimal places instead of collapsing the sensor onto
/// whole integers.
const UNITS_PER_G: i32 = 1_000;
const UNITS_PER_DPS: i32 = 1_000;

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
    /// `accel` in g. Emitted scaled to the report's milli-units.
    pub fn emit_sample(&mut self, gyro: [f32; 3], accel: [f32; 3]) -> io::Result<()> {
        const RAD_TO_DEG: f32 = 180.0 / std::f32::consts::PI;
        let events = [
            axis_event(
                AbsoluteAxisCode::ABS_X,
                accel[0] * UNITS_PER_G as f32,
                -ACCEL_RANGE_MG,
                ACCEL_RANGE_MG,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_Y,
                accel[1] * UNITS_PER_G as f32,
                -ACCEL_RANGE_MG,
                ACCEL_RANGE_MG,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_Z,
                accel[2] * UNITS_PER_G as f32,
                -ACCEL_RANGE_MG,
                ACCEL_RANGE_MG,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_RX,
                gyro[0] * RAD_TO_DEG * UNITS_PER_DPS as f32,
                -GYRO_RANGE_MDPS,
                GYRO_RANGE_MDPS,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_RY,
                gyro[1] * RAD_TO_DEG * UNITS_PER_DPS as f32,
                -GYRO_RANGE_MDPS,
                GYRO_RANGE_MDPS,
            ),
            axis_event(
                AbsoluteAxisCode::ABS_RZ,
                gyro[2] * RAD_TO_DEG * UNITS_PER_DPS as f32,
                -GYRO_RANGE_MDPS,
                GYRO_RANGE_MDPS,
            ),
        ];
        self.device.emit(&events)
    }
}

fn axis_setups() -> Vec<UinputAbsSetup> {
    let mut setups = Vec::new();
    let mut push = |code: AbsoluteAxisCode, min: i32, max: i32, resolution: i32| {
        setups.push(UinputAbsSetup::new(
            code,
            AbsInfo::new(0, min, max, 0, 0, resolution),
        ));
    };
    for code in [
        AbsoluteAxisCode::ABS_X,
        AbsoluteAxisCode::ABS_Y,
        AbsoluteAxisCode::ABS_Z,
    ] {
        push(code, -ACCEL_RANGE_MG, ACCEL_RANGE_MG, UNITS_PER_G);
    }
    for code in [
        AbsoluteAxisCode::ABS_RX,
        AbsoluteAxisCode::ABS_RY,
        AbsoluteAxisCode::ABS_RZ,
    ] {
        push(code, -GYRO_RANGE_MDPS, GYRO_RANGE_MDPS, UNITS_PER_DPS);
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
    fn test_axis_event_scales_to_milli_units_and_clamps() {
        let event = axis_event(AbsoluteAxisCode::ABS_RY, 91_400.0, -2_048_000, 2_048_000);
        assert_eq!(event.value(), 91_400);
        let clamped = axis_event(AbsoluteAxisCode::ABS_X, 12_500.0, -8_000, 8_000);
        assert_eq!(clamped.value(), 8_000);
        let negative = axis_event(AbsoluteAxisCode::ABS_Z, -100_000.0, -8_000, 8_000);
        assert_eq!(negative.value(), -8_000);
    }

    #[test]
    fn test_axis_event_holds_sub_g_tilt_precision() {
        // A 0.02 g change (about one degree of tilt), already scaled to
        // milli-g, must survive the wire: whole-g reports rounded it to
        // zero.
        let event = axis_event(AbsoluteAxisCode::ABS_X, 20.0, -8_000, 8_000);
        assert_eq!(event.value(), 20);
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
            assert_eq!(setup.absinfo().resolution(), 1_000);
            if accel_codes.contains(&setup.code()) {
                assert_eq!(setup.absinfo().minimum(), -8_000);
                assert_eq!(setup.absinfo().maximum(), 8_000);
            } else {
                assert!(gyro_codes.contains(&setup.code()));
                assert_eq!(setup.absinfo().minimum(), -2_048_000);
                assert_eq!(setup.absinfo().maximum(), 2_048_000);
            }
        }
    }
}
