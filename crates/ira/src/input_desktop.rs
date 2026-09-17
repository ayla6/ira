//! The app's side of the desktop-default controller behaviour. While Ira is
//! open it holds a client connection to the input daemon and keeps a
//! standing wish on file: an idle controller runs its default layout
//! through the daemon instead of going silent. A controller whose settings
//! page says Disabled opts out, and a game launch always supersedes the
//! wish — the daemon resumes it when the last game ends.
//!
//! The thread also re-evaluates on a slow cadence so hotplugs and settings
//! saves apply without wiring GTK into the daemon protocol.

use std::time::Duration;

use ira_config::load_config;
use ira_config::Config;
use ira_input::{discover_gamepads, DeviceInfo};
use ira_models::ControllerInputMode;

/// How often the wish is re-evaluated (config file, hotplug).
const TICK: Duration = Duration::from_secs(2);
/// How often the daemon's event stream is drained. Game output lines are
/// broadcast to every client, so a slow reader could back up the daemon's
/// broadcast and stall its loop; drain far more often than that.
const DRAIN: Duration = Duration::from_millis(250);

/// Starts the background thread; failures are logged, never fatal — the
/// desktop behaviour is a default, not a requirement.
pub fn start(save_dir: String) {
    let result = std::thread::Builder::new()
        .name("ira-input-desktop".to_string())
        .spawn(move || run(save_dir));
    if let Err(error) = result {
        eprintln!("Failed to start the desktop input thread: {error}");
    }
}

/// What should currently be running for the connected controller.
#[derive(Debug, Clone, PartialEq)]
struct Desired {
    enabled: bool,
    profile: Option<String>,
}

fn run(save_dir: String) {
    let calibration = ira_input::calibration_store_path(&save_dir);
    let mut client: Option<ira_launcher::input_daemon::DaemonClient> = None;
    let mut sent: Option<Desired> = None;
    let mut next_tick = std::time::Instant::now();
    loop {
        if client.is_none() {
            client = ira_launcher::input_daemon::desktop_client().ok();
            if client.is_none() {
                std::thread::sleep(TICK);
                continue;
            }
            // A fresh daemon knows nothing about the previous wish.
            sent = None;
            next_tick = std::time::Instant::now();
        }
        if std::time::Instant::now() >= next_tick {
            next_tick = std::time::Instant::now() + TICK;
            // A broken config file falls back to defaults inside
            // load_config, which would disable the desktop behaviour; that
            // is the honest reading of an unreadable configuration.
            let desired = desired_for(&load_config(), &save_dir);
            if sent.as_ref() != Some(&desired) {
                let delivered = client
                    .as_mut()
                    .map(|client| {
                        ira_launcher::input_daemon::send_desktop_default(
                            client,
                            desired.enabled,
                            desired.profile.as_deref(),
                            Some(calibration.to_string_lossy().as_ref()),
                        )
                    })
                    .unwrap_or_else(|| Err("no daemon connection".to_string()));
                match delivered {
                    Ok(()) => sent = Some(desired),
                    Err(error) => {
                        eprintln!("ira-input: desktop wish failed; will reconnect: {error}");
                        client = None;
                        continue;
                    }
                }
            }
        }
        // Drain the daemon's broadcasts so the socket never backs up; the
        // short wait keeps the reconnect and tick cadence responsive.
        let idle = client
            .as_mut()
            .map(|client| client.wait_event_timeout(DRAIN, |_| {}).is_ok())
            .unwrap_or(false);
        if !idle {
            client = None;
        }
    }
}

/// Resolves the connected controller's desktop behaviour: managed by
/// default, native when its controller settings say Disabled. The wish
/// applies to the pad the hub actually manages — the first in discovery
/// order — so a second controller's Enabled setting must not drag the
/// first (Disabled) one into a remap it was exempted from.
fn desired_for(config: &Config, save_dir: &str) -> Desired {
    let mut devices = discover_gamepads();
    devices.sort_by(|left, right| left.path.cmp(&right.path));
    desired_for_pads(config, save_dir, &devices)
}

fn desired_for_pads(config: &Config, save_dir: &str, devices: &[DeviceInfo]) -> Desired {
    match devices.first() {
        Some(device) => {
            desired_for_device(config, save_dir, device)
                .unwrap_or(Desired {
                    enabled: false,
                    profile: None,
                })
        }
        // Nothing connected (or everything disabled): the daemon releases
        // the pad, and its idle timer may retire it until Ira needs it
        // again.
        None => Desired {
            enabled: false,
            profile: None,
        },
    }
}

fn desired_for_device(config: &Config, save_dir: &str, device: &DeviceInfo) -> Option<Desired> {
    let key = Config::controller_key(device.vendor, device.product);
    let entry = config.controller_defaults.get(&key);
    let enabled = entry
        .map(|entry| entry.mode != ControllerInputMode::Disabled)
        .unwrap_or(true);
    if !enabled {
        return None;
    }
    let configured = entry.map(|entry| entry.profile.clone()).unwrap_or_default();
    let configured = (!configured.is_empty())
        .then(|| std::path::PathBuf::from(&configured))
        .filter(|path| path.is_file());
    let profile = configured
        .or_else(|| crate::ui::find_controller_default_profile(save_dir, &key))
        .map(|path| path.to_string_lossy().into_owned());
    Some(Desired { enabled: true, profile })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vendor: u16, product: u16) -> DeviceInfo {
        DeviceInfo {
            path: "/dev/input/event0".into(),
            name: "Test Pad".to_string(),
            vendor,
            product,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        }
    }

    #[test]
    fn test_desired_for_device_defaults_to_managed_without_an_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::default();
        let desired =
            desired_for_device(&config, tmp.path().to_str().unwrap(), &device(0x2dc8, 0x6012))
                .expect("a controller with no entry is managed by default");
        assert!(desired.enabled);
    }

    #[test]
    fn test_desired_for_device_disabled_entry_opts_out() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.controller_defaults.insert(
            "2dc8:6012".to_string(),
            ira_config::ControllerInputConfig {
                mode: ControllerInputMode::Disabled,
                profile: String::new(),
            },
        );
        assert_eq!(
            desired_for_device(&config, tmp.path().to_str().unwrap(), &device(0x2dc8, 0x6012)),
            None,
            "an explicitly disabled controller keeps native desktop input"
        );
    }

    #[test]
    fn test_desired_for_pads_follows_the_hubs_pad_not_the_first_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.controller_defaults.insert(
            "2dc8:6012".to_string(),
            ira_config::ControllerInputConfig {
                mode: ControllerInputMode::Disabled,
                profile: String::new(),
            },
        );
        // The hub manages the FIRST pad; the second pad being enabled must
        // not drag the disabled first pad into a remap.
        let pads = [device(0x2dc8, 0x6012), device(0x054c, 0x0ce6)];
        assert!(
            !desired_for_pads(&config, tmp.path().to_str().unwrap(), &pads).enabled,
            "the managed pad's Disabled setting must win the wish"
        );
    }

    #[test]
    fn test_desired_for_pads_enables_the_managed_pad() {
        let tmp = tempfile::tempdir().unwrap();
        let pads = [device(0x2dc8, 0x6012)];
        assert!(desired_for_pads(
            &Config::default(),
            tmp.path().to_str().unwrap(),
            &pads
        )
        .enabled);
    }

    #[test]
    fn test_desired_for_pads_releases_with_nothing_connected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!desired_for_pads(&Config::default(), tmp.path().to_str().unwrap(), &[]).enabled);
    }
}
