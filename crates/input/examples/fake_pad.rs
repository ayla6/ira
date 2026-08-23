//! Test bench: a fake physical gamepad driven by stdin lines, for exercising
//! the real daemon end to end without hardware.
//!
//! Commands (one per line):
//!   ltp / ltr          left trigger key down / up (BTN_TL2)
//!   rtp / rtr          right trigger key down / up (BTN_TR2)
//!   lt <0-255>         ABS_Z raw value
//!   rt <0-255>         ABS_RZ raw value
//!   sticks <x> <y> <rx> <ry>  stick axes in raw -32768..32767 units
//!   jitter             resting trigger noise sweep on ABS_Z/ABS_RZ
//!
//! Prints its evdev node paths on startup; feed them to ira-input --device.

use std::io::{BufRead, Write};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, UinputAbsSetup,
};

const NAME: &str = "Test Bench Pad";

fn main() -> std::io::Result<()> {
    let mut builder = VirtualDevice::builder()?.name(NAME);
    let keys: AttributeSet<KeyCode> = [
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
    builder = builder.with_keys(&keys)?;
    for setup in axis_setups() {
        builder = builder.with_absolute_axis(&setup)?;
    }
    let mut device = builder.build()?;
    let nodes = device.enumerate_dev_nodes_blocking()?;
    for node in nodes.flatten() {
        println!("node {}", node.display());
    }
    println!("ready");
    std::io::stdout().flush()?;

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        match line.trim().split_once(' ') {
            Some(("lt", value)) => emit_axis(&mut device, AbsoluteAxisCode::ABS_Z, parse(value)),
            Some(("rt", value)) => emit_axis(&mut device, AbsoluteAxisCode::ABS_RZ, parse(value)),
            Some(("sticks", rest)) => {
                let values: Vec<i32> = rest.split_whitespace().map(parse).collect();
                if values.len() == 4 {
                    for (code, value) in [
                        (AbsoluteAxisCode::ABS_X, values[0]),
                        (AbsoluteAxisCode::ABS_Y, values[1]),
                        (AbsoluteAxisCode::ABS_RX, values[2]),
                        (AbsoluteAxisCode::ABS_RY, values[3]),
                    ] {
                        emit_axis(&mut device, code, value);
                    }
                }
            }
            _ => match line.trim() {
                "ltp" | "ltr" => {
                    emit_key(&mut device, KeyCode::BTN_TL2, line.trim() == "ltp")?;
                }
                "rtp" | "rtr" => {
                    emit_key(&mut device, KeyCode::BTN_TR2, line.trim() == "rtp")?;
                }
                "jitter" => {
                    for value in [0, 1, 2, 1, 0] {
                        emit_axis(&mut device, AbsoluteAxisCode::ABS_Z, value);
                        emit_axis(&mut device, AbsoluteAxisCode::ABS_RZ, value);
                    }
                }
                "quit" => break,
                "" => {}
                other => eprintln!("unknown command: {other}"),
            },
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}

fn parse(raw: &str) -> i32 {
    raw.parse().unwrap_or(0)
}

fn axis_setup(code: AbsoluteAxisCode, min: i32, max: i32) -> UinputAbsSetup {
    UinputAbsSetup::new(code, AbsInfo::new(0, min, max, 0, 0, 1))
}

fn axis_setups() -> Vec<UinputAbsSetup> {
    vec![
        axis_setup(AbsoluteAxisCode::ABS_X, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_Y, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_RX, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_RY, -32768, 32767),
        axis_setup(AbsoluteAxisCode::ABS_Z, 0, 255),
        axis_setup(AbsoluteAxisCode::ABS_RZ, 0, 255),
        axis_setup(AbsoluteAxisCode::ABS_HAT0X, -1, 1),
        axis_setup(AbsoluteAxisCode::ABS_HAT0Y, -1, 1),
    ]
}

fn emit_axis(device: &mut VirtualDevice, code: AbsoluteAxisCode, value: i32) {
    let _ = device.emit(&[InputEvent::new(EventType::ABSOLUTE.0, code.0, value)]);
}

fn emit_key(device: &mut VirtualDevice, code: KeyCode, pressed: bool) -> std::io::Result<()> {
    // VirtualDevice::emit appends SYN_REPORT itself.
    device.emit(&[InputEvent::new(
        EventType::KEY.0,
        code.0,
        i32::from(pressed),
    )])
}
