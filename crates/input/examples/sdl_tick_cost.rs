//! Measures the daemon's per-tick SDL sequence (pump + sensor update + two
//! data queries) against the real controller, at tick cadence. A rising
//! per-second cost means SDL-side work grows unboundedly — the event queue
//! nobody drains being the usual suspect.
//!
//! ```sh
//! cargo build -p ira-input --example sdl_tick_cost
//! target/debug/examples/sdl_tick_cost [seconds]
//! ```

use ira_input::{discover_gamepads, Sdl3SensorBackend};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|argument| argument.parse().ok())
        .unwrap_or(20);
    let device = discover_gamepads().into_iter().next();
    let Some(device) = device else {
        eprintln!("no gamepad found");
        return;
    };
    eprintln!(
        "pad: {} ({:04x}:{:04x})",
        device.name, device.vendor, device.product
    );
    let mut sensor = match Sdl3SensorBackend::open(&device) {
        Ok(Some(sensor)) => sensor,
        Ok(None) => {
            eprintln!("no SDL sensor for this pad; nothing to measure");
            return;
        }
        Err(error) => {
            eprintln!("SDL open failed: {error}");
            return;
        }
    };

    let started = Instant::now();
    let mut window = Instant::now();
    let (mut calls, mut samples, mut busy) = (0u64, 0u64, Duration::ZERO);
    while started.elapsed() < Duration::from_secs(seconds) {
        let t0 = Instant::now();
        match sensor.read(now_us()) {
            Ok(Some(_)) => samples += 1,
            Ok(None) => {}
            Err(error) => {
                eprintln!("sensor read failed: {error}");
                return;
            }
        }
        busy += t0.elapsed();
        calls += 1;
        let elapsed = window.elapsed();
        if elapsed >= Duration::from_secs(1) {
            eprintln!(
                "calls={calls:5} (target 1000)  fresh={samples:5}  avg read = {:>6.1} us   elapsed {} s",
                busy.as_micros() as f64 / calls as f64,
                started.elapsed().as_secs()
            );
            calls = 0;
            samples = 0;
            busy = Duration::ZERO;
            window = Instant::now();
        }
        // Daemon cadence: 1 kHz tick, 4 ms default before the estimator locks.
        std::thread::sleep(Duration::from_millis(1).saturating_sub(t0.elapsed()));
    }
}
