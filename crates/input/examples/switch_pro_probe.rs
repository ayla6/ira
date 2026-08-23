//! Test bench: drive the virtual *real* Switch Pro Controller and watch
//! whether hid-nintendo completes its handshake. Run for a few seconds and
//! watch stdout plus /proc/bus/input/devices: two input nodes (gamepad and
//! IMU, sharing the controller's serial) mean the kernel driver claimed us.

use ira_input::PadState;
use ira_input::SwitchProUhidDevice;
use std::time::{Duration, Instant};

fn main() {
    let seconds: f32 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(6.0);
    let mut device = match SwitchProUhidDevice::create() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("switch_pro_probe: create failed: {error}");
            return;
        }
    };
    println!("created 057e:2009; driving the hid-nintendo handshake");
    let mut pad = PadState::default();
    if std::env::var("STICK_SWEEP").is_ok() {
        // Held phases so an evdev reader can attribute each extreme to an
        // axis without correlating frequencies.
        let phases =
            [("center", 0.0, 0.0), ("lx +1", 1.0, 0.0), ("lx -1", -1.0, 0.0), ("ly +1", 0.0, 1.0), ("ly -1", 0.0, -1.0)];
        for (title, x, y) in phases {
            println!("phase: {title}");
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
                pad.lx = x;
                pad.ly = y;
                // Upright rest: 1 g down the device Z, zero rotation.
                if let Err(error) = device.tick(&pad, [0.0, 0.0, 1.0], [0.0; 3]) {
                    eprintln!("switch_pro_probe: tick failed: {error}");
                    return;
                }
                std::thread::sleep(Duration::from_millis(4));
            }
        }
        println!("done");
        return;
    }
    let start = Instant::now();
    while start.elapsed().as_secs_f32() < seconds {
        let t = start.elapsed().as_secs_f32();
        pad.lx = (t * 3.0).sin();
        pad.cross = (t * 2.0) as i32 % 2 == 0;
        let jitter = (t * 37.0).sin();
        let accel_g = [(t * 5.0).sin() * 1.2, (t * 5.0).cos() * 1.2, 1.0 + jitter * 0.4];
        let gyro_dps = [t * 90.0, 45.0 + jitter, -30.0 - jitter];
        if let Err(error) = device.tick(&pad, accel_g, gyro_dps) {
            eprintln!("switch_pro_probe: tick failed: {error}");
            return;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    println!("done; check /proc/bus/input/devices for the gamepad and IMU nodes");
}
