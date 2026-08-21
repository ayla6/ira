use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ira_input::{
    discover_gamepads, discover_sdl_gamepads, GyroProcessingOptions, GyroProcessor, InputEvent,
    InputProfile, MappingEngine, OutputEvent, PhysicalGamepad, Sdl3SensorBackend, VirtualGamepad,
    VirtualGamepadBackend, VirtualKeyboard, VirtualMouse,
};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

const VIRTUAL_XBOX_VENDOR: u16 = 0x045e;
const VIRTUAL_XBOX_PRODUCT: u16 = 0x028e;
const SWITCH_PRO_VENDOR: u16 = 0x057e;
const SWITCH_PRO_PRODUCT: u16 = 0x2009;
const SENSOR_SAMPLE_INTERVAL: Duration = Duration::from_millis(4);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PROFILE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const STEAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RECONNECT_INTERVAL: Duration = Duration::from_millis(250);
const STEAM_START_TIMEOUT: Duration = Duration::from_secs(60);
const STEAM_EXIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProcessIdentity {
    pid: i32,
    start_time: u64,
}

struct SteamProcessSnapshot {
    processes: HashSet<ProcessIdentity>,
    complete: bool,
}

struct SteamSession {
    app_id: String,
    baseline: HashSet<ProcessIdentity>,
    started_at: Instant,
    seen: bool,
    empty_since: Option<Instant>,
    stop_sent: bool,
}

struct LoopSchedule {
    sensor: Instant,
    process: Instant,
    profile: Instant,
    steam: Instant,
    reconnect: Instant,
}

impl LoopSchedule {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            sensor: now,
            process: now,
            profile: now,
            steam: now,
            reconnect: now,
        }
    }

    fn timeout(
        &self,
        sensor_active: bool,
        child_active: bool,
        profile_active: bool,
        steam_active: bool,
        disconnected: bool,
    ) -> Option<Duration> {
        [
            sensor_active.then(|| remaining(self.sensor, SENSOR_SAMPLE_INTERVAL)),
            child_active.then(|| remaining(self.process, PROCESS_POLL_INTERVAL)),
            profile_active.then(|| remaining(self.profile, PROFILE_POLL_INTERVAL)),
            steam_active.then(|| remaining(self.steam, STEAM_POLL_INTERVAL)),
            disconnected.then(|| remaining(self.reconnect, RECONNECT_INTERVAL)),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

fn remaining(last_run: Instant, interval: Duration) -> Duration {
    interval.saturating_sub(last_run.elapsed())
}

impl SteamSession {
    fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            baseline: steam_processes(app_id).processes,
            started_at: Instant::now(),
            seen: false,
            empty_since: None,
            stop_sent: false,
        }
    }

    fn request_stop(&mut self) {
        if !self.stop_sent {
            request_steam_stop(&self.app_id);
            self.stop_sent = true;
        }
    }

    fn poll(&mut self, launcher_exited: bool) -> bool {
        let snapshot = steam_processes(&self.app_id);
        if !snapshot.complete {
            self.empty_since = None;
            return false;
        }
        let active = snapshot
            .processes
            .difference(&self.baseline)
            .next()
            .is_some();
        if active {
            self.seen = true;
            self.empty_since = None;
            return false;
        }
        if self.seen {
            return self.empty_since.get_or_insert_with(Instant::now).elapsed() >= STEAM_EXIT_GRACE;
        }
        self.stop_sent || (launcher_exited && self.started_at.elapsed() >= STEAM_START_TIMEOUT)
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    STOP_REQUESTED.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }
}

struct Arguments {
    device: Option<PathBuf>,
    profile: Option<PathBuf>,
    list: bool,
    probe_sensors: bool,
    steam_app_id: Option<String>,
    trace: bool,
    command: Vec<String>,
}

struct TraceState {
    enabled: bool,
    last_report: Instant,
    gyro: [f32; 3],
    mouse: [f32; 2],
}

impl TraceState {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_report: Instant::now(),
            gyro: [0.0; 3],
            mouse: [0.0; 2],
        }
    }

    fn record_input(&mut self, event: InputEvent) {
        if self.enabled {
            eprintln!(
                "ira-input: input source={:?} value={:.3}",
                event.source, event.value
            );
        }
    }

    fn record_gyro(&mut self, gyro: [f32; 3]) {
        self.gyro = gyro;
    }

    fn record_output(&mut self, output: &OutputEvent) {
        if self.enabled
            && !matches!(
                output,
                OutputEvent::GamepadAxis { .. } | OutputEvent::MouseMotion { .. }
            )
        {
            eprintln!("ira-input: output {output:?}");
        }
        if let OutputEvent::MouseMotion { axis, value } = output {
            match axis {
                ira_input::MouseAxis::X => self.mouse[0] += value,
                ira_input::MouseAxis::Y => self.mouse[1] += value,
                ira_input::MouseAxis::Wheel | ira_input::MouseAxis::WheelX => {}
            }
        }
    }

    fn flush(&mut self) {
        if !self.enabled || self.last_report.elapsed() < Duration::from_millis(250) {
            return;
        }
        if self.gyro.iter().any(|value| value.abs() > 0.001)
            || self.mouse.iter().any(|value| value.abs() > 0.01)
        {
            eprintln!(
                "ira-input: trace gyro=({:.3}, {:.3}, {:.3}) mouse_delta=({:.2}, {:.2})",
                self.gyro[0],
                self.gyro[1],
                self.gyro[2],
                self.mouse[0],
                self.mouse[1],
            );
        }
        self.last_report = Instant::now();
        self.gyro = [0.0; 3];
        self.mouse = [0.0; 2];
    }
}

