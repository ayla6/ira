use ira_db::{record_session, DbConn};
use ira_input_ipc::{DaemonClient, Event};
use ira_models::AppMessage;
use ira_models::AppSender;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const PR_SET_CHILD_SUBREAPER: i32 = 36;

const WINE_BG_PROCESSES: &[&str] = &[
    "wineserver",
    "services.exe",
    "winedevice.exe",
    "plugplay.exe",
    "explorer.exe",
    "wineconsole",
    "svchost.exe",
    "rpcss.exe",
    "rundll32.exe",
    "mscorsvw.exe",
    "iexplore.exe",
    "winedbg.exe",
    "tabtip.exe",
    "conhost.exe",
];

// ─── In-memory game log store ───

type GameLog = Arc<Mutex<Vec<String>>>;

static GAME_LOGS: OnceLock<Mutex<HashMap<i64, GameLog>>> = OnceLock::new();

fn game_logs() -> &'static Mutex<HashMap<i64, GameLog>> {
    GAME_LOGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns the shared log buffer for a game. Creates a new empty buffer if none exists.
pub fn get_game_log(game_id: i64) -> GameLog {
    let mut logs = game_logs().lock().unwrap();
    logs.entry(game_id)
        .or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Clears the log buffer for a game (called at launch start).
pub fn clear_game_log(game_id: i64) {
    let logs = game_logs().lock().unwrap();
    if let Some(log) = logs.get(&game_id) {
        log.lock().unwrap().clear();
    }
}

fn set_subreaper() {
    let ret = unsafe { libc::prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if ret != 0 {
        eprintln!(
            "Warning: failed to set child subreaper (prctl returned {})",
            ret
        );
    }
}

/// Spawns the game process with piped stdout/stderr for in-memory logging.
pub fn spawn_game(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_path: Option<&str>,
) -> Result<Child, String> {
    spawn_game_with(command, env, cwd, log_path)
}

/// The spawn behind both `spawn_game` and `spawn_detached`.
///
/// Games must survive Ira: no PR_SET_PDEATHSIG (it fires when the spawning
/// thread exits, killing the game when Ira quits), own process group (so
/// terminal Ctrl-C never reaches them), and file-backed stdio — piped
/// output SIGPIPEs the game the moment our reader threads die with us.
/// `log_path` truncates and receives stdout+stderr (O_APPEND interleaved);
/// `None` sends both to null. Callers tail the file for the live log view.
fn spawn_game_with(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_path: Option<&str>,
) -> Result<Child, String> {
    if command.is_empty() {
        return Err("Failed to spawn game process: empty command".to_string());
    }
    set_subreaper();

    let mut cmd = Command::new(&command[0]);
    for arg in &command[1..] {
        cmd.arg(arg);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // Clear the parent env entirely so filtered vars (CARGO_*, RUSTUP_*, etc.)
    // don't leak into the child. Our env list is the complete env for the child.
    cmd.env_clear();
    for (key, val) in env {
        cmd.env(key, val);
    }

    cmd.stdout(game_stdio(log_path));
    cmd.stderr(game_stdio(log_path));
    cmd.process_group(0);

    match cmd.spawn() {
        Ok(child) => Ok(child),
        Err(error) => {
            let diagnostic = format_spawn_error(command, env, cwd, &error);
            eprintln!("launch: {diagnostic}");
            Err(diagnostic)
        }
    }
}

/// Stdio for spawned games: append to `log_path`, or null when absent.
/// Never pipes: our reader threads die with Ira, and a game writing to a
/// dead pipe eats SIGPIPE the moment we quit (or freezes on a full one).
fn game_stdio(log_path: Option<&str>) -> Stdio {
    let Some(path) = log_path else {
        return Stdio::null();
    };
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, "");
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

fn format_spawn_error(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    error: &std::io::Error,
) -> String {
    let program = command.first().map(String::as_str).unwrap_or("");
    let resolved_program = resolve_program(program, env);
    let cwd_info = cwd
        .map(|path| {
            format!(
                "{} (exists={}, directory={})",
                path,
                Path::new(path).exists(),
                Path::new(path).is_dir()
            )
        })
        .unwrap_or_else(|| "<inherit>".to_string());
    let path = env_value(env, "PATH").unwrap_or_else(|| "<unset>".to_string());
    let prefix = env_value(env, "WINEPREFIX").unwrap_or_else(|| "<unset>".to_string());
    let wine = env_value(env, "WINE").unwrap_or_else(|| "<unset>".to_string());

    format!(
        "Failed to spawn game process: {error} (kind={:?}, raw_os_error={:?}); command={command:?}; program={program:?}; resolved_program={:?}; program_exists={}; cwd={cwd_info}; PATH={path:?}; WINE={wine:?}; WINEPREFIX={prefix:?}",
        error.kind(),
        error.raw_os_error(),
        resolved_program.as_ref().map(|p| p.display().to_string()),
        resolved_program.as_ref().is_some_and(|p| p.is_file()),
    )
}

fn env_value(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

fn resolve_program(program: &str, env: &[(String, String)]) -> Option<std::path::PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') {
        return Some(path.to_path_buf());
    }
    let search_path = env_value(env, "PATH").or_else(|| std::env::var("PATH").ok())?;
    std::env::split_paths(&search_path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Renders a command for the log the way a shell would need it typed:
/// arguments a shell would misread (spaces, quotes, globs, …) are
/// single-quoted, so a path with spaces reads as one argument and the line
/// can be pasted into a terminal. The spawn itself is argv-based and never
/// needs this; it's display-only.
fn display_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            let plain = !arg.is_empty()
                && arg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_/.:=,+@%~".contains(&b));
            if plain {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn game_log_path(save_dir: &str, game_id: i64) -> String {
    Path::new(save_dir)
        .join("logs")
        .join(format!("{}.log", game_id))
        .to_string_lossy()
        .into_owned()
}

/// Follows a game log FILE into the shared buffer (replaces pipe capture:
/// pipes SIGPIPE the game when Ira exits, files don't). Stops after `done`
/// is set and the file goes quiet for ~1s so exit-flushes still land.
/// Reopens per poll, so truncation/rotation just restarts the offset.
pub(crate) fn tail_file_to_log(path: String, log: GameLog, done: Arc<AtomicBool>) {
    use std::io::{Read, Seek, SeekFrom};
    std::thread::spawn(move || {
        let mut offset = 0u64;
        let mut pending = String::new();
        let mut quiet_polls = 0u32;
        loop {
            let mut grown = false;
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() < offset {
                    offset = 0;
                }
                if let Ok(mut f) = std::fs::File::open(&path) {
                    if f.seek(SeekFrom::Start(offset)).is_ok() {
                        let mut buf = [0u8; 8192];
                        loop {
                            match f.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    grown = true;
                                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                                    while let Some(pos) = pending.find('\n') {
                                        let line =
                                            pending[..pos].trim_end_matches('\r').to_string();
                                        log.lock().unwrap().push(line);
                                        pending = pending[pos + 1..].to_string();
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        if let Ok(pos) = f.stream_position() {
                            offset = pos;
                        }
                    }
                }
            }
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                if !grown {
                    quiet_polls += 1;
                    if quiet_polls >= 4 {
                        if !pending.is_empty() {
                            log.lock().unwrap().push(std::mem::take(&mut pending));
                        }
                        break;
                    }
                } else {
                    quiet_polls = 0;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

/// Drains one piped output stream of a spawned process into the shared game
/// log buffer, one line at a time. Runs on its own thread so both pipes can
/// be read live without deadlocking on a full pipe buffer.
pub(crate) fn pipe_lines_to_log<R: Read + std::marker::Send + 'static>(pipe: Option<R>, log: GameLog) {
    std::thread::spawn(move || {
        let Some(mut reader) = pipe else { return };
        let mut buf = [0u8; 4096];
        let mut pending = String::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if !pending.is_empty() {
                        log.lock().unwrap().push(pending);
                    }
                    break;
                }
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    while let Some(pos) = pending.find('\n') {
                        let line = pending[..pos].trim_end_matches('\r').to_string();
                        log.lock().unwrap().push(line);
                        pending = pending[pos + 1..].to_string();
                    }
                }
            }
        }
    });
}

/// Spawns a process that is not tracked as a play session: nothing is
/// recorded, no messages are sent, and the child is left running after Ira
/// exits (aside from the subreaper). Its output is streamed into the log
/// buffer for `log_key` so the in-app log viewer still shows it.
pub fn spawn_detached(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_key: i64,
    header: String,
    desktop_hold: Option<DaemonClient>,
) -> Result<(), String> {
    // Detached output lives in temp (no save dir is threaded here) and is
    // tailed into the buffer; the game must survive Ira either way.
    let log_path = std::env::temp_dir()
        .join(format!("ira-detached-{log_key}.log"))
        .to_string_lossy()
        .into_owned();
    let mut child = spawn_game_with(command, env, cwd, Some(&log_path))?;
    clear_game_log(log_key);
    let log = get_game_log(log_key);
    log.lock().unwrap().push(header);

    let done = Arc::new(AtomicBool::new(false));
    tail_file_to_log(log_path, log.clone(), done.clone());

    let exit_log = log.clone();
    let tail_done = done.clone();
    std::thread::spawn(move || {
        // Held until the process exits: a detached spawn opened with
        // input remapping disabled must not be remapped underneath by
        // the daemon's idle desktop session. Dropping the client when
        // this thread ends releases the hold, however the process ends.
        let _desktop_hold = desktop_hold;
        match child.wait() {
            Ok(status) => {
                exit_log
                    .lock()
                    .unwrap()
                    .push(format!("Process exited with status {status}"));
            }
            Err(error) => {
                eprintln!("launch: failed to read detached process status: {error}");
            }
        }
        tail_done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    Ok(())
}

pub struct MonitorContext {
    pub sender: AppSender,
    pub game_id: i64,
    pub variant_id: Option<i64>,
    pub count_playtime: bool,
    pub started_at: i64,
    pub db: DbConn,
    pub running_games: Arc<Mutex<HashMap<i64, i32>>>,
    pub env: Vec<(String, String)>,
    pub command: Vec<String>,
    pub post_exit: String,
    pub working_dir: Option<String>,
    /// Game stdout/stderr file the spawn wrote (tail it; empty when the
    /// spawn had nowhere to log, e.g. daemon sessions pump over IPC).
    pub log_file: String,
}

pub fn monitor_process(mut child: Child, child_pid: i32, ctx: MonitorContext) {
    let game_id = ctx.game_id;

    // Clear previous log and get the shared buffer for this game.
    clear_game_log(game_id);
    let log_buf = get_game_log(game_id);

    log_launch_header(&ctx, &log_buf, "Started initial process");

    // The game writes to a file, never a pipe: our reader threads die with
    // Ira, and a dead pipe SIGPIPEs the game on its next log write.
    let tail_done = Arc::new(AtomicBool::new(false));
    if !ctx.log_file.is_empty() {
        tail_file_to_log(ctx.log_file.clone(), log_buf.clone(), tail_done.clone());
    }

    loop {
        std::thread::sleep(Duration::from_secs(2));
        match child.try_wait() {
            Ok(Some(status)) => {
                let message = format!("Initial process exited with status {status}");
                eprintln!("launch: {message}");
                log_buf.lock().unwrap().push(message);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let message = format!("Failed to read initial process status: {error}");
                eprintln!("launch: {message}");
                log_buf.lock().unwrap().push(message);
                break;
            }
        }
    }

    // Let the tail thread flush exit output, then stop (~1s).
    tail_done.store(true, std::sync::atomic::Ordering::SeqCst);
    finalize_game(&ctx, &log_buf, Some(child_pid), None);
}

/// Shared end-of-session bookkeeping for both monitors: zombie reaping,
/// post-exit script, playtime recording, and the UI notifications.
/// `exit_code` carries the daemon session's reported code, if any.
pub(crate) fn finalize_game(
    ctx: &MonitorContext,
    log_buf: &GameLog,
    child_pid: Option<i32>,
    exit_code: Option<i32>,
) {
    if let Some(code) = exit_code {
        let message = format!("Game session exited with code {code}");
        eprintln!("launch: {message}");
        log_buf.lock().unwrap().push(message);
    }
    if let Some(pid) = child_pid {
        reap_zombies(pid);
    }

    if !ctx.post_exit.is_empty() {
        run_post_exit(&ctx.post_exit, ctx.working_dir.as_deref(), log_buf);
    }

    let ended_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let duration = ended_at - ctx.started_at;

    if duration < 5 {
        eprintln!(
            "Game {} exited after {}s — possible crash, not recording session",
            ctx.game_id, duration
        );
    } else if ctx.count_playtime {
        if let Err(e) = record_session(
            &ctx.db,
            ctx.game_id,
            ctx.variant_id,
            ctx.started_at,
            ended_at,
        ) {
            eprintln!("Failed to record play session: {}", e);
        }
        if let Err(e) = ctx.sender.send(AppMessage::SessionRecorded {
            game_id: ctx.game_id,
            variant_id: ctx.variant_id,
            duration_seconds: duration,
            started_at: ctx.started_at,
            ended_at,
        }) {
            eprintln!("Failed to send SessionRecorded message: {}", e);
        }
    }

    ctx.running_games.lock().unwrap().remove(&ctx.game_id);
    if let Err(e) = ctx
        .sender
        .send(AppMessage::GameStopped(ctx.game_id, ctx.variant_id))
    {
        eprintln!("Failed to send GameStopped message: {}", e);
    }

    // Continue reaping zombies for a few seconds after the game exits.
    // Wine background processes and stragglers may die after the main
    // game process, and without a reaper they become visible as defunct
    // processes in htop/btop.
    if let Some(pid) = child_pid {
        for _ in 0..10 {
            std::thread::sleep(Duration::from_secs(1));
            reap_zombies(pid);
        }
    }
}

/// The first lines of every game log: the command being run and the launch
/// environment, minus cargo's own noise.
pub(crate) fn log_launch_header(ctx: &MonitorContext, log_buf: &GameLog, started_message: &str) {
    let mut log = log_buf.lock().unwrap();
    log.push(format!(
        "{} from {}",
        started_message,
        display_command(&ctx.command)
    ));
    let mut sorted_env = ctx.env.clone();
    sorted_env.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in &sorted_env {
        if k.starts_with("CARGO_") || k.starts_with("RUSTUP_") || k.starts_with("RUST_") {
            continue;
        }
        log.push(format!("DEBUG: {}={}", k, v));
    }
}

/// Monitor for a game handed to the resident input daemon: no child process
/// of our own — the daemon supervises the game and reports back over the
/// IPC socket. The playtime bookkeeping is identical to the wrapper path.
pub fn monitor_session(mut client: DaemonClient, ctx: MonitorContext) {
    let game_id = ctx.game_id;
    clear_game_log(game_id);
    let log_buf = get_game_log(game_id);
    log_launch_header(&ctx, &log_buf, "Started via the input daemon");

    let mut child_pid: Option<i32> = None;
    let result = client.wait_session(|event| match event {
        Event::SessionStarted { child_pid: pid, .. } => {
            child_pid = Some(pid);
            ctx.running_games.lock().unwrap().insert(ctx.game_id, pid);
        }
        Event::Output { line, .. } => log_buf.lock().unwrap().push(line),
        Event::Controller {
            connected,
            name,
            path,
        } => {
            let message = if connected {
                format!("Controller connected: {name} ({path})")
            } else {
                format!("Controller disconnected: {name}")
            };
            log_buf.lock().unwrap().push(message);
        }
        Event::ProfileReloaded { path, .. } => {
            log_buf
                .lock()
                .unwrap()
                .push(format!("Controller profile reloaded: {path}"));
        }
        Event::SessionEnded { .. } => {}
    });
    let exit_code = match result {
        Ok(code) => Some(code),
        Err(error) => {
            // The daemon connection dropped mid-session. The game itself may
            // still be running; say so instead of pretending this was a clean
            // end.
            let message = format!("Input daemon connection lost: {error}");
            eprintln!("launch: {message}");
            log_buf.lock().unwrap().push(message);
            None
        }
    };

    finalize_game(&ctx, &log_buf, child_pid, exit_code);
}

fn reap_zombies(pgid: i32) {
    loop {
        let ret = unsafe {
            let mut status: i32 = 0;
            libc::waitpid(-pgid, &mut status as *mut i32, libc::WNOHANG)
        };
        if ret <= 0 {
            break;
        }
    }
}

/// Runs the game's post-exit script synchronously — the monitor thread blocks
/// until it finishes, mirroring the pre-launch "wait" semantics. Output is
/// appended to the in-memory game log next to the game's own output.
fn run_post_exit(script: &str, working_dir: Option<&str>, log: &GameLog) {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(script);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    match cmd.output() {
        Ok(out) => {
            let mut lines = vec![format!(
                "Post-exit script exited with status {}",
                out.status
            )];
            for stream in [&out.stdout, &out.stderr] {
                for line in String::from_utf8_lossy(stream).lines() {
                    lines.push(format!("Post-exit: {line}"));
                }
            }
            log.lock().unwrap().extend(lines);
            if !out.status.success() {
                eprintln!("Post-exit script failed with status {}", out.status);
            }
        }
        Err(error) => {
            eprintln!("Failed to run post-exit script: {error}");
            log.lock()
                .unwrap()
                .push(format!("Failed to run post-exit script: {error}"));
        }
    }
}

/// Reap ALL children of the calling process, regardless of process group.
/// Used after the Wine branch of stop_game finishes killing — by that point
/// wineserver -k has completed so there are no conflicting waitpid calls.
fn reap_all() {
    loop {
        let ret = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if ret <= 0 {
            break;
        }
    }
}

fn proc_name(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let start = stat.rfind(')').unwrap_or(0);
    let after_comm = &stat[start + 1..];
    let mut parts = after_comm.split_whitespace();
    parts.next()?;
    let comm = parts.next()?;
    Some(comm.to_string())
}

fn proc_children(pid: i32) -> Vec<i32> {
    let mut children = Vec::new();
    let task_dir = format!("/proc/{}/task", pid);
    if let Ok(tasks) = std::fs::read_dir(&task_dir) {
        for task in tasks.flatten() {
            let children_path = task.path().join("children");
            if let Ok(data) = std::fs::read_to_string(&children_path) {
                for pid_str in data.split_whitespace() {
                    if let Ok(child_pid) = pid_str.parse::<i32>() {
                        children.push(child_pid);
                    }
                }
            }
        }
    }
    children
}

fn collect_descendants(pid: i32) -> Vec<i32> {
    let mut all = Vec::new();
    let mut stack = vec![pid];
    while let Some(p) = stack.pop() {
        let children = proc_children(p);
        for child in children {
            if !all.contains(&child) {
                all.push(child);
                stack.push(child);
            }
        }
    }
    all
}

fn is_wine_bg(pid: i32) -> bool {
    if let Some(name) = proc_name(pid) {
        let name_lower = name.to_lowercase();
        let name_trunc = if name_lower.len() > 15 {
            &name_lower[..15]
        } else {
            &name_lower
        };
        return WINE_BG_PROCESSES.iter().any(|bg| {
            let bg_lower = bg.to_lowercase();
            let bg_trunc = if bg_lower.len() > 15 {
                &bg_lower[..15]
            } else {
                &bg_lower
            };
            name_trunc == bg_trunc
        });
    }
    false
}

/// Stop a native game process group without running the Wine cleanup sequence.
/// Gamescope and its emulator children share the launcher's process group, so
/// terminating the group also stops the standalone overlay promptly.
fn stop_native_process_group(pid: i32) {
    let descendants = collect_descendants(pid);
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(750));
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            for child in descendants {
                libc::kill(child, libc::SIGKILL);
            }
        }
    });
}

/// Poll until every pid is gone or the deadline elapses. Replaces the
/// blind sleeps in the shutdown sequence so a clean exit doesn't pay
/// the full grace period.
fn wait_all_gone(pids: &[i32], deadline: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if pids.iter().all(|p| unsafe { libc::kill(*p, 0) } != 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn stop_game(
    pid: i32,
    wine_exe: Option<&str>,
    wine_prefix: Option<&str>,
    env: &[(String, String)],
) {
    if wine_exe.is_none() && wine_prefix.is_none() {
        stop_native_process_group(pid);
        return;
    }

    // Collect descendants BEFORE sending signals — the process tree may
    // change after the game exits (children get reparented).
    let descendants = collect_descendants(pid);

    // Step 1: SIGTERM just the game PID (not the whole group) so the
    // game can exit cleanly while wine infrastructure stays alive.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    let wine_exe = wine_exe.map(|s| s.to_string());
    let wine_prefix = wine_prefix.map(|s| s.to_string());
    let env: Vec<(String, String)> = env.to_vec();
    std::thread::spawn(move || {
        // Step 2: Wait for the game to exit (poll for up to 5 seconds).
        for _ in 0..50 {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Step 3: Run `wineserver -k` while the wine infrastructure is
        // still alive so it can coordinate a graceful shutdown.
        if let (Some(exe), Some(prefix)) = (&wine_exe, &wine_prefix) {
            let wineserver = find_wineserver(exe);
            if let Some(ws) = &wineserver {
                eprintln!("stop: running wineserver -k for prefix {}", prefix);
                let mut cmd = Command::new(ws);
                cmd.arg("-k");
                cmd.env("WINEPREFIX", prefix);
                for (k, v) in &env {
                    cmd.env(k, v);
                }
                if let Err(e) = cmd.status() {
                    eprintln!("Failed to run wineserver -k: {}", e);
                }
            }
        }

        // Step 4: Wait for wineserver cleanup — bounded by the same
        // three seconds, but over as soon as everyone is actually gone.
        let mut watched = vec![pid];
        watched.extend(&descendants);
        wait_all_gone(&watched, Duration::from_secs(3));

        // Step 5: SIGTERM any remaining stragglers, identifying wine bg
        // processes via is_wine_bg() for diagnostics.
        for d in &descendants {
            let alive = unsafe { libc::kill(*d, 0) } == 0;
            if alive {
                let name = proc_name(*d).unwrap_or_default();
                let bg = is_wine_bg(*d);
                if bg {
                    eprintln!("stop: wine bg straggler SIGTERM pid {} ({})", d, name);
                } else {
                    eprintln!("stop: non-wine straggler SIGTERM pid {} ({})", d, name);
                }
                unsafe {
                    libc::kill(*d, libc::SIGTERM);
                }
            }
        }

        // Step 6: Wait up to 2s for stragglers to exit.
        wait_all_gone(&descendants, Duration::from_secs(2));

        // Step 7: Final fallback — SIGKILL the entire process group,
        // then SIGKILL any remaining stragglers that escaped the group.
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        for d in &descendants {
            let alive = unsafe { libc::kill(*d, 0) } == 0;
            if alive {
                eprintln!(
                    "stop: force killing pid {} ({}) wine_bg={}",
                    d,
                    proc_name(*d).unwrap_or_default(),
                    is_wine_bg(*d)
                );
                unsafe {
                    libc::kill(*d, libc::SIGKILL);
                }
            }
        }

        // Kill the original game PID if somehow still alive.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if alive {
            eprintln!("stop: force killing game pid {}", pid);
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }

        // Step 8: Reap all zombies. Safe to use waitpid(-1) here because
        // wineserver -k has already completed and no other blocking wait
        // calls are in flight.
        reap_all();
    });
}

fn find_wineserver(wine_exe: &str) -> Option<String> {
    let wine_dir = Path::new(wine_exe).parent()?;
    let candidate = wine_dir.join("wineserver");
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().into_owned());
    }
    if wine_exe.contains("/proton") || wine_exe.contains("/umu") {
        let proton_dir = wine_dir.parent().or_else(|| wine_dir.parent())?;
        let ws = proton_dir.join("files/bin/wineserver");
        if ws.is_file() {
            return Some(ws.to_string_lossy().into_owned());
        }
        let ws2 = proton_dir.join("dist/bin/wineserver");
        if ws2.is_file() {
            return Some(ws2.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{display_command, format_spawn_error};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn test_display_command_quotes_arguments_with_spaces() {
        assert_eq!(
            display_command(&[
                "azahar".to_string(),
                "/games/Zelda Tears of the Kingdom.3ds".to_string(),
            ]),
            "azahar '/games/Zelda Tears of the Kingdom.3ds'"
        );
    }

    #[test]
    fn test_display_command_leaves_plain_arguments_bare() {
        assert_eq!(
            display_command(&[
                "flatpak".to_string(),
                "run".to_string(),
                "--filesystem=/games:ro".to_string(),
                "org.azahar_emu.Azahar".to_string(),
            ]),
            "flatpak run --filesystem=/games:ro org.azahar_emu.Azahar"
        );
    }

    #[test]
    fn test_tail_file_to_log_collects_lines_and_stops() {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("ira-tail-test-{}.log", std::process::id()));
        std::fs::write(&path, "first\n").unwrap();
        let log = super::get_game_log(-97531);
        log.lock().unwrap().clear();
        let done = Arc::new(AtomicBool::new(false));
        super::tail_file_to_log(
            path.to_string_lossy().into_owned(),
            log.clone(),
            done.clone(),
        );
        // Append after the tail starts: it must pick up growth.
        std::thread::sleep(std::time::Duration::from_millis(400));
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, "second").unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        // Give the tail its ~1s quiet window to flush and stop.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let lines = log.lock().unwrap().clone();
        assert!(lines.contains(&"first".to_string()), "{lines:?}");
        assert!(lines.contains(&"second".to_string()), "{lines:?}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_display_command_quotes_empty_and_embedded_quotes() {
        assert_eq!(display_command(&[String::new()]), "''");
        assert_eq!(
            display_command(&["/games/it's.iso".to_string()]),
            "'/games/it'\\''s.iso'"
        );
    }

    #[test]
    fn test_format_spawn_error_reports_missing_program_and_cwd() {
        let error = std::io::Error::from_raw_os_error(2);
        let message = format_spawn_error(
            &["/missing/game".to_string()],
            &[("PATH".to_string(), "/usr/bin".to_string())],
            Some("/missing/cwd"),
            &error,
        );

        assert!(message.contains("raw_os_error=Some(2)"));
        assert!(message.contains("program_exists=false"));
        assert!(message.contains("cwd=/missing/cwd (exists=false, directory=false)"));
        assert!(message.contains("WINEPREFIX=\"<unset>\""));
    }
}
