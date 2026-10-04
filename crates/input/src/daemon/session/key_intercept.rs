//! Keyboard chord authority and interception for gamescope launches.
//!
//! The host's X root grab cannot fire under gamescope (the compositor's
//! active keyboard grab takes precedence over passive grabs), so the daemon
//! owns keyboards the way it owns gamepads — with a focus gate:
//!
//! * Game focused: every keyboard is grabbed exclusively and all events
//!   are re-emitted through a virtual keyboard, except the toggle chord,
//!   which flips the overlay and is consumed. Only the combination is
//!   ever swallowed; the opening Tab never reaches the game.
//! * Overlay visible: the chord still flips (closing); every other key is
//!   dropped. Nothing keyboard-bound reaches the game.
//! * Game unfocused (or no virtual keyboard): read-only snoop — delivery
//!   untouched, chord reported but leaking like any other key.
//!
//! Failsafe: any crash closes our fds and the kernel releases the grabs;
//! transitions zero the virtual keyboard so no forwarded key sticks.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use evdev::{Device, EventSummary, KeyCode};
use ira_overlay_ipc::{KbdChord, MappedShm};

use crate::physical::is_ira_virtual_device;
use crate::{OutputEvent, VirtualKeyboard};

/// Hotplug rescan cadence: keyboards plugged late must be picked up, but
/// every rescan opens devices, so this stays far below the input rate.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Keyboard heuristic: letter row plus the two keys gamepads never carry.
fn is_keyboard(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_SPACE)
            && keys.contains(KeyCode::KEY_ENTER)
    })
}

pub(crate) struct KeyIntercept {
    matcher: KbdChord,
    shm: MappedShm,
    devices: HashMap<PathBuf, Device>,
    /// All-keys virtual keyboard for re-emission. `None` (creation failed)
    /// pins the intercept in snoop mode: grabbing without forwarding would
    /// swallow the user's typing.
    forward: Option<VirtualKeyboard>,
    /// evdev codes currently pressed on the virtual keyboard: released on
    /// every transition so nothing sticks downstream.
    fwd_held: HashSet<u16>,
    grabbed: bool,
    /// Opened keyboards whose grab failed (busy, racing opener): retried
    /// every rescan instead of silently staying ungrabbed forever.
    needs_grab: HashSet<PathBuf>,
    last_scan: Instant,
    warned: HashSet<PathBuf>,
}

impl KeyIntercept {
    /// Arms from the session env: same launcher marker as the gamepad
    /// toggle, same SHM mapping (read-write: firing flips through it).
    /// Silent `None` otherwise.
    pub(crate) fn arm(env: Option<&[(String, String)]>) -> Option<Self> {
        let shm_name = super::daemon_toggle_shm_name(env)?;
        let shm = MappedShm::open_rw(&shm_name).ok()?;
        let (key, mods, _, _, _, _) = shm.header().hotkeys();
        let forward = match VirtualKeyboard::create_all_keys() {
            Ok(keyboard) => Some(keyboard),
            Err(error) => {
                eprintln!("ira-input: key intercept snoop-only, no virtual keyboard: {error}");
                None
            }
        };
        let mut intercept = Self {
            matcher: KbdChord::new(key, mods),
            shm,
            devices: HashMap::new(),
            forward,
            fwd_held: HashSet::new(),
            grabbed: false,
            needs_grab: HashSet::new(),
            last_scan: Instant::now() - RESCAN_INTERVAL,
            warned: HashSet::new(),
        };
        intercept.rescan();
        eprintln!(
            "ira-input: key intercept watching {} keyboard(s)",
            intercept.devices.len()
        );
        Some(intercept)
    }

    /// One-line census of every input node at arm time: path, kernel name,
    /// and whether it counts as a keyboard. Turns "grabbed 0" from a
    /// mystery into a directly readable cause.
    pub(crate) fn log_input_census() {
        for (path, device) in evdev::enumerate() {
            eprintln!(
                "ira-input: input node {} ({}) keyboard={}",
                path.display(),
                device.name().unwrap_or("?"),
                is_keyboard(&device)
            );
        }
    }

    /// Flips the SHM visibility flag. Same contract as the gamepad toggle's
    /// flip, surfaced for logging.
    pub(crate) fn flip(&self) -> Option<bool> {
        self.shm.header().toggle_visible()
    }