fn main() {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("ira-input: {error}");
            eprintln!(
                "usage: ira-input --list | [--device PATH] [--profile PATH] [--steam-app-id ID] [--trace] -- COMMAND"
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
    match run_session(arguments) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ira-input: {error}");
            std::process::exit(1);
        }
    }
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut arguments = Arguments {
        device: None,
        profile: None,
        list: false,
        probe_sensors: false,
        steam_app_id: None,
        trace: false,
        command: Vec::new(),
    };
    let mut values = std::env::args().skip(1);
    while let Some(argument) = values.next() {
        if argument == "--" {
            arguments.command.extend(values);
            break;
        }
        match argument.as_str() {
            "--device" => {
                arguments.device = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--device requires a path".to_string())?,
                ));
            }
            "--profile" => {
                arguments.profile = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--profile requires a path".to_string())?,
                ));
            }
            "--steam-app-id" => {
                arguments.steam_app_id = Some(
                    values
                        .next()
                        .ok_or_else(|| "--steam-app-id requires an ID".to_string())?,
                );
            }
            "--list" => arguments.list = true,
            "--probe-sensors" => arguments.probe_sensors = true,
            "--trace" => arguments.trace = true,
            "--help" | "-h" => {
                println!(
                    "usage: ira-input --list | [--device PATH] [--profile PATH] [--steam-app-id ID] [--trace] -- COMMAND"
                );
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument {unknown}")),
        }
    }
    Ok(arguments)
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
        match Sdl3SensorBackend::open(&device) {
            Ok(Some(mut sensor)) => {
                println!("  SDL3 gyro: available");
                for _ in 0..5 {
                    thread::sleep(Duration::from_millis(20));
                    match sensor.read(now_us()) {
                        Ok(Some(sample)) => println!(
                            "  sample: x={:.5} y={:.5} z={:.5} accel={:?}",
                            sample.gyro[0],
                            sample.gyro[1],
                            sample.gyro[2],
                            sample.accel
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

fn run_session(arguments: Arguments) -> Result<i32, String> {
    STOP_REQUESTED.store(false, Ordering::Relaxed);
    install_signal_handlers();
    let mut trace = TraceState::new(arguments.trace);
    let mut gamepad = open_initial_gamepad(arguments.device.as_deref());
    let profile = load_profile(arguments.profile.as_deref())?;
    let mut mapper = MappingEngine::new(profile)?;
    let mut profile_monitor = arguments
        .profile
        .as_ref()
        .filter(|path| is_real_profile(path))
        .map(|path| ProfileMonitor::new(path.clone()));
    let profile_name = arguments
        .profile
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "builtin:default_gamepad".to_string());
    eprintln!(
        "ira-input: loaded {} bindings from {}",
        mapper.profile().bindings.len(),
        profile_name
    );
    let mut keyboard = create_keyboard(mapper.profile().keyboard_keycodes())?;
    let mut mouse = create_mouse(mapper.profile().uses_mouse())?;
    let mut virtual_gamepad = VirtualGamepad::create_for_backend(mapper.profile().backend)
        .map_err(|error| format!("failed to create virtual gamepad: {error}"))?;
    let mut sensor = gamepad.as_ref().and_then(|gamepad| open_sensor(gamepad.info()));
    let mut gyro_processor = make_gyro_processor(mapper.profile());
    let mut last_sensor_us: Option<u64> = None;
    // Ticks drive continuous outputs (mouse motion, gyro axes) and must run
    // even when no sensor exists, as long as something consumes them.
    let mut tick_needed = sensor.is_some() || mapper.has_continuous_outputs();
    if let Some(gamepad) = gamepad.as_mut() {
        if let Err(error) = gamepad.grab() {
            eprintln!("ira-input: {error}; continuing without exclusive grab");
        }
    }
    let mut steam_session = arguments.steam_app_id.as_deref().map(SteamSession::new);
    let mut launcher_exit_code = None;
    let mut child = if arguments.command.is_empty() {
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
        Some(
            command
                .spawn()
                .map_err(|error| format!("failed to launch target process: {error}"))?,
        )
    };
    let mut schedule = LoopSchedule::new();
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
    loop {
        let was_connected = gamepad.as_ref().is_some_and(|gamepad| gamepad.is_connected());
        let run_tick = tick_needed && schedule.sensor.elapsed() >= SENSOR_SAMPLE_INTERVAL;
        if run_tick {
            schedule.sensor = Instant::now();
        }
        let result = process_physical_inputs(
            &mut gamepad,
            &mut mapper,
            OutputTargets {
                gamepad: &mut virtual_gamepad,
                keyboard: keyboard.as_mut(),
                mouse: mouse.as_mut(),
            },
            &mut trace,
        )
        .and_then(|()| {
            process_tick(
                &mut sensor,
                &mut gyro_processor,
                &mut last_sensor_us,
                &mut mapper,
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                },
                &mut trace,
                run_tick,
            )
        });
        trace.flush();
        if let Err(error) = result {
            let resets = emit_outputs(
                mapper.reset(),
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                },
                &mut trace,
            );
            if let Err(reset_error) = resets {
                eprintln!("ira-input: failed to emit reset releases: {reset_error}");
            }
            stop_child(&mut child);
            return Err(error);
        }
        let connected = gamepad.as_ref().is_some_and(|gamepad| gamepad.is_connected());
        if was_connected && !connected {
            sensor = None;
            last_sensor_us = None;
            tick_needed = mapper.has_continuous_outputs();
            schedule.reconnect = Instant::now();
            emit_outputs(
                mapper.reset(),
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                },
                &mut trace,
            )?;
        }
        if STOP_REQUESTED.load(Ordering::Relaxed) {
            if let Some(session) = steam_session.as_mut() {
                session.request_stop();
            }
            stop_child(&mut child);
            return Ok(130);
        }
        let child_status = if child.is_some() && schedule.process.elapsed() >= PROCESS_POLL_INTERVAL
        {
            schedule.process = Instant::now();
            child
                .as_mut()
                .map(|child| child.try_wait())
                .transpose()
                .map_err(|error| format!("failed waiting for target process: {error}"))?
                .flatten()
        } else {
            None
        };
        if let Some(status) = child_status {
            let code = status.code().unwrap_or(1);
            child = None;
            if arguments.steam_app_id.is_none() {
                return Ok(code);
            }
            launcher_exit_code = Some(code);
        }
        if steam_session.is_some() && schedule.steam.elapsed() >= STEAM_POLL_INTERVAL {
            schedule.steam = Instant::now();
            if steam_session
                .as_mut()
                .is_some_and(|session| session.poll(launcher_exit_code.is_some()))
            {
                return Ok(launcher_exit_code.unwrap_or(0));
            }
        }
        if !connected && schedule.reconnect.elapsed() >= RECONNECT_INTERVAL {
            schedule.reconnect = Instant::now();
            sensor = None;
            match reconnect_gamepad(&mut gamepad) {
                Ok(true) => {
                    if let Some(gamepad) = gamepad.as_ref() {
                        eprintln!(
                            "ira-input: controller connected through {}",
                            gamepad.info().path.display()
                        );
                        sensor = open_sensor(gamepad.info());
                        last_sensor_us = None;
                        tick_needed = sensor.is_some() || mapper.has_continuous_outputs();
                    }
                    schedule.sensor = Instant::now();
                    if let Some(gamepad) = gamepad.as_mut() {
                        if let Err(error) = gamepad.grab() {
                            eprintln!("ira-input: failed to grab controller: {error}");
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => eprintln!("ira-input: controller reconnect failed: {error}"),
            }
        }
        if profile_monitor.is_some() && schedule.profile.elapsed() >= PROFILE_POLL_INTERVAL {
            schedule.profile = Instant::now();
            if let Some(monitor) = profile_monitor.as_mut() {
                if monitor.changed() {
                    if let Err(error) = reload_profile(
                        &mut mapper,
                        &mut virtual_gamepad,
                        &mut keyboard,
                        &mut mouse,
                        monitor.path(),
                        &mut trace,
                    ) {
                        eprintln!(
                            "ira-input: profile reload failed for {}: {error}",
                            monitor.path().display()
                        );
                    } else {
                        gyro_processor = make_gyro_processor(mapper.profile());
                        last_sensor_us = None;
                        tick_needed = sensor.is_some() || mapper.has_continuous_outputs();
                    }
                }
            }
        }
        let timeout = schedule.timeout(
            tick_needed,
            child.is_some(),
            profile_monitor.is_some(),
            steam_session.is_some(),
            !connected,
        );
        wait_for_inputs(&gamepad, timeout)?;
    }
}

/// Open the initially detected controller. Returns `None` (no error) when no
/// controller is plugged in yet — the session keeps running and picks one up
/// the moment it appears.
fn open_initial_gamepad(device: Option<&Path>) -> Option<PhysicalGamepad> {
    if let Some(path) = device {
        return match PhysicalGamepad::open(path, false) {
            Ok(gamepad) => Some(gamepad),
            Err(error) => {
                eprintln!("ira-input: {error}");
                None
            }
        };
    }
    let info = discover_gamepads().into_iter().next()?;
    match PhysicalGamepad::open(&info.path, false) {
        Ok(gamepad) => Some(gamepad),
        Err(error) => {
            eprintln!("ira-input: failed to open {}: {error}", info.path.display());
            None
        }
    }
}

/// Reconnect the previously-used controller, or detect a brand-new one when
/// none was present at launch. Returns true when a controller is now open.
fn reconnect_gamepad(gamepad: &mut Option<PhysicalGamepad>) -> Result<bool, String> {
    if let Some(existing) = gamepad.as_mut() {
        return existing.try_reconnect();
    }
    let Some(info) = discover_gamepads().into_iter().next() else {
        return Ok(false);
    };
    match PhysicalGamepad::open(&info.path, false) {
        Ok(opened) => {
            *gamepad = Some(opened);
            Ok(true)
        }
        Err(error) => {
            eprintln!("ira-input: failed to open {}: {error}", info.path.display());
            Ok(false)
        }
    }
}

/// Block until the kernel has an input event or scheduled work is due. When no
/// controller is present, waits on the scheduled timeout instead.
fn wait_for_inputs(
    gamepad: &Option<PhysicalGamepad>,
    timeout: Option<Duration>,
) -> Result<(), String> {
    if let Some(gamepad) = gamepad {
        return gamepad.wait_for_event(timeout);
    }
    let timeout_ms = timeout
        .map(|timeout| {
            timeout
                .as_nanos()
                .div_ceil(1_000_000)
                .min(libc::c_int::MAX as u128) as libc::c_int
        })
        .unwrap_or(-1);
    let result = unsafe { libc::poll(std::ptr::null_mut(), 0, timeout_ms) };
    if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
        return Err(format!(
            "failed waiting for controller: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn steam_processes(app_id: &str) -> SteamProcessSnapshot {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return SteamProcessSnapshot {
            processes: HashSet::new(),
            complete: false,
        };
    };
    let uid = unsafe { libc::geteuid() };
    let mut processes = HashSet::new();
    let mut complete = true;
    for pid in entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| *pid != std::process::id() as i32)
    {
        let proc_path = format!("/proc/{pid}");
        let Ok(metadata) = std::fs::metadata(&proc_path) else {
            continue;
        };
        if metadata.uid() != uid {
            continue;
        }
        let environment = match std::fs::read(format!("{proc_path}/environ")) {
            Ok(environment) => environment,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        if !environment_has_steam_app(&environment, app_id) {
            continue;
        }
        match process_start_time(pid) {
            Some(start_time) => {
                processes.insert(ProcessIdentity { pid, start_time });
            }
            None => complete = false,
        }
    }
    SteamProcessSnapshot {
        processes,
        complete,
    }
}

fn process_start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_process_start_time(&stat)
}

fn parse_process_start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn environment_has_steam_app(environment: &[u8], app_id: &str) -> bool {
    environment.split(|byte| *byte == 0).any(|variable| {
        ["SteamAppId", "SteamGameId", "STEAM_COMPAT_APP_ID"]
            .iter()
            .any(|key| {
                variable
                    .strip_prefix(format!("{key}=").as_bytes())
                    .is_some_and(|value| value == app_id.as_bytes())
            })
    })
}

fn request_steam_stop(app_id: &str) {
    let uri = format!("steam://stop/{app_id}");
    if std::process::Command::new("steam")
        .arg(&uri)
        .spawn()
        .is_err()
    {
        let _ = std::process::Command::new("xdg-open").arg(uri).spawn();
    }
}

fn inject_flatpak_env(program: &str, args: &mut Vec<String>, key: &str, value: &str) {
    let program = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str());
    let is_flatpak = program == Some("flatpak");
    let is_flatpak_spawn = program == Some("flatpak-spawn")
        && args
            .windows(2)
            .any(|window| window == ["--host", "flatpak"]);
    if !is_flatpak && !is_flatpak_spawn {
        return;
    }
    let Some(run_index) = args.iter().position(|argument| argument == "run") else {
        return;
    };
    args.insert(run_index + 1, format!("--env={key}={value}"));
}

fn inject_flatpak_target_env(
    program: &str,
    args: &mut Vec<String>,
    backend: VirtualGamepadBackend,
    vendor: Option<u16>,
    product: Option<u16>,
) {
    inject_flatpak_env(program, args, "SDL_JOYSTICK_HIDAPI", "0");
    if let Some(mapping) = sdl_mapping_for_backend(backend) {
        inject_flatpak_env(program, args, "SDL_GAMECONTROLLERCONFIG", &mapping);
    }
    if let (Some(vendor), Some(product)) = (vendor, product) {
        if let Some(ignored_device) = ignored_device_for_target(vendor, product, backend) {
            inject_flatpak_env(
                program,
                args,
                "SDL_GAMECONTROLLER_IGNORE_DEVICES",
                &ignored_device,
            );
        }
    }
}

fn sdl_mapping_for_backend(backend: VirtualGamepadBackend) -> Option<String> {
    match backend {
        VirtualGamepadBackend::XInput => None,
        VirtualGamepadBackend::DirectInput => Some(VirtualGamepad::direct_input_sdl_mapping()),
        VirtualGamepadBackend::SwitchPro => Some(VirtualGamepad::switch_pro_sdl_mapping()),
    }
}

fn ignored_device_for_target(
    vendor: u16,
    product: u16,
    backend: VirtualGamepadBackend,
) -> Option<String> {
    if ((vendor, product) == (VIRTUAL_XBOX_VENDOR, VIRTUAL_XBOX_PRODUCT)
        && backend == VirtualGamepadBackend::XInput)
        || ((vendor, product) == (SWITCH_PRO_VENDOR, SWITCH_PRO_PRODUCT)
            && backend == VirtualGamepadBackend::SwitchPro)
    {
        None
    } else {
        Some(format!("0x{vendor:04x}/0x{product:04x}"))
    }
}

fn load_profile(path: Option<&Path>) -> Result<InputProfile, String> {
    let profile = match path {
        None => InputProfile::default_gamepad(),
        Some(path) if path == Path::new("builtin:default_gamepad") => {
            InputProfile::default_gamepad()
        }
        Some(path) => {
            let contents = std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read profile {}: {error}", path.display()))?;
            InputProfile::from_json(&contents)
                .map_err(|error| format!("failed to parse profile {}: {error}", path.display()))?
        }
    };
    profile.validate().map_err(|error| match path {
        Some(path) => format!("invalid profile {}: {error}", path.display()),
        None => format!("invalid builtin profile: {error}"),
    })?;
    Ok(profile)
}

fn is_real_profile(path: &Path) -> bool {
    !path.to_string_lossy().starts_with("builtin:")
}

fn create_keyboard(keycodes: Vec<u16>) -> Result<Option<VirtualKeyboard>, String> {
    if keycodes.is_empty() {
        return Ok(None);
    }
    VirtualKeyboard::create(keycodes)
        .map(Some)
        .map_err(|error| format!("failed to create virtual keyboard: {error}"))
}

fn create_mouse(needed: bool) -> Result<Option<VirtualMouse>, String> {
    if !needed {
        return Ok(None);
    }
    VirtualMouse::create()
        .map(Some)
        .map_err(|error| format!("failed to create virtual mouse: {error}"))
}

struct ParsedEvent {
    mask: u32,
    name: String,
}

struct ProfileMonitor {
    fd: libc::c_int,
    watch: libc::c_int,
    path: PathBuf,
    parent: PathBuf,
    filename: String,
    reload: bool,
    last_watch_error: Instant,
    last_read_error: Instant,
}

impl ProfileMonitor {
    fn new(path: PathBuf) -> Self {
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let parent = if parent.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            parent
        };
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
        let fd = if fd < 0 {
            eprintln!(
                "ira-input: inotify_init1 failed: {}",
                std::io::Error::last_os_error()
            );
            -1
        } else {
            fd
        };
        let mut monitor = Self {
            fd,
            watch: -1,
            path,
            parent,
            filename,
            reload: false,
            last_watch_error: Instant::now(),
            last_read_error: Instant::now(),
        };
        monitor.ensure_watch();
        monitor.reload = false;
        monitor
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn changed(&mut self) -> bool {
        if self.fd < 0 {
            return false;
        }
        self.drain_events();
        self.ensure_watch();
        std::mem::take(&mut self.reload)
    }

    fn drain_events(&mut self) {
        let mut buffer = [0u8; 4096];
        loop {
            let read = unsafe { libc::read(self.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EAGAIN) {
                    break;
                }
                if self.last_read_error.elapsed() >= Duration::from_secs(5) {
                    eprintln!("ira-input: inotify read failed: {error}");
                    self.last_read_error = Instant::now();
                }
                break;
            }
            if read == 0 {
                break;
            }
            for event in parse_inotify_buffer(&buffer[..read as usize]) {
                self.handle_parsed(&event);
            }
        }
    }

    fn handle_parsed(&mut self, event: &ParsedEvent) {
        let mask = event.mask;
        if mask & libc::IN_IGNORED != 0 {
            self.watch = -1;
            return;
        }
        if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0 {
            self.watch = -1;
            return;
        }
        if mask & libc::IN_Q_OVERFLOW != 0 {
            eprintln!("ira-input: inotify queue overflow; forcing profile reload");
            self.reload = true;
            return;
        }
        if event.name == self.filename {
            self.reload = true;
        }
    }

    fn ensure_watch(&mut self) {
        if self.watch != -1 || self.fd < 0 {
            return;
        }
        let Some(c_path) = std::ffi::CString::new(self.parent.as_os_str().as_bytes()).ok() else {
            return;
        };
        let mask = libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_TO
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF;
        let watch = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), mask) };
        if watch < 0 {
            if self.last_watch_error.elapsed() >= Duration::from_secs(5) {
                eprintln!(
                    "ira-input: inotify_add_watch failed for {}: {}",
                    self.parent.display(),
                    std::io::Error::last_os_error()
                );
                self.last_watch_error = Instant::now();
            }
            return;
        }
        self.watch = watch;
        self.reload = true;
    }
}

