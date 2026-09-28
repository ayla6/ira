//! The pad hub: the daemon's single owner of every physical controller.
//! Sessions never touch hardware — they subscribe, and the hub forwards the
//! events and motion samples of the focused game's controller while replaying
//! that game's rumble back onto the physical pad. Unfocused sessions stay
//! subscribed but receive nothing: frozen, not closed.

mod holds;
mod pad;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::{InputEvent, InputSource, PhysicalGamepad, RumbleCommand, SensorSample};

use super::session::{now_us, open_rumble};
use super::signals::STOP_REQUESTED;

use pad::{PhysicalPad, RECONNECT_INTERVAL};

/// How often an idle hub (no pad, or a hidraw takeover with no pollable
/// descriptor) re-checks commands, hotplug, and buffered driver events.
const IDLE_POLL: Duration = Duration::from_millis(50);
/// During a Switch-protocol takeover events arrive on hidraw with no
/// pollable descriptor, so that mode polls on a tight cadence instead.
const SWITCH_POLL: Duration = Duration::from_millis(5);
/// SDL exposes only each sensor's latest sample and offers no wakeup for
/// new reports, so the hub must poll. USB controllers stream IMU reports
/// at up to 1000 Hz and SDL discards everything but the newest, so
/// polling once per millisecond is what keeps the passthrough from
/// stepping over (losing) intermediate samples; the timestamp freshness
/// filter makes the extra polls no-ops when nothing new arrived.
const SENSOR_POLL: Duration = Duration::from_millis(1);
/// How long a Switch-protocol takeover without any report may last before
/// the hub abandons it and returns to the evdev input path.
const SWITCH_DRIVER_SILENCE_LIMIT: Duration = Duration::from_secs(3);
/// How long a Stop for a session that never subscribed is remembered. A
/// session that has not joined the hub within this window was never really
/// running (its spawn failed, or it exited before subscribing), so the
/// pending stop outlived its purpose.
const PENDING_STOP_TTL: Duration = Duration::from_secs(10);

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
        version: u16,
    },
    /// This session lost focus routing or the physical pad disconnected:
    /// release held outputs (the virtual pad would otherwise freeze with
    /// buttons held) and drop stale queued input.
    Frozen,
    /// This session (re)gained focus routing, carrying the physical button
    /// pressed set: the session releases anything its mapper holds but the
    /// pad reports up, reconciling state lost across the routing gap
    /// (focus flap mid-hold, reconnect) instead of sticking it forever.
    Live { pressed: Vec<InputSource> },
    /// Session-local wake: the profile watcher thread signals a save. The
    /// hub never sends this variant.
    ProfileChanged,
    /// The game's controller layout was switched in the app; the session
    /// reloads from the new profile path. Server-routed, focus-independent.
    ReloadProfile(PathBuf),
    /// The server told this session (a desktop-default one) to finish.
    /// Server-routed, focus-independent.
    Stop,
}

/// The pad's state at subscribe time, so a session can build its output
/// stack for the right controller kind before its game even spawns.
#[derive(Debug, Clone)]
pub(crate) struct PadSnapshot {
    pub(crate) motion: bool,
    pub(crate) pad: Option<(String, String, u16, u16, u16)>,
    /// The open pad answers the Switch controller protocol directly.
    pub(crate) switch_protocol: bool,
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
    /// Physical button pressed set, for overlay-close hand-back: buttons
    /// held through the overlay's input blackout need re-presses, since
    /// no edges arrive while held.
    GetPressed {
        reply: Sender<HashSet<InputSource>>,
    },
    /// Server-routed layout switch for one session, regardless of focus.
    ReloadProfile { id: u64, path: PathBuf },
    /// Ask one session to finish cleanly (a desktop-default session being
    /// replaced or released).
    Stop(u64),
    /// The session passes the physical pad through untouched (it already
    /// speaks the profile's native protocol): the hub must not grab it.
    SetPassthrough { id: u64, passthrough: bool },
}

