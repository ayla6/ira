//! Measures the daemon's CPU against a live virtual Switch Pro: creates
//! the uhid twin, spawns `ira-input` on its evdev node with a real profile,
//! samples the daemon's CPU for a few seconds, and prints the verdict.
//!
//! ```sh
//! cargo build -p ira-input --example daemon_cpu_probe --features nothing
//! sudo target/debug/examples/daemon_cpu_probe
//! ```

use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    if !std::path::Path::new("/dev/uhid").exists() {
        eprintln!("no /dev/uhid here");
        return;
    }
    // The twin is created by the probe test infrastructure; here we reuse
    // the shipped example twin instead: fake_pad creates a uinput pad the
    // daemon can bind.
    eprintln!("spawning fake_pad to provide a physical pad");
    let mut pad = Command::new("target/debug/examples/fake_pad")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("build fake_pad first: cargo build -p ira-input --examples");
    std::thread::sleep(Duration::from_millis(800));

    let devices = std::fs::read_dir("/dev/input")
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect::<Vec<_>>();
    let mut pad_path = None;
    for path in devices {
        if let Ok(device) = evdev::Device::open(&path) {
            if device
                .supported_keys()
                .is_some_and(|keys| keys.contains(evdev::KeyCode::BTN_SOUTH))
                && device
                    .name()
                    .is_some_and(|name| name.contains("Test Bench Pad"))
            {
                pad_path = Some(path);
            }
        }
    }
    let Some(pad_path) = pad_path else {
        eprintln!("fake pad node not found");
        let _ = pad.kill();
        let _ = pad.wait();
        return;
    };
    eprintln!("pad at {}", pad_path.display());

    // Minimal real profile: backend xinput, gyro off, rumble on.
    let profile = r#"{
        "name": "cpu-probe",
        "backend": "xinput",
        "action_sets": [{"name": "Default", "inputs": [
            {"source": {"button": "a"},
             "activators": [{"kind": "full_press",
                             "outputs": [{"gamepad_button": "a"}]}]}
        ]}]
    }"#;
    let profile_path = std::path::Path::new("/tmp/ira_cpu_probe_profile.json");
    std::fs::write(profile_path, profile).unwrap();

    let mut daemon = Command::new("target/debug/ira-input")
        .arg("--device")
        .arg(&pad_path)
        .arg("--profile")
        .arg(profile_path)
        .arg("--")
        .arg("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let daemon_pid = daemon.id();

    std::thread::sleep(Duration::from_secs(1));
    let samples = 3;
    for i in 0..samples {
        std::thread::sleep(Duration::from_secs(2));
        let stat = std::fs::read_to_string(format!("/proc/{daemon_pid}/stat"))
            .expect("daemon died");
        let utime: u64 = stat.split_whitespace().nth(13).unwrap().parse().unwrap();
        let stime: u64 = stat.split_whitespace().nth(14).unwrap().parse().unwrap();
        eprintln!("sample {}: utime+stime ticks = {}/{}", i + 1, utime, stime);
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = pad.kill();
    let _ = pad.wait();
    eprintln!("done — ticks grow ~100/s per full core; 500/s = 5 cores hot");
}