impl Drop for ProfileMonitor {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

fn parse_inotify_buffer(buffer: &[u8]) -> Vec<ParsedEvent> {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut events = Vec::new();
    let mut offset = 0;
    while offset + header <= buffer.len() {
        let event = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const libc::inotify_event)
        };
        let event_size = header + event.len as usize;
        if offset + event_size > buffer.len() {
            break;
        }
        let name = if event.len > 0 {
            let bytes = &buffer[offset + header..offset + event_size];
            String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .to_string()
        } else {
            String::new()
        };
        events.push(ParsedEvent {
            mask: event.mask,
            name,
        });
        offset += event_size;
    }
    events
}

fn reload_profile(
    mapper: &mut MappingEngine,
    virtual_gamepad: &mut VirtualGamepad,
    keyboard: &mut Option<VirtualKeyboard>,
    mouse: &mut Option<VirtualMouse>,
    path: &Path,
    trace: &mut TraceState,
) -> Result<(), String> {
    let profile = load_profile(Some(path))?;
    let new_mapper = MappingEngine::new(profile)?;
    let backend_changed = mapper.profile().backend != new_mapper.profile().backend;
    if backend_changed {
        return Err("virtual gamepad backend changes require restarting the game".to_string());
    }
    let keycodes_changed =
        mapper.profile().keyboard_keycodes() != new_mapper.profile().keyboard_keycodes();
    let mouse_changed = mapper.profile().uses_mouse() != new_mapper.profile().uses_mouse();
    let replacement_keyboard = if keycodes_changed {
        create_keyboard(new_mapper.profile().keyboard_keycodes())?
    } else {
        None
    };
    let replacement_mouse = if mouse_changed {
        create_mouse(new_mapper.profile().uses_mouse())?
    } else {
        None
    };
    emit_outputs(
        mapper.reset(),
        OutputTargets {
            gamepad: virtual_gamepad,
            keyboard: keyboard.as_mut(),
            mouse: mouse.as_mut(),
        },
        trace,
    )?;
    if keycodes_changed {
        *keyboard = replacement_keyboard;
    }
    if mouse_changed {
        *mouse = replacement_mouse;
    }
    *mapper = new_mapper;
    eprintln!(
        "ira-input: reloaded {} bindings from {}",
        mapper.profile().bindings.len(),
        path.display()
    );
    Ok(())
}

