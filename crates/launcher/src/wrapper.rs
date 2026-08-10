use ira_db::{record_session, DbConn};
use ira_models::AppMessage;
use ira_models::AppSender;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
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
    _log_path: Option<&str>,
) -> Result<Child, String> {
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

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.process_group(0);

    cmd.spawn()
        .map_err(|e| format!("Failed to spawn game process: {}", e))
}

pub fn game_log_path(save_dir: &str, game_id: i64) -> String {
    Path::new(save_dir)
        .join("logs")
        .join(format!("{}.log", game_id))
        .to_string_lossy()
        .into_owned()
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
}

pub fn monitor_process(mut child: Child, child_pid: i32, ctx: MonitorContext) {
    let game_id = ctx.game_id;

    // Clear previous log and get the shared buffer for this game.
    clear_game_log(game_id);
    let log_buf = get_game_log(game_id);

    // Log the command and env vars to the in-memory buffer (matching Lutris/umu format).
    // This runs in the monitor thread so it doesn't block the main thread.
    {
        let mut log = log_buf.lock().unwrap();
        log.push(format!(
            "Started initial process from {}",
            ctx.command.join(" ")
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

    // Spawn separate threads for stdout and stderr so both are read live.
    // Reading them sequentially would block stderr until stdout EOFs (i.e. never,
    // while the game is running), losing all shim/VK-layer diagnostic output.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let log_buf_out = log_buf.clone();
    std::thread::spawn(move || {
        let Some(mut out) = stdout else { return };
        let mut buf = [0u8; 4096];
        let mut pending = String::new();
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if !pending.is_empty() {
                        log_buf_out.lock().unwrap().push(pending);
                    }
                    break;
                }
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    while let Some(pos) = pending.find('\n') {
                        let line = pending[..pos].trim_end_matches('\r').to_string();
                        log_buf_out.lock().unwrap().push(line);
                        pending = pending[pos + 1..].to_string();
                    }
                }
            }
        }
    });

    let log_buf_err = log_buf.clone();
    std::thread::spawn(move || {
        let Some(mut err) = stderr else { return };
        let mut buf = [0u8; 4096];
        let mut pending = String::new();
        loop {
            match err.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if !pending.is_empty() {
                        log_buf_err.lock().unwrap().push(pending);
                    }
                    break;
                }
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    while let Some(pos) = pending.find('\n') {
                        let line = pending[..pos].trim_end_matches('\r').to_string();
                        log_buf_err.lock().unwrap().push(line);
                        pending = pending[pos + 1..].to_string();
                    }
                }
            }
        }
    });

    loop {
        std::thread::sleep(Duration::from_secs(2));
        reap_zombies(child_pid);

        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {}
            Err(_) => break,
        }
    }

    reap_zombies(child_pid);

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
    for _ in 0..10 {
        std::thread::sleep(Duration::from_secs(1));
        reap_zombies(child_pid);
    }
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

/// Reap ALL children of the calling process, regardless of process group.
/// Used after stop_game_with_wine finishes killing — by that point
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

pub fn stop_game_with_wine(
    pid: i32,
    wine_exe: Option<&str>,
    wine_prefix: Option<&str>,
    env: &[(String, String)],
) {
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

        // Step 4: Wait for wineserver cleanup.
        std::thread::sleep(Duration::from_secs(3));

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

        // Step 6: Wait 2s for stragglers to exit.
        std::thread::sleep(Duration::from_secs(2));

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
