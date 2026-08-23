//! Test bench: create a uhid device with the real Switch Pro Controller
//! identity (057e:2009) and a generic gamepad descriptor, then report what
//! the kernel did with it. Run for a few seconds and watch the output plus
//! /proc/bus/input/devices; hid-nintendo either claims the device (nodes
//! appear, possibly with its own layout) or rejects the descriptor mid-
//! probe (nodes vanish, matching what hid-sony did to the virtual DS4).

use ira_input::{UhidDevice, BUS_USB};

const DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x05, // Usage (Gamepad)
    0xA1, 0x01, // Collection (Application)
    0xA1, 0x00, //   Collection (Physical)
    0x09, 0x30, 0x09, 0x31, 0x09, 0x32, 0x09, 0x35, // X, Y, Z, Rz
    0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x04,
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0xC0, //   End Collection
    0x09, 0x39, //   Usage (Hat switch)
    0x15, 0x00, 0x25, 0x07, 0x35, 0x00, 0x46, 0x3B, 0x01, 0x65, 0x14,
    0x75, 0x04, 0x95, 0x01,
    0x81, 0x42, //     Input (Data, Variable, Null State)
    0x05, 0x09, //   Usage Page (Button)
    0x19, 0x01, 0x29, 0x10, // Buttons 1..16
    0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x10,
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0xC0, // End Collection
];

fn main() {
    let seconds: f32 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(5.0);
    let mut device = match UhidDevice::create(
        "Ira Virtual Switch Pro Probe",
        "",
        DESCRIPTOR,
        BUS_USB,
        0x057e,
        0x2009,
    ) {
        Ok(device) => device,
        Err(error) => {
            eprintln!("switch_pro_probe: create failed: {error}");
            return;
        }
    };
    println!("created 057e:2009; watching kernel reaction");
    let start = std::time::Instant::now();
    while start.elapsed().as_secs_f32() < seconds {
        for event in device.poll().unwrap() {
            println!("  kernel event: {event:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!("done; check /proc/bus/input/devices for surviving nodes");
}
