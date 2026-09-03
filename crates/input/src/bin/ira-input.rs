use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ira_input::daemon::{parse_arguments, run_session};
use ira_input::{discover_gamepads, discover_sdl_gamepads, Sdl3SensorBackend};

fn main() {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("ira-input: {error}");
            eprintln!(
                "usage: ira-input [--vdf-import IN.vdf OUT.json] | --list | [--device PATH] [--profile PATH] [--steam-app-id ID] [--trace] -- COMMAND"
            );
            std::process::exit(2);
        }
    };
    if arguments.list {
        list_devices();
        return;
    }
    if arguments.probe_sensors {
        probe_sensors();
        return;
    }
    if arguments.daemon {
        match ira_input::daemon::server::run_daemon() {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("ira-input: {error}");
                std::process::exit(1);
            }
        }
    }
    if let Some((input, output)) = arguments.vdf_import.clone() {
        match import_vdf_file(&input, &output) {
            Ok(report) => {
                println!("imported {} to {}", input.display(), output.display());
                for warning in &report.warnings {
                    eprintln!("ira-input: import warning: {warning}");
                }
            }
            Err(error) => {
                eprintln!("ira-input: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    // The daemon is the default session host; an explicit --no-daemon (or a
    // command-less mapping session) stays in-process.
    if !arguments.no_daemon && !arguments.command.is_empty() {
        match ira_input::daemon::run_via_daemon(&arguments) {
            Ok(code) => std::process::exit(code),
            Err(reason) => eprintln!(
                "ira-input: daemon unavailable ({reason}); running the session in-process"
            ),
        }
    }
    match run_session(arguments) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ira-input: {error}");
            std::process::exit(1);
        }
    }
}

fn list_devices() {
    let devices = discover_gamepads();
    if devices.is_empty() {
        println!("No gamepads found.");
        return;
    }
    for device in devices {
        println!(
            "{}: {} (vendor={:04x}, product={:04x}, version={:04x}, evdev_gyro={})",
            device.path.display(),
            device.name,
            device.vendor,
            device.product,
            device.version,
            device.has_evdev_gyro
        );
    }
}

fn probe_sensors() {
    match discover_sdl_gamepads() {
        Ok(gamepads) => {
            println!("SDL3 gamepads: {}", gamepads.len());
            for gamepad in gamepads {
                println!(
                    "  id={} name={:?} path={:?} vendor={:04x} product={:04x} gyro={} accel={}",
                    gamepad.id,
                    gamepad.name,
                    gamepad.path,
                    gamepad.vendor,
                    gamepad.product,
                    gamepad.has_gyro,
                    gamepad.has_accelerometer
                );
            }
        }
        Err(error) => println!("SDL3 enumeration failed: {error}"),
    }
    let devices = discover_gamepads();
    if devices.is_empty() {
        println!("No gamepads found.");
        return;
    }
    for device in devices {
        println!("{}: {}", device.path.display(), device.name);
        if ira_input::EvdevImu::open(&device).is_some() {
            println!("  kernel IMU: available");
        }
        match Sdl3SensorBackend::open(&device) {
            Ok(Some(mut sensor)) => {
                println!("  SDL3 gyro: available");
                for _ in 0..5 {
                    thread::sleep(Duration::from_millis(20));
                    match sensor.read(now_us()) {
                        Ok(Some(sample)) => println!(
                            "  sample: x={:.5} y={:.5} z={:.5} accel={:?}",
                            sample.gyro[0], sample.gyro[1], sample.gyro[2], sample.accel
                        ),
                        Ok(None) => println!("  sample: unavailable"),
                        Err(error) => println!("  sample error: {error}"),
                    }
                }
            }
            Ok(None) => println!("  SDL3 gyro: unavailable"),
            Err(error) => println!("  SDL3 gyro probe failed: {error}"),
        }
    }
}

/// Convert a Steam VDF layout into Ira's JSON profile format.
fn import_vdf_file(input: &Path, output: &Path) -> Result<ira_input::ImportReport, String> {
    let (profile, report) =
        ira_input::import_vdf_file(input).map_err(|error| format!("import failed: {error}"))?;
    let json = serde_json::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    std::fs::write(output, json + "\n").map_err(|error| error.to_string())?;
    Ok(report)
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}