fn open_sensor(device: &ira_input::DeviceInfo) -> Option<Sdl3SensorBackend> {
    match Sdl3SensorBackend::open(device) {
        Ok(Some(sensor)) => {
            eprintln!("ira-input: SDL3 gyro backend active");
            Some(sensor)
        }
        Ok(None) => {
            eprintln!("ira-input: no gyro sensor available for this controller");
            None
        }
        Err(error) => {
            eprintln!("ira-input: gyro backend unavailable: {error}");
            None
        }
    }
}

fn process_physical_inputs(
    gamepad: &mut Option<PhysicalGamepad>,
    mapper: &mut MappingEngine,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    let Some(gamepad) = gamepad.as_mut() else {
        return Ok(());
    };
    for event in gamepad.fetch_events()? {
        emit_mapped(
            mapper,
            OutputTargets {
                gamepad: targets.gamepad,
                keyboard: targets.keyboard.as_deref_mut(),
                mouse: targets.mouse.as_deref_mut(),
            },
            event,
            trace,
        )?;
    }
    Ok(())
}

fn make_gyro_processor(profile: &InputProfile) -> GyroProcessor {
    GyroProcessor::new(
        profile.gyro_calibration,
        GyroProcessingOptions {
            smoothing: profile.gyro.smoothing,
            auto_calibrate: true,
        },
    )
}

