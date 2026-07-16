use ira_models::AppSender;
use ira_models::AppMessage;
use ira_db::{DbConn, record_session};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::os::unix::process::CommandExt;
use std::time::Duration;
use std::path::Path;

const PR_SET_CHILD_SUBREAPER: i32 = 36;

const WINE_BG_PROCESSES: &[&str] = &[
    "wineserver", "services.exe", "winedevice.exe", "plugplay.exe",
    "explorer.exe", "wineconsole", "svchost.exe", "rpcss.exe",
    "rundll32.exe", "mscorsvw.exe", "iexplore.exe", "winedbg.exe",
    "tabtip.exe", "conhost.exe",
];

fn set_subreaper() {
    let ret = unsafe { libc::prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if ret != 0 {
        eprintln!("Warning: failed to set child subreaper (prctl returned {})", ret);
    }
}

pub fn spawn_game(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_path: Option<&str>,
) -> Result<Child, String> {
    set_subreaper();

    let mut cmd = Command::new(&command[0]);
    for arg in &command[1..] {
        cmd.arg(arg);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (key, val) in env {
        cmd.env(key, val);
    }
    if let Some(path) = log_path {
        if let Some(parent) = Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::File::create(path) {
            Ok(f) => {
                cmd.stdout(Stdio::from(f.try_clone().unwrap_or_else(|_| {
                    std::fs::OpenOptions::new().write(true).open("/dev/null").unwrap()
                })));
                cmd.stderr(Stdio::from(f));
            }
            Err(_) => {
                cmd.stdout(Stdio::null());
                cmd.stderr(Stdio::null());
            }
        }
    } else {
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
    }
    cmd.process_group(0);

    cmd.spawn().map_err(|e| format!("Failed to spawn game process: {}", e))
}

pub fn game_log_path(save_dir: &str, game_id: i64) -> String {
    Path::new(save_dir)
        .join("logs")
        .join(format!("{}.log", game_id))
        .to_string_lossy()
        .into_owned()
}

pub fn monitor_process(
    mut child: Child,
    child_pid: i32,
    sender: &AppSender,
    game_id: i64,
    started_at: i64,
    db: DbConn,
    running_games: Arc<Mutex<HashMap<i64, i32>>>,
) {
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
    let duration = ended_at - started_at;

    if duration < 5 {
        eprintln!("Game {} exited after {}s — possible crash, not recording session", game_id, duration);
    } else {
        if let Err(e) = record_session(&db, game_id, started_at, ended_at) {
            eprintln!("Failed to record play session: {}", e);
        }
        let _ = sender.send(AppMessage::SessionRecorded {
            game_id,
            duration_seconds: duration,
            started_at,
            ended_at,
        });
    }

    running_games.lock().unwrap().remove(&game_id);
    let _ = sender.send(AppMessage::GameStopped(game_id));
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
        let name_trunc = if name_lower.len() > 15 { &name_lower[..15] } else { &name_lower };
        return WINE_BG_PROCESSES.iter().any(|bg| {
            let bg_lower = bg.to_lowercase();
            let bg_trunc = if bg_lower.len() > 15 { &bg_lower[..15] } else { &bg_lower };
            name_trunc == bg_trunc
        });
    }
    false
}

pub fn stop_game_with_wine(pid: i32, wine_exe: Option<&str>, wine_prefix: Option<&str>, env: &[(String, String)]) {
    // Collect descendants BEFORE sending signals — the process tree may
    // change after the game exits (children get reparented).
    let descendants = collect_descendants(pid);

    // Step 1: SIGTERM just the game PID (not the whole group) so the
    // game can exit cleanly while wine infrastructure stays alive.
    unsafe { libc::kill(pid, libc::SIGTERM); }

    let wine_exe = wine_exe.map(|s| s.to_string());
    let wine_prefix = wine_prefix.map(|s| s.to_string());
    let env: Vec<(String, String)> = env.to_vec();
    std::thread::spawn(move || {
        // Step 2: Wait for the game to exit (poll for up to 5 seconds).
        for _ in 0..50 {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive { break; }
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
                let _ = cmd.status();
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
                unsafe { libc::kill(*d, libc::SIGTERM); }
            }
        }

        // Step 6: Wait 2s for stragglers to exit.
        std::thread::sleep(Duration::from_secs(2));

        // Step 7: Final fallback — SIGKILL any survivors.
        for d in &descendants {
            let alive = unsafe { libc::kill(*d, 0) } == 0;
            if alive {
                eprintln!(
                    "stop: force killing pid {} ({}) wine_bg={}",
                    d, proc_name(*d).unwrap_or_default(), is_wine_bg(*d)
                );
                unsafe { libc::kill(*d, libc::SIGKILL); }
            }
        }

        // Kill the original game PID if somehow still alive.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if alive {
            eprintln!("stop: force killing game pid {}", pid);
            unsafe { libc::kill(pid, libc::SIGKILL); }
        }
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
