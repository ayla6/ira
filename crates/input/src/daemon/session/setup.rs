use std::io::BufRead;

use crate::{
    MappingEngine, PhysicalGamepad, ReportRateEstimator, VirtualGamepad, VirtualKeyboard,
    VirtualMouse,
};

use super::devices::{
    apply_controller_deadzone, apply_controller_layout, is_real_profile, load_profile_or_default,
    make_gyro_processor, open_initial_gamepad, open_rumble, open_sensor, resolved_layout_for,
};
use super::loop_io::LoopSchedule;
use super::sensor::{tick_needed_for, SensorPipeline};
use super::super::{
    ignored_device_for_target, SessionEvent, inject_flatpak_target_env, sdl_mapping_for_backend,
    Arguments, ProfileMonitor, SteamWatcher, TraceState,
};

/// Everything the mapping loop needs, opened once before the event loop
/// starts. Field names mirror the locals the loop body has always used.
pub(crate) struct SessionSetup {
    pub(crate) trace: TraceState,
    pub(crate) gamepad: Option<PhysicalGamepad>,
    pub(crate) mapper: MappingEngine,
    pub(crate) profile_monitor: Option<ProfileMonitor>,
    pub(crate) keyboard: Option<VirtualKeyboard>,
    pub(crate) mouse: Option<VirtualMouse>,
    pub(crate) virtual_gamepad: VirtualGamepad,
    pub(crate) pad_state: crate::PadState,
    pub(crate) pipeline: SensorPipeline,
    pub(crate) rumble_output: Option<crate::PhysicalRumble>,
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
    gamepad: &Option<PhysicalGamepad>,
) -> Option<std::process::Child> {
    if arguments.command.is_empty() {
        return None;
    }
    let mut target_args = arguments.command[1..].to_vec();
    inject_flatpak_target_env(
        &arguments.command[0],
        &mut target_args,
        mapper.profile().backend,
        gamepad.as_ref().map(|gamepad| gamepad.info().vendor),
        gamepad.as_ref().map(|gamepad| gamepad.info().product),
    );
    let mut command = std::process::Command::new(&arguments.command[0]);
    command.args(target_args);
    command.env("SDL_JOYSTICK_HIDAPI", "0");
    if let Some(mapping) = sdl_mapping_for_backend(mapper.profile().backend) {
        command.env("SDL_GAMECONTROLLERCONFIG", mapping);
    }
    if let Some(gamepad) = gamepad.as_ref() {
        if let Some(ignored_device) = ignored_device_for_target(
            gamepad.info().vendor,
            gamepad.info().product,
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
    let mut gamepad = open_initial_gamepad(arguments.device.as_deref());
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
    let sensor = gamepad
        .as_ref()
        .and_then(|gamepad| open_sensor(gamepad.info()));
    // Switch-protocol takeover: when neither the kernel's IMU node nor SDL
    // sources motion, speak the pad's own protocol over hidraw — what SDL
    // does for games, and the only gyro path for Switch-mode pads
    // hid-nintendo has not claimed. It replaces evdev as the input source
    // too, because switching the report mode invalidates what generic HID
    // parses from the descriptor.
    let mut switch_hidraw = sensor
        .is_none()
        .then(|| {
            gamepad
                .as_ref()
                .and_then(|gamepad| crate::SwitchHidrawPad::open(gamepad.info()))
        })
        .flatten();    if let (Some(driver), Some(gamepad)) = (switch_hidraw.as_mut(), gamepad.as_ref()) {
        driver.set_nintendo_layout(resolved_layout_for(
            gamepad.info(),
            arguments.calibration.as_deref(),
        ));
    }
    // When an experimental uhid controller owns the session (DS4 or the
    // hid-nintendo Switch Pro), the uinput pad must not exist too or games
    // see two controllers and often bind the motionless one. Outputs still
    // reach the pad shadow that the hidraw reports and the cemuhook stream
    // read from.
    let motion_available = sensor.is_some() || switch_hidraw.is_some();
    let stack = super::output_stack::build_virtual_stack(
        motion_available,
        arguments.motion_port != Some(0),
        mapper.profile(),
    );
    let (pad_vendor, pad_product) = gamepad
        .as_ref()
        .map(|gamepad| (gamepad.info().vendor, gamepad.info().product))
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
    apply_controller_layout(&mut gamepad, arguments.calibration.as_deref());
    // Game-side rumble replays on the physical controller unless the profile
    // opts out; a controller without force feedback just stays silent. The
    // Switch-protocol driver replays rumble itself, and its report mode
    // makes the vendor DInput packet wrong for the pad, so it stays the
    // only rumble path while active.
    let rumble_output = if switch_hidraw.is_some() {
        None
    } else {
        open_rumble(gamepad.as_ref(), mapper.profile().rumble)
    };
    let last_sensor_us: Option<u64> = None;
    let ever_had_sensor = motion_available;
    let pipeline = SensorPipeline {
        sensor,
        switch_hidraw,
        gyro_processor,
        last_sensor_us,
        motion: stack.motion,
        motion_device: stack.motion_device,
        ds4_hid: stack.ds4_hid,
        dualsense_hid: stack.dualsense_hid,
        imu_hid: stack.imu_hid,
        switch_pro_hid: stack.switch_pro_hid,
        ever_had_sensor,
        last_dsu_ts: 0,
    };
    // Ticks drive continuous outputs (mouse motion, gyro axes) and must run
    // even when no sensor exists, as long as something consumes them.
    let tick_needed = tick_needed_for(&pipeline, &mapper);
    let report_rate = ReportRateEstimator::default();
    if let Some(gamepad) = gamepad.as_mut() {
        if let Err(error) = gamepad.grab() {
            eprintln!("ira-input: {error}; continuing without exclusive grab");
        }
    }
    let steam_watch = arguments.steam_app_id.as_deref().map(SteamWatcher::spawn);
    let launcher_exit_code = None;
    let child = spawn_session_child(arguments, &mapper, &gamepad);
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
    if let Some(gamepad) = gamepad.as_ref() {
        eprintln!(
            "ira-input: mapping {} through {}",
            gamepad.info().name,
            gamepad.info().path.display()
        );
        eprintln!(
            "ira-input: physical SDL device excluded ({:04x}:{:04x})",
            gamepad.info().vendor,
            gamepad.info().product
        );
    } else {
        eprintln!("ira-input: no controller found; waiting for one to be plugged in");
    }


    Ok(SessionSetup {
        trace,
        gamepad,
        mapper,
        profile_monitor,
        keyboard: stack.keyboard,
        mouse: stack.mouse,
        virtual_gamepad: stack.gamepad,
        pad_state,
        pipeline,
        rumble_output,
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