struct RouteEntry {
    events: Sender<PadEvent>,
    focused: bool,
    live_always: bool,
    always_claims_focus: bool,
    focus_seq: u64,
    passthrough: bool,
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
    let mut pending_stops: HashMap<u64, Instant> = HashMap::new();
    // Physical button pressed set, tracked across every batch the hub
    // reads (routed or drained): sessions reconcile against it on `Live`.
    // Buttons only — axes are level-semantic and need no reconciliation.
    let mut pressed: HashSet<InputSource> = HashSet::new();
    // Last session that held routing: releases arriving with no target
    // are forwarded here (best-effort) instead of dropped, so a release
    // landing in a focus gap cannot stick a button forever.
    let mut last_live: Option<u64> = None;

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
            &mut pending_stops,
            &pressed,
        ) {
            // Every handle is gone: a standalone session finished and its
            // private hub has no reason to outlive it.
            return;
        }
        // With no sessions subscribed, the pad stays untouched: ungrabbed
        // and unread, so the desktop keeps full native use of it. The hub
        // still watches for hotplug so a later subscribe is instant.
        let subscribed = !routes.is_empty();
        if pad
            .gamepad
            .as_ref()
            .is_some_and(|gamepad| !gamepad.is_connected())
        {
            eprintln!("hub: controller disconnected");
            let _ = controller_events.send((false, String::new(), String::new()));
            // The routed session would otherwise keep its virtual outputs
            // frozen in the last held state: endless held buttons with no
            // physical pad left to release them. Physical truth clears too:
            // the next `Live` reconciles against an empty set.
            notify_disconnect(&routes, &routed);
            pressed.clear();
            pad.drop_pad();
        }
        // While any session claims the pad, the physical controller's
        // companion evdev nodes (IMU sensors, vendor keyboards) are held
        // exclusively: no other process may read raw hardware input past
        // the remap. Re-evaluated every pass so reconnects — which create
        // fresh nodes — are re-held, and a node our own sensor attaches to
        // is released back to it.
        let any_claiming = routes.values().any(|route| !route.passthrough);
        if subscribed && any_claiming
            && pad
                .gamepad
                .as_ref()
                .is_some_and(|gamepad| !gamepad.is_grabbed())
        {
            // The first subscribe after an idle stretch claims the pad for
            // routed sessions again.
            if let Err(error) = pad.grab() {
                eprintln!("hub: failed to grab controller: {error}");
            }
        }
        if (!subscribed || !any_claiming)
            && pad.gamepad.as_ref().is_some_and(|pad| pad.is_grabbed())
        {
            // The last session went away: hand the controller back to the
            // desktop instead of holding it dead in the void.
            pad.ungrab();
        }
        pad.set_sibling_holds(subscribed && any_claiming);
        if pad.gamepad.is_none() && pad.reconnect_at.elapsed() >= RECONNECT_INTERVAL {
            pad.reconnect_at = Instant::now();
            if let Some((name, path, vendor, product, version)) = pad.try_open(subscribed) {
                let _ = controller_events.send((true, name.clone(), path.clone()));
                // Sessions that subscribed before the pad existed (the normal
                // case: opening the pad probes the motion source for ~200 ms)
                // need the identity to load their per-controller calibration.
                for entry in routes.values() {
                    let _ = entry.events.send(PadEvent::Connected {
                        name: name.clone(),
                        path: path.clone(),
                        vendor,
                        product,
                        version,
                    });
                }
                // Resume the routed session: the disconnect above froze it
                // (releasing stuck outputs), and retarget stays silent here
                // because the routing target never changed.
                notify_reconnect(&routes, &routed, &pressed);
                broadcast_motion(&routes, pad.motion_alive());
            }
        }
        // Pad present but still motion-less: probe again on a backing-off
        // cadence. The connect-time probe races udev and SDL's enumeration,
        // so a controller switched on after the daemon is ready would
        // otherwise stay motion-less until it reconnects.
        if subscribed
            && pad.gamepad.is_some()
            && !pad.motion_alive()
            && pad.motion_retry_at.elapsed() >= pad.motion_retry_delay
        {
            pad.retry_motion(&routes);
        }
        if subscribed && pad.gamepad.is_some() {
            read_pad(&mut pad, &routes, &routed, &mut pressed, &last_live);
        } else if let Some(switch) = pad.switch_hidraw.as_mut() {
            // A Switch-protocol takeover needs its periodic servicing even
            // while nothing is routed; that path reads hidraw passively, so
            // the desktop's evdev view of the pad is unaffected.
            switch.service();
        }
        retarget(&mut routes, &mut routed, &pressed, &mut last_live);
        park(&pad, subscribed);
    }
}