    /// Polls all watched keyboards. `want_grab` is the session's focus
    /// gate; `overlay_open` switches forwarding to consuming. Returns true
    /// when the chord newly completed (caller flips).
    pub(crate) fn poll(&mut self, want_grab: bool, overlay_open: bool) -> bool {
        if self.last_scan.elapsed() >= RESCAN_INTERVAL {
            self.last_scan = Instant::now();
            self.rescan();
        }
        let want = want_grab && self.forward.is_some();
        if want != self.grabbed {
            self.set_grabbed(want);
        }
        let mut fired = false;
        let mut dead = Vec::new();
        let matcher = &mut self.matcher;
        let forward = &mut self.forward;
        let fwd_held = &mut self.fwd_held;
        let grabbed = self.grabbed;
        for (path, device) in self.devices.iter_mut() {
            match device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        if let EventSummary::Key(_, code, value) = event.destructure() {
                            if matcher.feed(code.0, value) {
                                fired = true;
                            } else if grabbed && !overlay_open {
                                Self::forward_key(forward, fwd_held, code.0, value);
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => dead.push(path.clone()),
            }
        }
        for path in dead {
            self.devices.remove(&path);
            self.warned.remove(&path);
        }
        fired
    }

    /// Re-emits one key event on the virtual keyboard, tracking held state
    /// for transition releases. Repeats (value 2) forward as presses,
    /// faithful to what the kernel would have delivered.
    fn forward_key(
        forward: &mut Option<VirtualKeyboard>,
        fwd_held: &mut HashSet<u16>,
        code: u16,
        value: i32,
    ) {
        let Some(keyboard) = forward.as_mut() else {
            return;
        };
        let pressed = value != 0;
        if pressed == fwd_held.contains(&code) {
            return;
        }
        if keyboard
            .emit(&OutputEvent::Key {
                keycode: code,
                pressed,
            })
            .is_err()
        {
            return;
        }
        if pressed {
            fwd_held.insert(code);
        } else {
            fwd_held.remove(&code);
        }
    }

    /// Releases everything the virtual keyboard holds. Runs on every grab
    /// transition so focus and overlay edges never stick a key downstream.
    fn zero_forward(&mut self) {
        if self.fwd_held.is_empty() {
            return;
        }
        if let Some(forward) = self.forward.as_mut() {
            for code in std::mem::take(&mut self.fwd_held) {
                let _ = forward.emit(&OutputEvent::Key {
                    keycode: code,
                    pressed: false,
                });
            }
        } else {
            self.fwd_held.clear();
        }
    }

    fn set_grabbed(&mut self, grabbed: bool) {
        self.zero_forward();
        if grabbed {
            let mut count = 0;
            // Borrow paths first: grabbing needs &mut device plus
            // &mut self for failure bookkeeping.
            let paths: Vec<PathBuf> = self.devices.keys().cloned().collect();
            for path in paths {
                let grabbed = self
                    .devices
                    .get_mut(&path)
                    .is_some_and(|device| device.grab().is_ok());
                if grabbed {
                    count += 1;
                    self.needs_grab.remove(&path);
                    self.warned.remove(&path);
                } else {
                    self.needs_grab.insert(path.clone());
                    if self.warned.insert(path.clone()) {
                        eprintln!(
                            "ira-input: key intercept grab failed on {} (busy or racing opener); retrying",
                            path.display()
                        );
                    }
                }
            }
            eprintln!(
                "ira-input: key intercept grabbed {count}/{} keyboard(s)",
                self.devices.len()
            );
        } else {
            for device in self.devices.values_mut() {
                let _ = device.ungrab();
            }
            self.needs_grab.clear();
            eprintln!("ira-input: key intercept released keyboards");
        }
        self.grabbed = grabbed;
        // Physical state is unknown across the transition: held keys are
        // grip, not chord parts.
        let (key, mods, _, _, _, _) = self.shm.header().hotkeys();
        self.matcher = KbdChord::new(key, mods);
    }

    fn rescan(&mut self) {
        let mut seen = HashSet::new();
        for (path, probe) in evdev::enumerate() {
            // Never watch our own outputs: the virtual keyboard would
            // otherwise watch (and with a grab, swallow) its own
            // re-emitted keys.
            if is_ira_virtual_device(probe.name().unwrap_or_default()) {
                continue;
            }
            if !is_keyboard(&probe) {
                continue;
            }
            seen.insert(path.clone());
            if self.devices.contains_key(&path) {
                continue;
            }
            match Device::open(&path) {
                Ok(mut opened) => {
                    if opened.set_nonblocking(true).is_err() {
                        continue;
                    }
                    // Drain pre-open state: keys held before the open must
                    // not count as fresh presses, and the drain doubles as
                    // a liveness check.
                    let _ = opened.fetch_events();
                    // A failed grab keeps the device for snooping (delivery
                    // untouched) and retries below; dropping it would blind
                    // the chord path too.
                    if self.grabbed && opened.grab().is_err() {
                        self.needs_grab.insert(path.clone());
                        if self.warned.insert(path.clone()) {
                            eprintln!(
                                "ira-input: key intercept grab failed on {} (busy or racing opener); retrying",
                                path.display()
                            );
                        }
                    } else {
                        self.warned.remove(&path);
                    }
                    self.devices.insert(path, opened);
                }
                Err(error) => {
                    if self.warned.insert(path.clone()) {
                        eprintln!(
                            "ira-input: key intercept cannot open {}: {error}",
                            path.display()
                        );
                    }
                }
            }
        }
        // Retry failed grabs (a stale holder may have exited since).
        if self.grabbed && !self.needs_grab.is_empty() {
            let retry: Vec<PathBuf> = self
                .needs_grab
                .iter()
                .filter(|path| self.devices.contains_key(*path))
                .cloned()
                .collect();
            for path in retry {
                let grabbed = self
                    .devices
                    .get_mut(&path)
                    .is_some_and(|device| device.grab().is_ok());
                if grabbed {
                    self.needs_grab.remove(&path);
                    self.warned.remove(&path);
                    eprintln!(
                        "ira-input: key intercept grabbed {} on retry",
                        path.display()
                    );
                }
            }
        }
        self.devices.retain(|path, _| seen.contains(path));
        self.needs_grab.retain(|path| seen.contains(path));
        self.warned.retain(|path| seen.contains(path) || self.devices.contains_key(path));
    }
}

impl Drop for KeyIntercept {
    fn drop(&mut self) {
        // The kernel releases the grabs on close; the virtual keyboard
        // needs explicit zeroes or forwarded keys stick downstream.
        self.zero_forward();
        for device in self.devices.values_mut() {
            let _ = device.ungrab();
        }
    }
}
