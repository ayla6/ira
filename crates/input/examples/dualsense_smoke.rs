//! Test bench: create the virtual DualSense and stream synthetic states so
//! SDL (or any hidraw reader) can be pointed at it. SDL should classify it
//! as a PS5 pad with gyro and accel via its third-party capability probe.
//!
//! Usage: dualsense_smoke [seconds]

use std::time::{Duration, Instant};

use ira_input::{DualsenseUhidDevice, MotionSample, PadState};

fn main() {
    let seconds: f32 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0);
    let uniq = format!("ira-smoke-{}", std::process::id());
    let mut device = match DualsenseUhidDevice::create(&uniq) {
        Ok(device) => device,
        Err(error) => {
            eprintln!("dualsense_smoke: failed to create virtual DualSense: {error}");
            return;
        }
    };
    println!("created {uniq}; streaming states");
    let mut pad = PadState::default();
    let start = Instant::now();
    let mut reports = 0u32;
    while start.elapsed().as_secs_f32() < seconds {
        let t = start.elapsed().as_secs_f32();
        pad.lx = (t * 3.0).sin();
        pad.cross = (t * 2.0) as i32 % 2 == 0;
        let sample = MotionSample {
            accel_ms2: [0.0, 0.0, 9.80665],
            // Slow yaw rotation so sensor readers see real motion.
            gyro_dps: [0.0, (t * 20.0).sin() * 45.0, 0.0],
            timestamp_us: start.elapsed().as_micros() as u64,
        };
        if device.send_state(&pad, &sample).is_ok() {
            reports += 1;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    println!("sent {reports} reports over {:.1}s", seconds);
}
