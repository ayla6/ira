use std::io::BufRead;

use std::sync::mpsc::Receiver;

use crate::{MappingEngine, ReportRateEstimator, VirtualGamepad, VirtualKeyboard, VirtualMouse};

use super::devices::{
    apply_controller_deadzone, is_real_profile, load_profile_or_default, make_gyro_processor,
};
use super::loop_io::LoopSchedule;
use super::sensor::{tick_needed_for, SensorPipeline};
use super::super::hub::{HubCommand, HubHandle, PadEvent, Subscribe};
use super::super::{
    ignored_device_for_target, inject_flatpak_target_env, sdl_mapping_for_backend, Arguments,
    ProfileMonitor, SessionEvent, SteamWatcher, TraceState,
};

/// Everything the mapping loop needs, opened once before the event loop
/// starts. Field names mirror the locals the loop body has always used.
pub(crate) struct SessionSetup {
    pub(crate) trace: TraceState,
    pub(crate) hub: HubHandle,
    /// Pad events routed to this session by the hub.
    pub(crate) pad_events: Receiver<PadEvent>,
    pub(crate) mapper: MappingEngine,
    pub(crate) profile_monitor: Option<ProfileMonitor>,
    pub(crate) keyboard: Option<VirtualKeyboard>,
    pub(crate) mouse: Option<VirtualMouse>,
    pub(crate) virtual_gamepad: VirtualGamepad,
    pub(crate) pad_state: crate::PadState,
    pub(crate) pipeline: SensorPipeline,
    pub(crate) motion_enabled: bool,
    pub(crate) pad_vendor: u16,
    pub(crate) pad_product: u16,
    pub(crate) tick_needed: bool,
    pub(crate) report_rate: ReportRateEstimator,
    pub(crate) steam_watch: Option<SteamWatcher>,
    pub(crate) launcher_exit_code: Option<i32>,
    pub(crate) child: Option<std::process::Child>,
    pub(crate) schedule: LoopSchedule,
    pub(crate) focus: Option<crate::FocusWatcher>,
    pub(crate) paused_for_focus: bool,
    pub(crate) cursor_watcher: Option<crate::CursorWatcher>,
    pub(crate) cursor_visible: Option<bool>,
}


/// Spawns the game process. The legacy wrapper passes nothing but the
/// command line: the child inherits the wrapper's environment (which the
/// launcher already built). A daemon session carries the full environment
/// and working directory in the request and streams the child's output back
/// to clients, because the game log lives in the app, not here.
fn spawn_session_child(
    arguments: &Arguments,
    mapper: &MappingEngine,
    pad_identity: Option<(u16, u16)>,
) -> Option<std::process::Child> {
    if arguments.command.is_empty() {
        return None;
    }
    let mut target_args = arguments.command[1..].to_vec();
    inject_flatpak_target_env(
        &arguments.command[0],
        &mut target_args,
        mapper.profile().backend,
        pad_identity.map(|(vendor, _)| vendor),
        pad_identity.map(|(_, product)| product),
    );
    let mut command = std::process::Command::new(&arguments.command[0]);
    command.args(target_args);
    command.env("SDL_JOYSTICK_HIDAPI", "0");
    if let Some(mapping) = sdl_mapping_for_backend(mapper.profile().backend) {
        command.env("SDL_GAMECONTROLLERCONFIG", mapping);
    }
    if let Some((vendor, product)) = pad_identity {
        if let Some(ignored_device) = ignored_device_for_target(
            vendor,
            product,
            mapper.profile().backend,
        ) {
            command.env("SDL_GAMECONTROLLER_IGNORE_DEVICES", ignored_device);
        }
    }
    if let Some(env) = &arguments.env {
        // The request environment is complete: the launcher's list already
        // excludes everything it wants filtered, so start from zero rather
        // than leaking the daemon's own desktop environment into the game.
        command.env_clear();
        for (key, value) in env {
            command.env(key, value);
        }
    }
    if let Some(dir) = &arguments.working_dir {
        command.current_dir(dir);
    }
    if arguments.events.is_some() {
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!(
                "failed to launch target process {}: {error}",
                arguments.command[0]
            );
            eprintln!("ira-input: {message}");
            return None;
        }
    };
    if let Some(events) = &arguments.events {
        let _ = events.send(SessionEvent::SessionStarted {
            child_pid: child.id() as i32,
            command: arguments.command.clone(),
        });
        pump_output(child.stdout.take(), events);
        pump_output(child.stderr.take(), events);
    }
    Some(child)
}

/// Forwards the game's stdout/stderr to clients line by line. The pipes EOF
/// when the child exits, so the pump threads end on their own.
fn pump_output<R: std::io::Read + Send + 'static>(
    reader: Option<R>,
    events: &std::sync::mpsc::Sender<SessionEvent>,
) {
    if let Some(reader) = reader {
        let events = events.clone();
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(reader);
            for line in reader.lines().map_while(Result::ok) {
                if events.send(SessionEvent::Output(line)).is_err() {
                    break;
                }
            }
        });
    }
}

