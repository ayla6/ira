//! Root-only probe: creates the virtual twins over /dev/uhid and asserts
//! their kernel-generated evdev nodes expose the controls the wire layout
//! promises — the acceptance test the descriptors never had. Without a
//! writable /dev/uhid it prints a note and passes trivially; run it in the
//! dev container with:
//!
//! ```sh
//! cargo test -p ira-input --test uhid_twin_probe --no-run
//! sudo target/debug/deps/uhid_twin_probe-* --ignored --nocapture
//! ```

use ira_input::{Ds4UhidDevice, DualsenseUhidDevice, SwitchProUhidDevice};
use std::collections::HashSet;
use std::path::PathBuf;

fn uhid_available() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uhid")
        .is_ok()
}

/// Where the host's live evdev nodes are. The distrobox bind-mounts each
/// /dev/input/eventNN that existed at container start, so newly created
/// twins would be invisible there — the host's full /dev shows them.
fn input_dir() -> PathBuf {
    if std::path::Path::new("/run/host/dev/input").is_dir() {
        PathBuf::from("/run/host/dev/input")
    } else {
        PathBuf::from("/dev/input")
    }
}

/// Paths of every evdev node currently on the machine.
fn evdev_snapshot() -> HashSet<PathBuf> {
    let mut paths = HashSet::new();
    let Ok(entries) = std::fs::read_dir(input_dir()) else {
        return paths;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("event") {
            paths.insert(entry.path());
        }
    }
    paths
}

/// Waits for udev to settle and opens every evdev node that appeared.
/// Freshly created nodes race udev's ownership handover, so a first open
/// can hit EPERM; one delayed retry absorbs that. The full window is
/// always awaited — the host's live input tree produces unrelated new
/// nodes at any moment, and breaking early on those misses the twins.
fn new_devices(before: &HashSet<PathBuf>) -> Vec<evdev::Device> {
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut pending: Vec<PathBuf> = evdev_snapshot()
        .difference(before)
        .cloned()
        .collect();
    let mut devices = Vec::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        pending.retain(|path| match evdev::Device::open(path) {
            Ok(device) => {
                devices.push(device);
                false
            }
            Err(error) => {
                eprintln!("  open {} failed: {error}", path.display());
                true
            }
        });
        if pending.is_empty() {
            break;
        }
    }
    devices
}

fn assert_gamepad_controls(name: &str, device: &evdev::Device) {
    let axes = device.supported_absolute_axes().expect("absolute axes");
    for axis in [
        evdev::AbsoluteAxisCode::ABS_X,
        evdev::AbsoluteAxisCode::ABS_Y,
        evdev::AbsoluteAxisCode::ABS_Z,
        evdev::AbsoluteAxisCode::ABS_RZ,
        evdev::AbsoluteAxisCode::ABS_RX,
        evdev::AbsoluteAxisCode::ABS_RY,
        evdev::AbsoluteAxisCode::ABS_HAT0X,
    ] {
        assert!(
            axes.contains(axis),
            "{name}: missing axis {axis:?} on {}",
            device.name().unwrap_or_default()
        );
    }
    let buttons = device.supported_keys().expect("keys");
    for button in [
        evdev::KeyCode::BTN_SOUTH,
        evdev::KeyCode::BTN_EAST,
        evdev::KeyCode::BTN_NORTH,
        evdev::KeyCode::BTN_WEST,
        evdev::KeyCode::BTN_TL,
        evdev::KeyCode::BTN_TR,
        evdev::KeyCode::BTN_SELECT,
        evdev::KeyCode::BTN_START,
        evdev::KeyCode::BTN_MODE,
        evdev::KeyCode::BTN_THUMBL,
        evdev::KeyCode::BTN_THUMBR,
    ] {
        assert!(
            buttons.contains(button),
            "{name}: missing button {button:?} on {}",
            device.name().unwrap_or_default()
        );
    }
}

