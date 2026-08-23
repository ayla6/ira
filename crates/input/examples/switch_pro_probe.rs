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