/// Parks until the pad has evdev input or the cadence timeout elapses.
/// Without subscribed sessions the pad is not being read, so its descriptor
/// must not be polled — it would report ready forever and spin the loop.
fn park(pad: &PhysicalPad, subscribed: bool) {
    let timeout = if pad.switch_hidraw.is_some() {
        SWITCH_POLL
    } else if pad.sensor.is_some() {
        SENSOR_POLL
    } else {
        IDLE_POLL
    };
    if !subscribed {
        std::thread::sleep(timeout);
        return;
    }
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
/// `Live` (with the physical pressed set for reconciliation), the one that
/// loses it gets `Frozen`.
fn retarget(
    routes: &mut HashMap<u64, RouteEntry>,
    routed: &mut Option<u64>,
    pressed: &HashSet<InputSource>,
    last_live: &mut Option<u64>,
) {
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
            let _ = entry.events.send(PadEvent::Live {
                pressed: pressed.iter().copied().collect(),
            });
        }
        *last_live = Some(new);
    }
    *routed = target;
}

/// Freezes the routed session on physical disconnect so it releases held
/// outputs and drops stale input. No-op when nothing is routed.
fn notify_disconnect(routes: &HashMap<u64, RouteEntry>, routed: &Option<u64>) {
    let Some(id) = routed else { return };
    if let Some(entry) = routes.get(id) {
        let _ = entry.events.send(PadEvent::Frozen);
    }
}

