//! The pad hub: the daemon's single owner of every physical controller.
//! Sessions never touch hardware — they subscribe, and the hub forwards the
//! events and motion samples of the focused game's controller while replaying
//! that game's rumble back onto the physical pad. Unfocused sessions stay
//! subscribed but receive nothing: frozen, not closed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::{
    InputEvent, PhysicalGamepad, PhysicalRumble, RumbleCommand, SensorSample, SwitchHidrawPad,
};

use super::session::{
    apply_controller_layout, now_us, open_rumble, open_sensor, reconnect_gamepad,
    resolved_layout_for, GyroSource,
};
use super::signals::STOP_REQUESTED;

/// How often an idle hub (no pad, or a hidraw takeover with no pollable
/// descriptor) re-checks commands, hotplug, and buffered driver events.
const IDLE_POLL: Duration = Duration::from_millis(50);
/// During a Switch-protocol takeover events arrive on hidraw with no
/// pollable descriptor, so that mode polls on a tight cadence instead.
const SWITCH_POLL: Duration = Duration::from_millis(5);
/// Reconnect cadence for a pad that vanished, matching the old per-session
/// reconnect interval.
const RECONNECT_INTERVAL: Duration = Duration::from_millis(250);
/// How long a Switch-protocol takeover without any report may last before
/// the hub abandons it and returns to the evdev input path.
const SWITCH_DRIVER_SILENCE_LIMIT: Duration = Duration::from_secs(3);

/// Everything a session receives from the physical side.
#[derive(Debug, Clone)]
pub(crate) enum PadEvent {
    /// A raw controller event, routed here because this session owns focus.
    Input(InputEvent),
    /// One motion sample from whichever gyro source the pad provides.
    Sample(SensorSample),
    /// Whether any motion source is currently live for the pad.
    Motion(bool),
    /// A pad was opened (first connect or reconnect).
    Connected {
        name: String,
        path: String,
        vendor: u16,
        product: u16,
    },
    /// This session lost focus routing: release held outputs.
    Frozen,
    /// This session regained focus routing.
    Live,
    /// Session-local wake: the profile watcher thread signals a save. The
    /// hub never sends this variant.
    ProfileChanged,
}

/// The pad's state at subscribe time, so a session can build its output
/// stack for the right controller kind before its game even spawns.
#[derive(Debug, Clone)]
pub(crate) struct PadSnapshot {
    pub(crate) motion: bool,
    pub(crate) pad: Option<(String, String, u16, u16)>,
}

/// A session registers with the hub before its loop starts.
pub(crate) struct Subscribe {
    pub(crate) id: u64,
    pub(crate) events: Sender<PadEvent>,
    /// Sessions that opted out of focus pausing stay live when they are the
    /// only one — the pre-daemon behavior.
    pub(crate) live_always: bool,
    pub(crate) calibration: Option<PathBuf>,
    pub(crate) device: Option<PathBuf>,
    pub(crate) reply: Sender<PadSnapshot>,
}

pub(crate) enum HubCommand {
    Subscribe(Subscribe),
    Unsubscribe(u64),
    Focus { id: u64, focused: bool },
    /// Rumble from a session; only the routed session's commands play.
    Rumble { id: u64, command: RumbleCommand },
}

struct RouteEntry {
    events: Sender<PadEvent>,
    focused: bool,
    live_always: bool,
    always_claims_focus: bool,
    focus_seq: u64,
}

impl RouteEntry {
    fn claims_focus(&self) -> bool {
        self.focused || self.always_claims_focus
    }
}

/// The handle sessions and the server keep to talk to the hub thread.
#[derive(Clone)]
pub(crate) struct HubHandle {
    commands: Sender<HubCommand>,
}

impl HubHandle {
    pub(crate) fn send(&self, command: HubCommand) {
        // A dead hub means the daemon is shutting down; winding-down
        // sessions need no pad anymore.
        let _ = self.commands.send(command);
    }
}

/// Physical-side state, owned exclusively by the hub thread.
struct PhysicalPad {
    gamepad: Option<PhysicalGamepad>,
    switch_hidraw: Option<SwitchHidrawPad>,
    sensor: Option<GyroSource>,
    rumble: Option<PhysicalRumble>,
    calibration: Option<PathBuf>,
    device_hint: Option<PathBuf>,
    reconnect_at: Instant,
}

impl PhysicalPad {
    fn new() -> Self {
        Self {
            gamepad: None,
            switch_hidraw: None,
            sensor: None,
            rumble: None,
            calibration: None,
            device_hint: None,
            // Try the first open immediately instead of one interval late.
            reconnect_at: Instant::now()
                .checked_sub(RECONNECT_INTERVAL)
                .unwrap_or_else(Instant::now),
        }
    }

    fn motion_alive(&self) -> bool {
        self.sensor.is_some() || self.switch_hidraw.is_some()
    }