#[test]
#[ignore]
fn uhid_twins_expose_full_gamepad_controls() {
    if !uhid_available() {
        eprintln!("skipping: /dev/uhid is not writable");
        return;
    }
    let before = evdev_snapshot();

    // UHID_CREATE2 answers asynchronously — the kernel's HID core parses
    // the descriptor during its probe and kills the device on failure. The
    // evdev-node assertions below are the acceptance check.
    let ds4 = Ds4UhidDevice::create("ira-probe-ds4");
    let dualsense = DualsenseUhidDevice::create("ira-probe-ds");
    let switch_pro = SwitchProUhidDevice::create("ira-probe-switch-pro");
    for (label, created) in [
        ("DS4", ds4.is_ok()),
        ("DualSense", dualsense.is_ok()),
        ("Switch Pro", switch_pro.is_ok()),
    ] {
        eprintln!(
            "{label} twin creation: {}",
            if created { "ok" } else { "FAILED" }
        );
    }

    let devices = new_devices(&before);
    for device in &devices {
        eprintln!(
            "twin node: {} [bus vendor:product {:04x}:{:04x}]",
            device.name().unwrap_or_default(),
            device.input_id().vendor(),
            device.input_id().product()
        );
    }

    let ds4_twin = devices
        .iter()
        .find(|device| device.input_id().vendor() == 0x0f0d && device.input_id().product() == 0x00ee)
        .expect("DS4 twin evdev node must exist");
    assert_gamepad_controls("DS4", ds4_twin);
    let dualsense_twin = devices
        .iter()
        .find(|device| device.input_id().vendor() == 0x0f0d && device.input_id().product() == 0x0163)
        .expect("DualSense twin evdev node must exist");
    assert_gamepad_controls("DualSense", dualsense_twin);
    // hid-nintendo registers its nodes only after its USB handshake
    // completes, which needs the daemon answering subcommands — the probe
    // does not run a tick loop, so absence here is expected, not a
    // regression. When a node does show up it must have face buttons.
    if let Some(switch_pro_twin) = devices.iter().find(|device| {
        device.input_id().vendor() == 0x057e && device.input_id().product() == 0x2009
    }) {
        let buttons = switch_pro_twin.supported_keys().expect("keys");
        assert!(
            buttons.contains(evdev::KeyCode::BTN_SOUTH),
            "Switch Pro twin has no face buttons"
        );
    } else {
        eprintln!("Switch Pro twin not registered (handshake needs a tick loop) — ok");
    }
}

/// The first NEW evdev node with this input id that is the gamepad itself:
/// hid-nintendo also registers an IMU companion whose node must be skipped.
fn twin_node(before: &HashSet<PathBuf>, vendor: u16, product: u16) -> Option<(PathBuf, evdev::Device)> {
    let now = evdev_snapshot();
    if now.len() <= before.len() {
        return None;
    }
    let mut paths: Vec<PathBuf> = now.difference(before).cloned().collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| Some((path.clone(), evdev::Device::open(path).ok()?)))
        .find(|(_, device)| {
            device.input_id().vendor() == vendor
                && device.input_id().product() == product
                && device
                    .supported_keys()
                    .is_some_and(|keys| keys.contains(evdev::KeyCode::BTN_SOUTH))
        })
}

/// The kernel HID driver bound to a 057e:2009 device, if any. Empty also
/// covers environments that mask /sys (distrobox) — callers there fall back
/// to the FF capability as binding proof.
fn switch_pro_hid_driver() -> String {
    let Ok(entries) = std::fs::read_dir("/sys/class/hid") else {
        return String::new();
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .to_uppercase()
            .contains("057E:2009")
        {
            continue;
        }
        if let Ok(link) = std::fs::read_link(entry.path().join("driver")) {
            if let Some(name) = link.file_name() {
                return name.to_string_lossy().to_string();
            }
        }
    }
    String::new()
}

