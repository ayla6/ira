//! The hub's physical side: the one open controller, its motion source, the
//! Switch-protocol takeover, and rumble — plus the reconnect and motion
//! retry cadences that keep all of them alive across hotplugs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{PhysicalGamepad, PhysicalRumble, SwitchHidrawPad};

use super::super::session::{
    apply_controller_layout, open_sensor, probe_sensor, reconnect_gamepad, resolved_layout_for,
    GyroSource,
};
use super::{broadcast_motion, RouteEntry};

/// Reconnect cadence for a pad that vanished, matching the old per-session
/// reconnect interval.
pub(super) const RECONNECT_INTERVAL: Duration = Duration::from_millis(250);
/// First retry delay for a pad that opened without a motion source: the
/// connect-time probe races udev and SDL's first enumeration, so a
/// controller switched on after the daemon is ready can lose it. Retries
/// back off so a genuinely gyroless pad pays only a small periodic probe.
const MOTION_RETRY_START: Duration = Duration::from_millis(250);
const MOTION_RETRY_MAX: Duration = Duration::from_secs(4);

/// Physical-side state, owned exclusively by the hub thread.
pub(super) struct PhysicalPad {
    pub(super) gamepad: Option<PhysicalGamepad>,
    pub(super) switch_hidraw: Option<SwitchHidrawPad>,
    pub(super) sensor: Option<GyroSource>,
    /// Exclusive hold on the pad's hidraw while a native-twin route wants
    /// the hardware hidden from SDL-style stacks.
    pub(super) hidraw_grab: Option<crate::hidraw_grab::PadHidrawGrab>,
    pub(super) rumble: Option<PhysicalRumble>,
    pub(super) calibration: Option<PathBuf>,
    pub(super) device_hint: Option<PathBuf>,
    pub(super) reconnect_at: Instant,
    /// Next scheduled motion probe for a pad that is up but motion-less.
    pub(super) motion_retry_at: Instant,
    pub(super) motion_retry_delay: Duration,
}

impl PhysicalPad {
    pub(super) fn new() -> Self {
        Self {
            gamepad: None,
            switch_hidraw: None,
            sensor: None,
            hidraw_grab: None,
            rumble: None,
            calibration: None,
            device_hint: None,
            // Try the first open immediately instead of one interval late.
            reconnect_at: Instant::now()
                .checked_sub(RECONNECT_INTERVAL)
                .unwrap_or_else(Instant::now),
            motion_retry_at: Instant::now(),
            motion_retry_delay: MOTION_RETRY_START,
        }
    }

    /// Re-evaluates the exclusive hidraw hold that keeps SDL-style stacks
    /// from claiming the physical pad over a native twin. Re-applied every
    /// hub pass: reconnects create fresh hidraw nodes, and a stale hold
    /// (its device went away) must be replaced by one on the new node.
    pub(super) fn set_hidraw_grab(&mut self, hold: bool) {
        if hold && self.hidraw_grab.is_none() {
            if let Some(gamepad) = self.gamepad.as_ref() {
                let path = gamepad.info().path.clone();
                match crate::hidraw_grab::grab_pad_hidraw(&path) {
                    Some(grab) => {
                        eprintln!(
                            "hub: holding {} exclusively; the native twin is the pad SDL sees",
                            grab.path().display()
                        );
                        self.hidraw_grab = Some(grab);
                    }
                    None => self.hidraw_grab = None,
                }
            }
        } else if !hold && self.hidraw_grab.is_some() {
            self.hidraw_grab = None; // dropping the file releases the grab
            eprintln!("hub: hidraw hold released; the physical pad is public again");
        }
    }

    pub(super) fn motion_alive(&self) -> bool {
        self.sensor.is_some() || self.switch_hidraw.is_some()
    }

    /// Claims the open pad exclusively for routed sessions.
    pub(super) fn grab(&mut self) -> Result<(), String> {
        match self.gamepad.as_mut() {
            Some(gamepad) => gamepad.grab(),
            None => Ok(()),
        }
    }

    /// Hands the pad back to the desktop when the last session leaves.
    pub(super) fn ungrab(&mut self) {
        if let Some(gamepad) = self.gamepad.as_mut() {
            gamepad.ungrab();
        }
    }