/// The virtual devices mapped events are written to, borrowed per loop pass.
struct OutputTargets<'a> {
    gamepad: &'a mut VirtualGamepad,
    keyboard: Option<&'a mut VirtualKeyboard>,
    mouse: Option<&'a mut VirtualMouse>,
}

fn process_tick(
    sensor: &mut Option<Sdl3SensorBackend>,
    gyro_processor: &mut GyroProcessor,
    last_sensor_us: &mut Option<u64>,
    mapper: &mut MappingEngine,
    targets: OutputTargets<'_>,
    trace: &mut TraceState,
    run: bool,
) -> Result<(), String> {
    if !run {
        return Ok(());
    }
    let mut sensor_failed = false;
    if let Some(sensor) = sensor.as_mut() {
        match sensor.read(now_us()) {
            Ok(Some(sample)) => {
                let dt = last_sensor_us
                    .map(|last| sample.timestamp_us.saturating_sub(last) as f32 / 1_000_000.0)
                    .unwrap_or(1.0 / 250.0)
                    .clamp(0.0005, 0.05);
                *last_sensor_us = Some(sample.timestamp_us);
                trace.record_gyro(sample.gyro);
                let rates = gyro_processor.process(sample.gyro, sample.accel, dt);
                mapper.update_gyro(rates);
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("ira-input: gyro backend stopped: {error}");
                sensor_failed = true;
            }
        }
    }
    if sensor_failed {
        *sensor = None;
    }
    let outputs = mapper.tick(now_us());
    emit_outputs(outputs, targets, trace)
}