/// End-to-end behind "rumble doesn't work in Switch Pro mode": hid-nintendo
/// only takes the twin (and only registers force feedback on its pad) when
/// the connect conversation is answered briskly enough to finish the probe.
/// Drive the same servicing pass the daemon now runs every iteration and
/// require the driver claim plus an FF-capable pad afterwards.
#[test]
#[ignore]
fn uhid_switch_pro_handshake_earns_a_rumble_capable_pad() {
    if !uhid_available() {
        eprintln!("skipping: /dev/uhid is not writable");
        return;
    }
    let before = evdev_snapshot();
    let mut switch_pro =
        SwitchProUhidDevice::create("ira-probe-switch-pro").expect("Switch Pro twin creation");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let (_, device) = loop {
        switch_pro.service().expect("virtual Switch Pro stopped");
        match twin_node(&before, 0x057e, 0x2009) {
            Some(found) => break found,
            None => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "hid-nintendo never registered the virtual pro controller"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    let name = device.name().unwrap_or_default();
    eprintln!("registered twin pad: {name}");
    let keys = device.supported_keys().expect("keys");
    assert!(keys.contains(evdev::KeyCode::BTN_SOUTH), "{name}: no face buttons");
    assert!(keys.contains(evdev::KeyCode::BTN_START), "{name}: no start");
    let rumble_ready = device
        .supported_ff()
        .is_some_and(|effects| effects.contains(evdev::FFEffectCode::FF_RUMBLE));
    assert!(
        rumble_ready,
        "{name}: pad lacks FF_RUMBLE — hid-nintendo did not finish probing"
    );
    let driver = switch_pro_hid_driver();
    if driver.is_empty() {
        // No sysfs view: FF_RUMBLE above is the binding proof anyway —
        // hid-generic never wires force feedback, so only hid-nintendo's
        // probe completing can produce the effect type.
        eprintln!("sysfs unreadable here; FF_RUMBLE implies hid-nintendo took the pad");
    } else {
        assert_eq!(
            driver, "nintendo",
            "twin was claimed by '{driver}', not hid-nintendo"
        );
    }
}

/// Exercises the Switch-over-hidraw physical driver against a live twin:
/// the same handshake, 0x30 parsing and rumble encoding the daemon uses on
/// a Switch-mode pad hid-nintendo has not claimed. The twin runs in a
/// servicing thread because the driver's probe blocks waiting for replies
/// the twin must be polled to send.
#[test]
#[ignore]
fn switch_hidraw_driver_reads_motion_buttons_and_rumble_from_a_twin() {
    use ira_input::{DeviceInfo, PadState, SwitchHidrawPad};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if !uhid_available() {
        eprintln!("skipping: /dev/uhid is not writable");
        return;
    }
    let before = evdev_snapshot();
    let mut switch_pro =
        SwitchProUhidDevice::create("ira-probe-switch-hidraw").expect("Switch Pro twin creation");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let pad_path = loop {
        switch_pro.service().expect("virtual Switch Pro stopped");
        match twin_node(&before, 0x057e, 0x2009) {
            Some(found) => break found.0,
            None => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "hid-nintendo never registered the virtual pro controller"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    eprintln!("twin registered at {}", pad_path.display());

    let info = DeviceInfo {
        path: pad_path,
        name: "Ira Virtual Switch Pro Controller".to_string(),
        vendor: 0x057e,
        product: 0x2009,
        version: 0,
        has_evdev_gyro: false,
        supported_buttons: Vec::new(),
    };
    let running = Arc::new(AtomicBool::new(true));
    let rumble_seen = Arc::new(AtomicBool::new(false));
    let thread_running = Arc::clone(&running);
    let thread_rumble = Arc::clone(&rumble_seen);
    let streamer = std::thread::spawn(move || {
        let pad = PadState {
            cross: true,
            lx: 0.5,
            ..PadState::default()
        };
        while thread_running.load(Ordering::Relaxed) {
            switch_pro.service().expect("virtual Switch Pro stopped");
            switch_pro
                .tick(&pad, [0.0, 0.0, 1.0], [10.0, -20.0, 30.0])
                .expect("twin report failed");
            for command in switch_pro.take_rumble() {
                if command.strong > 0 {
                    thread_rumble.store(true, Ordering::Relaxed);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    let mut driver = match SwitchHidrawPad::open(&info) {
        Some(driver) => driver,
        None => panic!("Switch hidraw driver did not engage on the twin"),
    };
    eprintln!("hidraw driver engaged");

    let mut samples = 0u32;
    let mut saw_button = false;
    let mut peak_dps = 0.0f32;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        driver.service();
        if let Some(sample) = driver.take_sample(0) {
            samples += 1;
            for rate in sample.gyro {
                peak_dps = peak_dps.max(rate.abs().to_degrees());
            }
        }
        for event in driver.take_events() {
            if matches!(
                event.source,
                ira_input::InputSource::Button(ira_input::GamepadButton::A)
            ) && event.value > 0.0
            {
                saw_button = true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Rumble round-trip: the driver encodes, the twin decodes.
    driver.play_rumble(ira_input::RumbleCommand {
        strong: 65_535,
        weak: 32_767,
        duration_ms: 120,
    });
    std::thread::sleep(std::time::Duration::from_millis(150));

    running.store(false, Ordering::Relaxed);
    let _ = streamer.join();

    let rumble = rumble_seen.load(Ordering::Relaxed);
    eprintln!("samples {samples}, peak {peak_dps:.1} deg/s, button {saw_button}, rumble {rumble}");
    assert!(samples > 0, "hidraw driver never delivered motion");
    assert!(peak_dps > 8.0, "gyro magnitudes wrong: peak {peak_dps:.1}");
    assert!(saw_button, "cross press never surfaced as button A");
    assert!(rumble, "twin never decoded a rumble report from the driver");
}

/// Reproduces the Switch-mode gyro failure end to end on the real kernel:
/// hid-nintendo registers its IMU companion for a registered twin exactly
/// as it does for a physical pad in Switch mode, so the daemon's whole
/// gyro chain — discovery, node open, sample drain, unit conversion — is
/// exercised against the same driver. Failing here is failing for the user.
#[test]
#[ignore]
fn kernel_imu_node_streams_gyro_for_a_registered_switch_pro_twin() {
    use ira_input::{DeviceInfo, EvdevImu, PadState};

    if !uhid_available() {
        eprintln!("skipping: /dev/uhid is not writable");
        return;
    }
    let before = evdev_snapshot();
    let mut switch_pro =
        SwitchProUhidDevice::create("ira-probe-switch-imu").expect("Switch Pro twin creation");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let (pad_path, pad_evdev) = loop {
        switch_pro.service().expect("virtual Switch Pro stopped");
        match twin_node(&before, 0x057e, 0x2009) {
            Some(found) => break found,
            None => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "hid-nintendo never registered the virtual pro controller"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    let pad_name = pad_evdev.name().unwrap_or_default().to_string();
    eprintln!("registered twin pad: {pad_name} at {}", pad_path.display());

    // What the daemon hands the sensor backend for the physical pad.
    let info = DeviceInfo {
        path: pad_path,
        name: pad_name.clone(),
        vendor: 0x057e,
        product: 0x2009,
        version: 0,
        has_evdev_gyro: false,
        supported_buttons: Vec::new(),
    };
    // udev publishes each node on its own schedule; the pad appeared a
    // moment ago and the companion can lag it, so poll like a daemon that
    // starts against an already-connected pad never has to.
    let mut imu = None;
    let discovery_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while imu.is_none() {
        imu = EvdevImu::open_in(&input_dir(), &info);
        if imu.is_some() || std::time::Instant::now() >= discovery_deadline {
            break;
        }
        switch_pro.service().expect("virtual Switch Pro stopped");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut imu = match imu {
        Some(imu) => imu,
        None => panic!(
            "no kernel IMU node discovered beside '{pad_name}' — \
             this is exactly how the daemon loses Switch-mode gyro"
        ),
    };
    eprintln!("IMU companion discovered");

    // Stream 0x30 reports with real motion the way the daemon's tick does,
    // and drain the kernel node the way the daemon's sensor read does.
    let pad = PadState::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let (mut samples, mut peak_dps) = (0u32, 0.0f32);
    while std::time::Instant::now() < deadline {
        switch_pro.service().expect("virtual Switch Pro stopped");
        switch_pro
            .tick(&pad, [0.0, 0.0, 1.0], [10.0, -20.0, 30.0])
            .expect("twin report failed");
        if let Ok(Some(sample)) = imu.read(0) {
            samples += 1;
            for rate in sample.gyro {
                peak_dps = peak_dps.max(rate.abs().to_degrees());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("read {samples} IMU samples, peak |gyro| {peak_dps:.1} deg/s");
    assert!(samples > 0, "kernel IMU node never delivered a sample");
    // Commanded 10..30 deg/s on each axis; a scaling or axis-map bug
    // collapses or scrambles this.
    assert!(
        peak_dps > 8.0,
        "gyro samples arrived but magnitudes are wrong: peak {peak_dps:.1} deg/s"
    );
}