/// Resumes the routed session on (re)connect: the disconnect froze it, and
/// retarget stays silent because the routing target never changed. Carries
/// the physical pressed set like a routing `Live`. No-op when nothing is
/// routed.
fn notify_reconnect(
    routes: &HashMap<u64, RouteEntry>,
    routed: &Option<u64>,
    pressed: &HashSet<InputSource>,
) {
    let Some(id) = routed else { return };
    if let Some(entry) = routes.get(id) {
        let _ = entry.events.send(PadEvent::Live {
            pressed: pressed.iter().copied().collect(),
        });
    }
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
    pending_stops: &mut HashMap<u64, Instant>,
    pressed: &HashSet<InputSource>,
) -> bool {
    pending_stops.retain(|_, requested_at| requested_at.elapsed() < PENDING_STOP_TTL);
    loop {
        match commands.try_recv() {
            // Every handle is gone: a standalone session finished and its
            // private hub has no reason to outlive it.
            Err(TryRecvError::Disconnected) => return false,
            Err(TryRecvError::Empty) => return true,
            Ok(HubCommand::SetPassthrough { id, passthrough }) => {
                if let Some(route) = routes.get_mut(&id) {
                    route.passthrough = passthrough;
                }
            }
            Ok(HubCommand::Subscribe(subscribe)) => {
                let events = subscribe.events.clone();
                // A Stop for a session that has not subscribed yet (the
                // daemon stops a desktop session whose spawn is still
                // settling) must not be lost: answer the subscribe with
                // the stop so the session exits instead of running forever
                // unrouted.
                let stop_pending = pending_stops.remove(&subscribe.id).is_some();
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
                        passthrough: false,
                    },
                );
                // Bring the newcomer up to speed synchronously: current
                // motion state and, when the pad is already open, its
                // identity — the session builds its output stack for the
                // right controller kind before spawning its game.
                let snapshot = PadSnapshot {
                    motion: pad.motion_alive(),
                    switch_protocol: pad.switch_hidraw.is_some(),
                    pad: pad.gamepad.as_ref().map(|gamepad| {
                        let info = gamepad.info();
                        (
                            info.name.clone(),
                            info.path.display().to_string(),
                            info.vendor,
                            info.product,
                            info.version,
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
                        version: info.version,
                    });
                }
                // Only after the handshake: the session's setup is still
                // waiting on the snapshot reply above.
                if stop_pending {
                    let _ = events.send(PadEvent::Stop);
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
            Ok(HubCommand::GetPressed { reply }) => {
                let _ = reply.send(pressed.clone());
            }
            Ok(HubCommand::ReloadProfile { id, path }) => {
                if let Some(entry) = routes.get(&id) {
                    let _ = entry.events.send(PadEvent::ReloadProfile(path));
                }
            }
            Ok(HubCommand::Stop(id)) => {
                match routes.get(&id) {
                    Some(entry) => {
                        let _ = entry.events.send(PadEvent::Stop);
                    }
                    None => {
                        // Not subscribed yet; held until it does.
                        pending_stops.insert(id, Instant::now());
                    }
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
/// Physical button truth is tracked on every batch — routed or drained —
/// for `Live` reconciliation. Releases arriving with no routing target go
/// to the last live session instead of the void: a press it mapped must
/// not lose its release to a focus gap and stick forever.
fn read_pad(
    pad: &mut PhysicalPad,
    routes: &HashMap<u64, RouteEntry>,
    routed: &Option<u64>,
    pressed: &mut HashSet<InputSource>,
    last_live: &Option<u64>,
) {
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
        // is mapped anywhere — except releases, which the last live
        // session may still be waiting for.
        if let Some(switch) = pad.switch_hidraw.as_mut() {
            forward_releases(switch.take_events(), routes, last_live, pressed);
        } else if let Some(gamepad) = pad.gamepad.as_mut() {
            match gamepad.fetch_events() {
                Ok(batch) => forward_releases(batch, routes, last_live, pressed),
                Err(error) => eprintln!("hub: failed to read controller: {error}"),
            }
        }
        return;
    };
    let events = &target.events;
    if let Some(switch) = pad.switch_hidraw.as_mut() {
        let batch: Vec<InputEvent> = switch.take_events();
        track_presses(&batch, pressed);
        for event in batch {
            let _ = events.send(PadEvent::Input(event));
        }
        if let Some(sample) = switch.take_sample(now_us()) {
            let _ = events.send(PadEvent::Sample(sample));
        }
    } else if let Some(gamepad) = pad.gamepad.as_mut() {
        match gamepad.fetch_events() {
            Ok(batch) => {
                track_presses(&batch, pressed);
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

/// Records the physical button pressed set from one batch. Buttons only:
/// axes are level-semantic and need no reconciliation. Mirrors the mapping
/// engine's button edge (`BUTTON_THRESHOLD`).
fn track_presses(batch: &[InputEvent], pressed: &mut HashSet<InputSource>) {
    for event in batch {
        if !matches!(event.source, InputSource::Button(_)) {
            continue;
        }
        if event.value > 0.5 {
            pressed.insert(event.source);
        } else {
            pressed.remove(&event.source);
        }
    }
}

/// Tracks physical truth, then forwards release edges to the last live
/// session and drops the rest. Best-effort: a gone session drops its copy.
fn forward_releases(
    batch: Vec<InputEvent>,
    routes: &HashMap<u64, RouteEntry>,
    last_live: &Option<u64>,
    pressed: &mut HashSet<InputSource>,
) {
    track_presses(&batch, pressed);
    let Some(id) = last_live else {
        return;
    };
    let Some(entry) = routes.get(id) else {
        return;
    };
    for event in batch {
        if matches!(event.source, InputSource::Button(_)) && event.value <= 0.5 {
            let _ = entry.events.send(PadEvent::Input(event));
        }
    }
}


fn broadcast_motion(routes: &HashMap<u64, RouteEntry>, available: bool) {
    for entry in routes.values() {
        let _ = entry.events.send(PadEvent::Motion(available));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(focused: bool, live_always: bool, focus_seq: u64) -> RouteEntry {
        let (tx, _rx) = channel();
        RouteEntry {
            events: tx,
            focused,
            live_always,
            always_claims_focus: false,
            focus_seq,
            passthrough: false,
        }
    }

    fn routed_entry() -> (HashMap<u64, RouteEntry>, Option<u64>, Receiver<PadEvent>) {
        let (tx, rx) = channel();
        let mut routes = HashMap::new();
        routes.insert(
            7,
            RouteEntry {
                events: tx,
                focused: true,
                live_always: false,
                always_claims_focus: false,
                focus_seq: 1,
                passthrough: false,
            },
        );
        (routes, Some(7), rx)
    }

    #[test]
    fn test_pick_target_routes_to_the_focused_session() {
        let mut routes = HashMap::new();
        routes.insert(1, entry(false, false, 0));
        routes.insert(2, entry(true, false, 1));
        assert_eq!(pick_target(&routes), Some(2));
    }

    #[test]
    fn test_pick_target_prefers_the_latest_focus_claim() {
        let mut routes = HashMap::new();
        routes.insert(1, entry(true, false, 1));
        routes.insert(2, entry(true, false, 2));
        assert_eq!(pick_target(&routes), Some(2));
    }

    #[test]
    fn test_pick_target_lone_live_always_session_stays_routed_unfocused() {
        let mut routes = HashMap::new();
        routes.insert(1, entry(false, true, 0));
        assert_eq!(pick_target(&routes), Some(1));
    }

    #[test]
    fn test_pick_target_paused_lone_session_routes_nothing() {
        // A focus-watched session that has not claimed focus (the game
        // launched focused but the first reading has not arrived, or the
        // game is unfocused) must not be routed: frozen, not live.
        let mut routes = HashMap::new();
        routes.insert(1, entry(false, false, 0));
        assert_eq!(pick_target(&routes), None);
    }

    #[test]
    fn test_pick_target_multi_session_without_focus_routes_nothing() {
        // The live-always fallback only covers a single session; with two
        // games and no focus signal there is no honest owner of the pad.
        let mut routes = HashMap::new();
        routes.insert(1, entry(false, true, 0));
        routes.insert(2, entry(false, false, 0));
        assert_eq!(pick_target(&routes), None);
    }

    #[test]
    fn test_disconnect_freezes_the_routed_session() {
        // Arrange: one routed session with a live receiver.
        let (routes, routed, rx) = routed_entry();
        // Act
        notify_disconnect(&routes, &routed);
        // Assert: it is told to release held outputs.
        assert!(
            matches!(rx.try_recv(), Ok(PadEvent::Frozen)),
            "routed session must freeze on disconnect"
        );
    }

    #[test]
    fn test_disconnect_with_nothing_routed_sends_nothing() {
        // Arrange: no routing target.
        let (routes, _, rx) = routed_entry();
        let empty: HashSet<InputSource> = HashSet::new();
        // Act: no panic, no event.
        notify_disconnect(&routes, &None);
        notify_reconnect(&routes, &None, &empty);
        // Assert
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_reconnect_resumes_the_routed_session() {
        // Arrange
        let (routes, routed, rx) = routed_entry();
        let empty: HashSet<InputSource> = HashSet::new();
        // Act
        notify_reconnect(&routes, &routed, &empty);
        // Assert: retarget stays silent on reconnect (target unchanged),
        // so the hub must resume the session explicitly.
        assert!(
            matches!(rx.try_recv(), Ok(PadEvent::Live { .. })),
            "routed session must resume on reconnect"
        );
    }

    #[test]
    fn test_retarget_live_carries_physical_pressed_set() {
        // Arrange: session 7 focused and unrouted, A physically held.
        let (mut routes, _, rx) = routed_entry();
        let mut routed = None;
        let pressed: HashSet<InputSource> =
            [crate::InputSource::Button(crate::GamepadButton::A)]
                .into_iter()
                .collect();
        let mut last_live = None;
        // Act
        retarget(&mut routes, &mut routed, &pressed, &mut last_live);
        // Assert: routed with the pressed set for reconciliation.
        assert_eq!(routed, Some(7));
        assert_eq!(last_live, Some(7));
        match rx.try_recv() {
            Ok(PadEvent::Live { pressed: got }) => {
                assert_eq!(
                    got,
                    vec![crate::InputSource::Button(crate::GamepadButton::A)]
                );
            }
            other => panic!("expected Live with pressed set, got {other:?}"),
        }
    }

    #[test]
    fn test_drain_forwards_releases_to_last_live_session() {
        // Arrange: last live session 7 with a receiver; batch holds a
        // press, a release, and axis noise.
        let (routes, _, rx) = routed_entry();
        let last_live = Some(7);
        let mut pressed: HashSet<InputSource> =
            [crate::InputSource::Button(crate::GamepadButton::A)]
                .into_iter()
                .collect();
        let batch = vec![
            InputEvent {
                source: crate::InputSource::Button(crate::GamepadButton::B),
                value: 1.0,
                timestamp_us: 0,
            },
            InputEvent {
                source: crate::InputSource::Button(crate::GamepadButton::A),
                value: 0.0,
                timestamp_us: 1,
            },
            InputEvent {
                source: crate::InputSource::Axis(crate::GamepadAxis::LeftX),
                value: 0.5,
                timestamp_us: 2,
            },
        ];
        // Act
        forward_releases(batch, &routes, &last_live, &mut pressed);
        // Assert: only the release is forwarded; truth tracked (B held, A up).
        match rx.try_recv() {
            Ok(PadEvent::Input(event)) => {
                assert_eq!(event.source, crate::InputSource::Button(crate::GamepadButton::A));
                assert_eq!(event.value, 0.0);
            }
            other => panic!("expected the release forwarded, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
        assert!(pressed.contains(&crate::InputSource::Button(crate::GamepadButton::B)));
        assert!(!pressed.contains(&crate::InputSource::Button(crate::GamepadButton::A)));
    }

    #[test]
    fn test_drain_without_last_live_tracks_truth_and_drops() {
        // Arrange: nobody ever routed.
        let routes: HashMap<u64, RouteEntry> = HashMap::new();
        let mut pressed = HashSet::new();
        let batch = vec![InputEvent {
            source: crate::InputSource::Button(crate::GamepadButton::A),
            value: 1.0,
            timestamp_us: 0,
        }];
        // Act: no panic, nothing sent anywhere.
        forward_releases(batch, &routes, &None, &mut pressed);
        // Assert: truth still tracked for the next `Live`.
        assert!(pressed.contains(&crate::InputSource::Button(crate::GamepadButton::A)));
    }
}