pub(crate) fn setup_session(arguments: &Arguments) -> Result<SessionSetup, String> {

    let trace = TraceState::new(arguments.trace);
    // A broken profile or unavailable uinput must never keep the game from
    // starting: degrade to the builtin layout, or to shadow-only devices,
    // and say so on stderr.
    let profile = load_profile_or_default(arguments.profile.as_deref());
    let mut mapper = MappingEngine::new(profile)?;
    let profile_monitor = arguments
        .profile
        .as_ref()
        .filter(|path| is_real_profile(path))
        .map(|path| ProfileMonitor::new(path.clone()));
    let profile_name = arguments
        .profile
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "builtin:default_gamepad".to_string());
    let mapped_inputs = mapper
        .profile()
        .action_sets
        .iter()
        .map(|set| set.inputs.len())
        .sum::<usize>();
    eprintln!("ira-input: loaded {mapped_inputs} mapped inputs from {profile_name}");
    // Subscribe to the pad hub — the daemon's shared one, or a private one
    // for a standalone (`--no-daemon`) session — and learn the pad's state
    // synchronously so the output stack matches the controller kind.
    let hub = match &arguments.hub {
        Some(hub) => hub.clone(),
        None => {
            let (presence, _presence_rx) = std::sync::mpsc::channel();
            super::super::hub::spawn(presence)
        }
    };
    let (pad_events_tx, pad_events_rx) = std::sync::mpsc::channel();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    hub.send(HubCommand::Subscribe(Subscribe {
        id: arguments.session_id,
        events: pad_events_tx.clone(),
        live_always: !arguments.pause_unfocused,
        calibration: arguments.calibration.clone(),
        device: arguments.device.clone(),
        reply: reply_tx,
    }));
    let snapshot = reply_rx
        .recv()
        .map_err(|_| "pad hub is gone".to_string())?;
    let motion_available = snapshot.motion;
    let stack = super::output_stack::build_virtual_stack(
        motion_available,
        arguments.motion_port != Some(0),
        mapper.profile(),
    );
    let (pad_vendor, pad_product) = snapshot
        .pad
        .as_ref()
        .map(|(_, _, vendor, product)| (*vendor, *product))
        .unwrap_or((0, 0));
    let gyro_processor = make_gyro_processor(
        mapper.profile(),
        pad_vendor,
        pad_product,
        arguments.calibration.as_deref(),
    );
    apply_controller_deadzone(
        &mut mapper,
        arguments.calibration.as_deref(),
        pad_vendor,
        pad_product,
    );
    let last_sensor_us: Option<u64> = None;
    let pipeline = SensorPipeline {
        motion_available,
        gyro_processor,
        last_sensor_us,
        motion: stack.motion,
        motion_device: stack.motion_device,
        ds4_hid: stack.ds4_hid,
        dualsense_hid: stack.dualsense_hid,
        imu_hid: stack.imu_hid,
        switch_pro_hid: stack.switch_pro_hid,
        ever_had_sensor: motion_available,
        last_dsu_ts: 0,
    };
    let tick_needed = tick_needed_for(&pipeline, &mapper);
    let report_rate = ReportRateEstimator::default();
    let steam_watch = arguments.steam_app_id.as_deref().map(SteamWatcher::spawn);
    let launcher_exit_code = None;
    let child = spawn_session_child(
        arguments,
        &mapper,
        snapshot.pad.as_ref().map(|(_, _, vendor, product)| (*vendor, *product)),
    );
    let pad_state = crate::PadState::default();
    let schedule = LoopSchedule::new();
    // Pause injection while the game window is unfocused (alt-tab). Without
    // an X server to ask, the watcher cannot exist and input stays active.
    let focus = if arguments.pause_unfocused {
        child
            .as_ref()
            .and_then(|child| crate::FocusWatcher::for_child(child.id()))
    } else {
        None
    };
    if arguments.pause_unfocused && focus.is_none() {
        eprintln!(
            "ira-input: no X server available for focus tracking; input stays active while unfocused"
        );
    }
    if focus.is_none() {
        // No focus watcher means no focus signal will ever come: claim the
        // hub routing outright.
        hub.send(HubCommand::Focus {
            id: arguments.session_id,
            focused: true,
        });
    }
    let paused_for_focus = false;
    // Cursor-driven set switching (Steam Input's "action set when the mouse
    // cursor is shown/hidden"). Same X11 limits as focus tracking: without
    // an X server the cursor always reads as visible and switching stays off.
    let cursor_watcher = if mapper.profile().action_set_when_cursor_shown.is_some()
        || mapper.profile().action_set_when_cursor_hidden.is_some()
    {
        match crate::CursorWatcher::create() {
            Some(watcher) => Some(watcher),
            None => {
                eprintln!(
                    "ira-input: no X server available for cursor tracking; \
                     cursor set switching disabled"
                );
                None
            }
        }
    } else {
        None
    };
    let cursor_visible: Option<bool> = None;


    // Profile saves wake the session channel the moment they land, so a
    // reload applies instantly even while the loop is parked on the hub.
    if let Some(fd) = profile_monitor.as_ref().and_then(|monitor| monitor.fd()) {
        let wake_tx = pad_events_tx.clone();
        let _ = std::thread::Builder::new()
            .name("ira-profile-wake".to_string())
            .spawn(move || profile_wake_pump(fd, wake_tx));
    }
    Ok(SessionSetup {
        trace,
        hub,
        pad_events: pad_events_rx,
        mapper,
        profile_monitor,
        keyboard: stack.keyboard,
        mouse: stack.mouse,
        virtual_gamepad: stack.gamepad,
        pad_state,
        pipeline,
        motion_enabled: stack.motion_enabled,
        pad_vendor,
        pad_product,
        tick_needed,
        report_rate,
        steam_watch,
        launcher_exit_code,
        child,
        schedule,
        focus,
        paused_for_focus,
        cursor_watcher,
        cursor_visible,
    })
}

/// Parks on the profile watcher's inotify descriptor and pushes a wake into
/// the session's pad channel for every change. When the session ends, the
/// watcher fd closes and the failed send retires the pump.
fn profile_wake_pump(fd: libc::c_int, events: std::sync::mpsc::Sender<PadEvent>) {
    loop {
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 250) };
        if ready > 0 && events.send(PadEvent::ProfileChanged).is_err() {
            return;
        }
        if ready < 0 {
            return;
        }
    }
}
