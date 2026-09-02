use crate::{
    MappingEngine, PhysicalGamepad, ReportRateEstimator, VirtualGamepad, VirtualKeyboard,
    VirtualMouse,
};

use super::devices::{
    apply_controller_deadzone, apply_controller_layout, create_keyboard, create_mouse,
    is_real_profile, load_profile_or_default, make_gyro_processor, open_initial_gamepad,
    open_rumble, open_sensor, resolved_layout_for,
};
use super::loop_io::LoopSchedule;
use super::sensor::{open_motion_node, spawn_paired_imu, SensorPipeline};
use super::super::{
    ignored_device_for_target, inject_flatpak_target_env, sdl_mapping_for_backend,
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
    let keyboard = match create_keyboard(mapper.profile().keyboard_keycodes()) {
        Ok(keyboard) => keyboard,
        Err(error) => {
            eprintln!("ira-input: {error}; keyboard output disabled, the game still launches");
            None
        }
    };
    let mouse = match create_mouse(mapper.profile().uses_mouse()) {
        Ok(mouse) => mouse,
        Err(error) => {
            eprintln!("ira-input: {error}; mouse output disabled, the game still launches");
            None
        }
    };
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
    let native_transport = motion_available && mapper.profile().wants_native_controller();
    let native_ds4 =
        native_transport && mapper.profile().backend == crate::VirtualGamepadBackend::DualShock4;
    let native_switch_pro =
        native_transport && mapper.profile().backend == crate::VirtualGamepadBackend::SwitchPro;
    let native_dualsense =
        native_transport && mapper.profile().backend == crate::VirtualGamepadBackend::DualSense;
    let virtual_gamepad = if native_ds4 || native_switch_pro || native_dualsense {
        eprintln!("ira-input: uinput gamepad suppressed; the uhid controller is the controller");
        VirtualGamepad::shadow_only(mapper.profile().backend)
    } else {
        match VirtualGamepad::create_for_backend(mapper.profile().backend) {
            Ok(virtual_gamepad) => virtual_gamepad,
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual gamepad: {error}; \
                     gamepad output disabled, the game still launches"
                );
                VirtualGamepad::shadow_only(mapper.profile().backend)
            }
        }
    };
    // The cemuhook stream is the DSU backend itself: picking that output
    // mode always streams, nothing toggles it. A profile on any other
    // backend can opt into a motion-only companion stream (`dsu_motion`)
    // for emulators that bind the DSU provider purely as a motion source;
    // its frames read neutral to a client that wants buttons. The
    // per-game launcher flag (--motion-port 0) remains the harder kill
    // switch. It works even when no gyro exists (motion just reads zero).
    let motion_enabled = arguments.motion_port != Some(0)
        && (mapper.profile().backend == crate::VirtualGamepadBackend::Dsu
            || mapper.profile().dsu_motion);
    let motion_server = if motion_enabled {
        crate::MotionServer::bind()
    } else {
        None
    };
    match (&motion_server, sensor.as_ref()) {
        (Some(_), Some(_)) => eprintln!(
            "ira-input: motion passthrough on udp/{} for emulators (cemuhook)",
            crate::MOTION_PORT
        ),
        (None, Some(_)) if arguments.motion_port != Some(0) => {
            eprintln!(
                "ira-input: udp/{} busy; motion passthrough disabled",
                crate::MOTION_PORT
            )
        }
        _ => {}
    }
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
    // The motion node must exist before the game opens the virtual pad:
    // SDL pairs sensor nodes with a pad at open time only.
    let motion_device = if motion_available && mapper.profile().native_motion {
        open_motion_node(mapper.profile().backend)
    } else {
        None
    };
    // Experimental whole-HID DualShock4: a real hidraw device whose reports
    // carry buttons, sticks, triggers AND motion, so SDL's DS4 driver reads
    // our sensors natively. A companion motion-only HID shares its serial:
    // SDL's evdev backend pairs them, which is the half flatpaks can see.
    let mut ds4_hid = None;
    let mut imu_hid = None;
    if native_ds4 {
        let uniq = format!("ira-virtual-{}", std::process::id());
        match crate::Ds4UhidDevice::create(&uniq) {
            Ok(device) => {
                eprintln!("ira-input: experimental native-motion DS4 exposed over hidraw");
                ds4_hid = Some(device);
                imu_hid = spawn_paired_imu(&uniq);
            }
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual DS4: {error}; \
                     /dev/uhid is root-only unless a uaccess rule grants it"
                );
            }
        }
    }
    let ever_had_sensor = motion_available;
    // A virtual *real* Switch Pro: hid-nintendo claims it, completes its
    // handshake against our answers and builds an IMU input node itself.
    // That kernel IMU carries no usable serial though, so SDL cannot pair
    // it with anything — the paired twin below is what delivers gyro.
    let mut switch_pro_hid = None;
    if native_switch_pro {
        let uniq = format!("ira-virtual-{}", std::process::id());
        match crate::SwitchProUhidDevice::create(&uniq) {
            Ok(device) => {
                eprintln!("ira-input: virtual Switch Pro claimed by hid-nintendo");
                switch_pro_hid = Some(device);
                imu_hid = spawn_paired_imu(&uniq);
            }
            Err(error) => {
                eprintln!("ira-input: failed to create virtual Switch Pro: {error}");
            }
        }
    }
    // Same whole-HID approach as the DS4, on SDL's PS5 third-party path:
    // the licensed HORI PID is typed PS5 in SDL's table and feature replies
    // enable sensors with an identity calibration.
    let mut dualsense_hid = None;
    if native_dualsense {
        let uniq = format!("ira-virtual-{}", std::process::id());
        match crate::DualsenseUhidDevice::create(&uniq) {
            Ok(device) => {
                eprintln!("ira-input: experimental native-motion DualSense exposed over hidraw");
                dualsense_hid = Some(device);
                imu_hid = spawn_paired_imu(&uniq);
            }
            Err(error) => {
                eprintln!(
                    "ira-input: failed to create virtual DualSense: {error}; \
                     /dev/uhid is root-only unless a uaccess rule grants it"
                );
            }
        }
    }
    let pipeline = SensorPipeline {
        sensor,
        switch_hidraw,
        gyro_processor,
        last_sensor_us,
        motion: motion_server,
        motion_device,
        ds4_hid,
        dualsense_hid,
        imu_hid,
        switch_pro_hid,
        ever_had_sensor,
        last_dsu_ts: 0,
    };
    // Ticks drive continuous outputs (mouse motion, gyro axes) and must run
    // even when no sensor exists, as long as something consumes them.
    let tick_needed = pipeline.motion_alive()
        || mapper.has_continuous_outputs()
        || mapper.profile().backend == crate::VirtualGamepadBackend::Dsu
        || pipeline.motion.is_some()
        || pipeline.ds4_hid.is_some();
    let report_rate = ReportRateEstimator::default();
    if let Some(gamepad) = gamepad.as_mut() {
        if let Err(error) = gamepad.grab() {
            eprintln!("ira-input: {error}; continuing without exclusive grab");
        }
    }
    let steam_watch = arguments.steam_app_id.as_deref().map(SteamWatcher::spawn);
    let launcher_exit_code = None;
    let child = if arguments.command.is_empty() {
        None
    } else {
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
        Some(command.spawn().map_err(|error| {
            format!(
                "failed to launch target process {}: {error}",
                arguments.command[0]
            )
        })?)
    };
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
        keyboard,
        mouse,
        virtual_gamepad,
        pad_state,
        pipeline,
        rumble_output,
        motion_enabled,
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