    /// Opens the first pad or reopens the previous one, re-establishing the
    /// motion source, the Switch-protocol takeover, and rumble.
    fn try_open(&mut self) -> Option<(String, String, u16, u16)> {
        match reconnect_gamepad(&mut self.gamepad) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(error) => {
                eprintln!("hub: controller reconnect failed: {error}");
                return None;
            }
        }
        if let Err(error) = self.gamepad.as_mut().unwrap().grab() {
            eprintln!("hub: failed to grab controller: {error}");
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

    fn drop_pad(&mut self) {
        self.gamepad = None;
        self.switch_hidraw = None;
        self.sensor = None;
        if let Some(rumble) = self.rumble.as_mut() {
            rumble.stop();
        }
        self.reconnect_at = Instant::now();
    }
}

/// Starts the hub thread.
pub(crate) fn spawn(controller_events: Sender<(bool, String, String)>) -> HubHandle {
    let (commands, receiver) = channel();
    let _ = std::thread::Builder::new()
        .name("ira-pad-hub".to_string())
        .spawn(move || run(receiver, controller_events));
    HubHandle { commands }
}

fn run(commands: Receiver<HubCommand>, controller_events: Sender<(bool, String, String)>) {
    let mut pad = PhysicalPad::new();
    let mut routes: HashMap<u64, RouteEntry> = HashMap::new();
    let mut focus_counter: u64 = 0;
    let mut routed: Option<u64> = None;

    loop {
        if STOP_REQUESTED.load(Ordering::Relaxed) {
            return;
        }
        if !drain_commands(
            &commands,
            &mut routes,
            &mut focus_counter,
            &mut pad,
            &routed,
        ) {
            // Every handle is gone: a standalone session finished and its
            // private hub has no reason to outlive it.
            return;
        }
        if pad
            .gamepad
            .as_ref()
            .is_some_and(|gamepad| !gamepad.is_connected())
        {
            eprintln!("hub: controller disconnected");
            let _ = controller_events.send((false, String::new(), String::new()));
            pad.drop_pad();
        }
        if pad.gamepad.is_none() && pad.reconnect_at.elapsed() >= RECONNECT_INTERVAL {
            pad.reconnect_at = Instant::now();
            if let Some((name, path, _, _)) = pad.try_open() {
                let _ = controller_events.send((true, name, path));
                broadcast_motion(&routes, pad.motion_alive());
            }
        }
        if pad.gamepad.is_some() {
            read_pad(&mut pad, &routes, &routed);
        }
        retarget(&mut routes, &mut routed);
        park(&pad);
    }
}

/// Parks until the pad has evdev input or the cadence timeout elapses.
fn park(pad: &PhysicalPad) {
    let timeout = if pad.switch_hidraw.is_some() {
        SWITCH_POLL
    } else {
        IDLE_POLL
    };
    match pad.gamepad.as_ref().and_then(PhysicalGamepad::device_fd) {
        Some(fd) => {
            let mut descriptor = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ms = timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
            let result = unsafe { libc::poll(&mut descriptor, 1, ms) };
            if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                eprintln!(
                    "hub: failed waiting for controller: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        None => std::thread::sleep(timeout),
    }
}

/// Applies routing changes: the session that gains focus routing gets
/// `Live`, the one that loses it gets `Frozen`.
fn retarget(routes: &mut HashMap<u64, RouteEntry>, routed: &mut Option<u64>) {
    let target = pick_target(routes);
    if target == *routed {
        return;
    }
    if let Some(old) = *routed {
        if let Some(entry) = routes.get_mut(&old) {
            let _ = entry.events.send(PadEvent::Frozen);
        }
    }
    if let Some(new) = target {
        if let Some(entry) = routes.get_mut(&new) {
            let _ = entry.events.send(PadEvent::Live);
        }
    }
    *routed = target;
}

/// The focused session wins (most recently focused on ties); a lone
/// `live_always` session stays routed when nothing is focused, preserving
/// the pre-daemon behavior for a single game.
fn pick_target(routes: &HashMap<u64, RouteEntry>) -> Option<u64> {
    routes
        .iter()
        .filter(|(_, entry)| entry.claims_focus())
        .max_by_key(|(_, entry)| entry.focus_seq)
        .map(|(id, _)| *id)
        .or_else(|| {
            if routes.len() == 1 {
                routes
                    .iter()
                    .find(|(_, entry)| entry.live_always)
                    .map(|(id, _)| *id)
            } else {
                None
            }
        })
}

/// Drains pending commands. Returns `false` when every hub handle was
/// dropped and the thread should exit.
fn drain_commands(
    commands: &Receiver<HubCommand>,
    routes: &mut HashMap<u64, RouteEntry>,
    focus_counter: &mut u64,
    pad: &mut PhysicalPad,
    routed: &Option<u64>,
) -> bool {
    loop {
        match commands.try_recv() {
            // Every handle is gone: a standalone session finished and its
            // private hub has no reason to outlive it.
            Err(TryRecvError::Disconnected) => return false,
            Err(TryRecvError::Empty) => return true,
            Ok(HubCommand::Subscribe(subscribe)) => {
                let events = subscribe.events.clone();
                if pad.calibration.is_none() {
                    pad.calibration = subscribe.calibration.clone();
                }
                if pad.device_hint.is_none() {
                    pad.device_hint = subscribe.device.clone();
                }
                routes.insert(
                    subscribe.id,
                    RouteEntry {
                        events: subscribe.events,
                        focused: false,
                        live_always: subscribe.live_always,
                        always_claims_focus: false,
                        focus_seq: 0,
                    },
                );
                // Bring the newcomer up to speed synchronously: current
                // motion state and, when the pad is already open, its
                // identity — the session builds its output stack for the
                // right controller kind before spawning its game.
                let snapshot = PadSnapshot {
                    motion: pad.motion_alive(),
                    pad: pad.gamepad.as_ref().map(|gamepad| {
                        let info = gamepad.info();
                        (
                            info.name.clone(),
                            info.path.display().to_string(),
                            info.vendor,
                            info.product,
                        )
                    }),
                };
                let _ = subscribe.reply.send(snapshot);
                let _ = events.send(PadEvent::Motion(pad.motion_alive()));
                if let Some(gamepad) = pad.gamepad.as_ref() {
                    let info = gamepad.info();
                    let _ = events.send(PadEvent::Connected {
                        name: info.name.clone(),
                        path: info.path.display().to_string(),
                        vendor: info.vendor,
                        product: info.product,
                    });
                }
            }
            Ok(HubCommand::Unsubscribe(id)) => {
                routes.remove(&id);
            }
            Ok(HubCommand::Focus { id, focused }) => {
                if let Some(entry) = routes.get_mut(&id) {
                    entry.focused = focused;
                    if focused {
                        *focus_counter += 1;
                        entry.focus_seq = *focus_counter;
                    }
                }
            }
            Ok(HubCommand::Rumble { id, command }) => {
                if *routed == Some(id) {
                    play_rumble(pad, command);
                }
            }
        }
    }
}

fn play_rumble(pad: &mut PhysicalPad, command: RumbleCommand) {
    if let Some(switch) = pad.switch_hidraw.as_mut() {
        switch.play_rumble(command);
    } else if let Some(rumble) = pad.rumble.as_mut() {
        rumble.play(command);
    }
}

/// Reads everything the physical pad currently offers and forwards it to the
/// routed session: evdev or Switch-protocol events, then motion samples.
fn read_pad(pad: &mut PhysicalPad, routes: &HashMap<u64, RouteEntry>, routed: &Option<u64>) {
    // Fail-safe for the Switch hidraw takeover: a pad that streamed once and
    // went silent must not starve every session of input.
    if pad
        .switch_hidraw
        .as_ref()
        .is_some_and(|driver| driver.silent_for() >= SWITCH_DRIVER_SILENCE_LIMIT)
    {
        eprintln!("hub: Switch hidraw driver went silent; falling back to evdev");
        pad.switch_hidraw = None;
        pad.rumble = open_rumble(pad.gamepad.as_ref(), true);
    }
    if let Some(rumble) = pad.rumble.as_mut() {
        rumble.service();
    }
    if let Some(switch) = pad.switch_hidraw.as_mut() {
        switch.service();
    }
    let Some(target) = routed.and_then(|id| routes.get(&id)) else {
        // Nobody is routed: drain so the fd never pins POLLIN, but nothing
        // is mapped anywhere.
        if let Some(switch) = pad.switch_hidraw.as_mut() {
            let _ = switch.take_events();
        } else if let Some(gamepad) = pad.gamepad.as_mut() {
            let _ = gamepad.fetch_events();
        }
        return;
    };
    let events = &target.events;
    if let Some(switch) = pad.switch_hidraw.as_mut() {
        for event in switch.take_events() {
            let _ = events.send(PadEvent::Input(event));
        }
        if let Some(sample) = switch.take_sample(now_us()) {
            let _ = events.send(PadEvent::Sample(sample));
        }
    } else if let Some(gamepad) = pad.gamepad.as_mut() {
        match gamepad.fetch_events() {
            Ok(batch) => {
                for event in batch {
                    let _ = events.send(PadEvent::Input(event));
                }
            }
            Err(error) => eprintln!("hub: failed to read controller: {error}"),
        }
    }
    if let Some(sensor) = pad.sensor.as_mut() {
        match sensor.read(now_us()) {
            Ok(Some(sample)) => {
                let _ = events.send(PadEvent::Sample(sample));
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("hub: gyro backend stopped: {error}");
                pad.sensor = None;
                broadcast_motion(routes, pad.motion_alive());
            }
        }
    }
}

fn broadcast_motion(routes: &HashMap<u64, RouteEntry>, available: bool) {
    for entry in routes.values() {
        let _ = entry.events.send(PadEvent::Motion(available));
    }
}