fn stop_child(child: &mut Option<std::process::Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn emit_mapped(
    mapper: &mut MappingEngine,
    targets: OutputTargets<'_>,
    event: InputEvent,
    trace: &mut TraceState,
) -> Result<(), String> {
    trace.record_input(event);
    emit_outputs(mapper.process(event), targets, trace)
}

fn emit_outputs(
    outputs: Vec<OutputEvent>,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    for output in outputs {
        trace.record_output(&output);
        targets
            .gamepad
            .emit(&output)
            .map_err(|error| format!("failed to emit virtual input: {error}"))?;
        if let Some(keyboard) = targets.keyboard.as_deref_mut() {
            keyboard
                .emit(&output)
                .map_err(|error| format!("failed to emit virtual keyboard input: {error}"))?;
        }
        if let Some(mouse) = targets.mouse.as_deref_mut() {
            mouse
                .emit(&output)
                .map_err(|error| format!("failed to emit virtual mouse input: {error}"))?;
        }
    }
    if let Some(mouse) = targets.mouse {
        mouse
            .flush()
            .map_err(|error| format!("failed to emit virtual mouse input: {error}"))?;
    }
    Ok(())
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_input::VirtualGamepadBackend;

    #[test]
    fn test_is_real_profile_excludes_builtin() {
        assert!(!is_real_profile(Path::new("builtin:default_gamepad")));
        assert!(!is_real_profile(Path::new("builtin:anything")));
        assert!(is_real_profile(Path::new("/tmp/profile.json")));
    }

    #[test]
    fn test_load_profile_preserves_backend_and_validates_bindings() {
        let dir = temp_profile_dir("backend");
        let path = dir.join("profile.json");
        let mut profile =
            InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::DirectInput);
        profile.bindings.clear();
        std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();

        let loaded = load_profile(Some(&path)).unwrap();
        assert_eq!(loaded.backend, VirtualGamepadBackend::DirectInput);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_inject_flatpak_env_places_mapping_after_run() {
        let mut args = vec!["run".to_string(), "net.shadps4.shadPS4".to_string()];
        inject_flatpak_env("/usr/bin/flatpak", &mut args, "KEY", "value");
        assert_eq!(args, ["run", "--env=KEY=value", "net.shadps4.shadPS4"]);

        let mut native = vec!["--fullscreen".to_string()];
        inject_flatpak_env("shadps4", &mut native, "KEY", "value");
        assert_eq!(native, ["--fullscreen"]);

        let mut nested = vec![
            "--host".to_string(),
            "flatpak".to_string(),
            "run".to_string(),
            "net.shadps4.shadPS4".to_string(),
        ];
        inject_flatpak_env("flatpak-spawn", &mut nested, "KEY", "value");
        assert_eq!(
            nested,
            [
                "--host",
                "flatpak",
                "run",
                "--env=KEY=value",
                "net.shadps4.shadPS4"
            ]
        );
    }

    #[test]
    fn test_environment_has_steam_app_matches_supported_markers() {
        assert!(environment_has_steam_app(
            b"PATH=/bin\0SteamAppId=123\0",
            "123"
        ));
        assert!(environment_has_steam_app(
            b"STEAM_COMPAT_APP_ID=123\0",
            "123"
        ));
        assert!(environment_has_steam_app(b"SteamGameId=123\0", "123"));
    }

    #[test]
    fn test_environment_has_steam_app_rejects_partial_or_different_ids() {
        assert!(!environment_has_steam_app(b"SteamAppId=1234\0", "123"));
        assert!(!environment_has_steam_app(b"OtherSteamAppId=123\0", "123"));
        assert!(!environment_has_steam_app(b"PATH=/bin\0", "123"));
    }

    #[test]
    fn test_parse_process_start_time_handles_spaces_in_process_name() {
        let stat = "42 (game with spaces) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 98765 20";
        assert_eq!(parse_process_start_time(stat), Some(98765));
        assert_eq!(parse_process_start_time("invalid"), None);
    }

    #[test]
    fn test_ignored_device_for_target_preserves_virtual_xbox() {
        assert_eq!(
            ignored_device_for_target(
                VIRTUAL_XBOX_VENDOR,
                VIRTUAL_XBOX_PRODUCT,
                VirtualGamepadBackend::XInput,
            ),
            None
        );
        assert_eq!(
            ignored_device_for_target(0x2dc8, 0x3106, VirtualGamepadBackend::XInput),
            Some("0x2dc8/0x3106".to_string())
        );
    }

    #[test]
    fn test_ignored_device_for_target_preserves_switch_pro_identity() {
        assert_eq!(
            ignored_device_for_target(
                SWITCH_PRO_VENDOR,
                SWITCH_PRO_PRODUCT,
                VirtualGamepadBackend::SwitchPro,
            ),
            None
        );
    }

    #[test]
    fn test_inject_flatpak_target_env_configures_switch_pro_isolation() {
        let mut args = vec!["run".to_string(), "com.example.Game".to_string()];
        inject_flatpak_target_env(
            "/usr/bin/flatpak",
            &mut args,
            VirtualGamepadBackend::SwitchPro,
            Some(SWITCH_PRO_VENDOR),
            Some(SWITCH_PRO_PRODUCT),
        );

        assert!(args.contains(&"--env=SDL_JOYSTICK_HIDAPI=0".to_string()));
        assert!(args.iter().any(|argument| {
            argument.starts_with("--env=SDL_GAMECONTROLLERCONFIG=030000007e0500000920000011810000,")
        }));
        assert!(!args
            .iter()
            .any(|argument| argument.starts_with("--env=SDL_GAMECONTROLLER_IGNORE_DEVICES=")));
    }

    #[test]
    fn test_sdl_mapping_is_configured_for_switch_pro_backend() {
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::SwitchPro)
            .unwrap()
            .starts_with("030000007e0500000920000011810000,"));
        assert!(sdl_mapping_for_backend(VirtualGamepadBackend::XInput).is_none());
    }

    #[test]
    fn test_loop_schedule_blocks_without_periodic_work() {
        let schedule = LoopSchedule::new();
        assert_eq!(schedule.timeout(false, false, false, false, false), None);
    }

    #[test]
    fn test_loop_schedule_uses_earliest_active_deadline() {
        let schedule = LoopSchedule::new();
        let timeout = schedule
            .timeout(true, true, true, true, true)
            .expect("active work must have a deadline");
        assert!(timeout <= SENSOR_SAMPLE_INTERVAL);
    }

    fn make_event(mask: u32, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1i32.to_ne_bytes());
        bytes.extend_from_slice(&mask.to_ne_bytes());
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        bytes.extend_from_slice(&(name.len() as u32).to_ne_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes
    }

    #[test]
    fn test_parse_inotify_buffer_extracts_names() {
        let mut buffer = make_event(libc::IN_CLOSE_WRITE, "profile.json");
        buffer.extend(make_event(libc::IN_MOVED_TO, "other.txt"));
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].mask, libc::IN_CLOSE_WRITE);
        assert_eq!(events[0].name, "profile.json");
        assert_eq!(events[1].mask, libc::IN_MOVED_TO);
        assert_eq!(events[1].name, "other.txt");
    }

    #[test]
    fn test_parse_inotify_buffer_handles_self_events() {
        let buffer = make_event(libc::IN_MOVE_SELF, "");
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mask, libc::IN_MOVE_SELF);
        assert_eq!(events[0].name, "");
    }

    #[test]
    fn test_parse_inotify_buffer_ignores_truncated_tail() {
        let mut buffer = make_event(libc::IN_CREATE, "profile.json");
        buffer.extend_from_slice(&[0xff; 3]);
        let events = parse_inotify_buffer(&buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "profile.json");
    }

    fn temp_profile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ira-input-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for_change(monitor: &mut ProfileMonitor, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if monitor.changed() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn test_profile_monitor_detects_in_place_write() {
        let dir = temp_profile_dir("inplace");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::write(&path, "two").unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_detects_atomic_rename() {
        let dir = temp_profile_dir("rename");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        let tmp = dir.join(".profile.json.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_detects_delete_and_recreate() {
        let dir = temp_profile_dir("recreate");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::remove_file(&path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        let tmp = dir.join(".profile.json.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(wait_for_change(&mut monitor, Duration::from_secs(2)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_profile_monitor_ignores_unrelated_files() {
        let dir = temp_profile_dir("unrelated");
        let path = dir.join("profile.json");
        std::fs::write(&path, "one").unwrap();
        let mut monitor = ProfileMonitor::new(path.clone());
        thread::sleep(Duration::from_millis(20));
        assert!(!monitor.changed());
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        thread::sleep(Duration::from_millis(50));
        assert!(!monitor.changed());
        std::fs::remove_dir_all(&dir).ok();
    }
}