    /// Opens the first pad or reopens the previous one, re-establishing the
    /// motion source, the Switch-protocol takeover, and rumble. The pad is
    /// grabbed only when sessions are subscribed: an idle hub must leave
    /// the controller to the desktop.
    pub(super) fn try_open(&mut self, grab: bool) -> Option<(String, String, u16, u16)> {
        // The motion probe below races udev and SDL's first enumeration;
        // schedule retries so a lost race heals itself while the pad stays
        // connected.
        self.motion_retry_at = Instant::now();
        self.motion_retry_delay = MOTION_RETRY_START;
        match reconnect_gamepad(&mut self.gamepad) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(error) => {
                eprintln!("hub: controller reconnect failed: {error}");
                return None;
            }
        }
        if grab {
            if let Err(error) = self.gamepad.as_mut().unwrap().grab() {
                eprintln!("hub: failed to grab controller: {error}");
            }
        }
        apply_controller_layout(&mut self.gamepad, self.calibration.as_deref());
        self.sensor = self
            .gamepad
            .as_ref()
            .and_then(|gamepad| open_sensor(gamepad.info()));
        if self.sensor.is_none() {
            self.switch_hidraw = self
                .gamepad
                .as_ref()
                .and_then(|gamepad| SwitchHidrawPad::open(gamepad.info()));
            if let (Some(driver), Some(gamepad)) =
                (self.switch_hidraw.as_mut(), self.gamepad.as_ref())
            {
                driver.set_nintendo_layout(resolved_layout_for(
                    gamepad.info(),
                    self.calibration.as_deref(),
                ));
            }
        }
        self.rumble = if self.switch_hidraw.is_some() {
            None
        } else {
            open_rumble(self.gamepad.as_ref(), true)
        };
        let info = self.gamepad.as_ref().unwrap().info();
        eprintln!("hub: controller connected through {}", info.path.display());
        Some((
            info.name.clone(),
            info.path.display().to_string(),
            info.vendor,
            info.product,
        ))
    }

    /// The pad is up but still motion-less: probe again. Success is
    /// broadcast so running sessions can attach their motion outputs.
    pub(super) fn retry_motion(&mut self, routes: &HashMap<u64, RouteEntry>) {
        self.motion_retry_at = Instant::now();
        self.motion_retry_delay = next_retry_delay(self.motion_retry_delay);
        let probed = self
            .gamepad
            .as_ref()
            .and_then(|gamepad| probe_sensor(gamepad.info()));
        if let Some(sensor) = probed {
            self.sensor = Some(sensor);
            eprintln!("hub: motion source attached on retry");
            broadcast_motion(routes, true);
            return;
        }
        // Switch-protocol fallback, mirroring `try_open`.
        let switch = self
            .gamepad
            .as_ref()
            .and_then(|gamepad| SwitchHidrawPad::open(gamepad.info()));
        if let Some(mut driver) = switch {
            if let Some(gamepad) = self.gamepad.as_ref() {
                driver
                    .set_nintendo_layout(resolved_layout_for(
                        gamepad.info(),
                        self.calibration.as_deref(),
                    ));
            }
            self.switch_hidraw = Some(driver);
            self.rumble = None;
            eprintln!("hub: switch takeover attached on retry");
            broadcast_motion(routes, true);
        }
    }

    pub(super) fn drop_pad(&mut self) {
        self.gamepad = None;
        self.switch_hidraw = None;
        self.sensor = None;
        // The held hidraw node died with the pad connection; the next open
        // creates a fresh one and the hub re-grabs it.
        self.hidraw_grab = None;
        if let Some(rumble) = self.rumble.as_mut() {
            rumble.stop();
        }
        self.reconnect_at = Instant::now();
        self.motion_retry_at = Instant::now();
        self.motion_retry_delay = MOTION_RETRY_START;
    }
}

/// Opens the physical side of rumble passthrough. Failure reasons are logged
/// exactly once here; a missing handle afterwards simply means "no rumble"
/// and every forwarded command is skipped.
fn open_rumble(
    gamepad: Option<&PhysicalGamepad>,
    enabled: bool,
) -> Option<crate::PhysicalRumble> {
    if !enabled {
        return None;
    }
    let info = gamepad?.info();
    let path = info.path.clone();
    match crate::PhysicalRumble::open(&path) {
        Ok(rumble) => Some(rumble),
        Err(primary_error) => match ff_sibling_node(info, &path) {
            Some(sibling) => {
                eprintln!(
                    "ira-input: {primary_error}; using rumble on {} instead",
                    sibling.display()
                );
                match crate::PhysicalRumble::open(&sibling) {
                    Ok(rumble) => Some(rumble),
                    Err(error) => {
                        eprintln!("ira-input: {error}");
                        None
                    }
                }
            }
            None => match crate::PhysicalRumble::open_vendor_hidraw(
                &path,
                info.vendor,
                info.product,
            ) {
                Ok(rumble) => {
                    eprintln!(
                        "ira-input: {primary_error}; replaying rumble through the 8BitDo \
                         hidraw protocol instead"
                    );
                    Some(rumble)
                }
                Err(error) => {
                    eprintln!("{primary_error}");
                    eprintln!("ira-input: {error}");
                    None
                }
            },
        },
    }
}

/// Finds another evdev node of the same physical controller that does
/// declare FF_RUMBLE. Pads with the classic Linux dual-node split often
/// keep force feedback off the node SDL picks as the gamepad.
fn ff_sibling_node(info: &crate::DeviceInfo, skip: &std::path::Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir("/dev/input").ok()?.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("event") || entry.path() == skip {
            continue;
        }
        let Ok(device) = evdev::Device::open(entry.path()) else {
            continue;
        };
        let id = device.input_id();
        if id.vendor() != info.vendor || id.product() != info.product {
            continue;
        }
        let has_ff = device
            .supported_ff()
            .is_some_and(|effects| effects.contains(evdev::FFEffectCode::FF_RUMBLE));
        if has_ff {
            return Some(entry.path());
        }
    }
    None
}

fn next_retry_delay(delay: Duration) -> Duration {
    (delay * 2).min(MOTION_RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{next_retry_delay, MOTION_RETRY_MAX, MOTION_RETRY_START};

    #[test]
    fn test_retry_delay_doubles_then_caps() {
        assert_eq!(
            next_retry_delay(MOTION_RETRY_START),
            Duration::from_millis(500)
        );
        let mut delay = MOTION_RETRY_START;
        for _ in 0..20 {
            delay = next_retry_delay(delay);
        }
        assert_eq!(delay, MOTION_RETRY_MAX, "backoff must stop at the cap");
    }
}
