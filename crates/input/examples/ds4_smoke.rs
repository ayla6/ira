//! Test bench: create the virtual DualShock4 and stream synthetic states so
//! SDL (or any hidraw reader) can be pointed at it. Prints the hidraw and
//! evdev nodes the kernel created.
//!
//! Usage: ds4_smoke [seconds]

use std::time::{Duration, Instant};

use ira_input::{Ds4UhidDevice, ImuUhidDevice, MotionSample, PadState};

fn main() {
    let seconds: f32 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(3.0);
    let uniq = format!("ira-smoke-{}", std::process::id());
    let mut device = match Ds4UhidDevice::create(&uniq) {
        Ok(device) => device,
        Err(error) => {
            eprintln!("ds4_smoke: failed to create virtual DS4: {error}");
            return;
        }
    };
    let mut imu = match ImuUhidDevice::create(&uniq) {
        Ok(imu) => imu,
        Err(error) => {
            eprintln!("ds4_smoke: failed to create paired IMU: {error}");
            return;
        }
    };
    println!("created {uniq}; waiting for the kernel to settle");
    std::thread::sleep(Duration::from_millis(400));
    for entry in std::fs::read_dir("/sys/class/hidraw").into_iter().flatten() {
        let path = entry.ok().map(|entry| entry.path());
        if let Some(path) = path {
            if let Ok(name) = std::fs::read_to_string(path.join("device/name")) {
                if name.starts_with("Ira Virtual") {
                    println!("hidraw node: {} ({})", path.display(), name.trim());
                }
            }
        }
    }

    let mut pad = PadState::default();
    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed().as_secs_f32() < seconds {
        // Sweep the left stick in a circle and rotate the gyro so a reader
        // sees both controls and sensors move. Constant axes jitter by one
        // LSB like real hardware — the kernel input layer suppresses
        // unchanged values, so a perfectly constant synthetic axis would
        // never emit an event.
        let t = start.elapsed().as_secs_f32();
        pad.lx = (t * 3.0).sin();
        pad.ly = (t * 3.0).cos();
        pad.cross = (t * 2.0) as i32 % 2 == 0;
        let jitter = (t * 37.0).sin();
        let sample = MotionSample {
            gyro_dps: [(t * 90.0).to_radians().to_degrees(), 45.0 + jitter, -30.0 - jitter],
            accel_ms2: [
                (t * 5.0).sin() * 12.0,
                (t * 5.0).cos() * 12.0,
                9.81 + jitter * 6.0,
            ],
            timestamp_us: start.elapsed().as_micros() as u64,
        };
        if let Err(error) = device.send_state(&pad, &sample) {
            eprintln!("ds4_smoke: send failed: {error}");
            return;
        }
        if let Err(error) = imu.send_sample(
            [
                sample.accel_ms2[0] / 9.80665,
                sample.accel_ms2[1] / 9.80665,
                sample.accel_ms2[2] / 9.80665,
            ],
            sample.gyro_dps,
        ) {
            eprintln!("ds4_smoke: imu send failed: {error}");
            return;
        }
        frames += 1;
        std::thread::sleep(Duration::from_millis(4));
    }
    println!("sent {frames} reports over {:.1}s", seconds);
}
