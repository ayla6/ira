//! Session supervision: the wrapper-process monitor, the input-daemon
//! session monitor, and their shared end-of-session bookkeeping.

use ira_db::{record_session, DbConn};
use ira_input_ipc::{DaemonClient, Event};
use ira_models::AppMessage;
use ira_models::AppSender;
use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::logs::{GameLog, clear_game_log, get_game_log, log_launch_header, tail_file_to_log};

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
    /// Pause-while-overlay-visible freeze permit (`None` = never freeze:
    /// setting off, in-process overlay, unowned pid, or no SHM).
    /// Consumed only by `monitor_process`; the daemon path ignores it.
    pub pause_watch: Option<super::freeze::FreezeCfg>,
}

pub fn monitor_process(mut child: Child, child_pid: i32, ctx: MonitorContext) {
    let game_id = ctx.game_id;

    clear_game_log(game_id);
    let log_buf = get_game_log(game_id);

    log_launch_header(&ctx.command, &ctx.env, &log_buf, "Started initial process");

    // The game writes to a file, never a pipe: our reader threads die with
    // Ira, and a dead pipe SIGPIPEs the game on its next log write.
    let tail_done = Arc::new(AtomicBool::new(false));
    if !ctx.log_file.is_empty() {
        tail_file_to_log(ctx.log_file.clone(), log_buf.clone(), tail_done.clone());
    }

    // Pause watcher on its own thread so the 2s try_wait cadence never delays
    // a freeze/thaw; the group id is the child's pid (spawn_game group leader).
    let freeze_exit = Arc::new(AtomicBool::new(false));
    let freeze_handle = ctx.pause_watch.clone().map(|cfg| {
        let exit = freeze_exit.clone();
        std::thread::spawn(move || super::freeze::watch(&cfg, child_pid, exit))
    });

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

    // Stop the watcher first and thaw unconditionally: finalize (and any
    // stop_game racing it) must never meet a frozen group.
    freeze_exit.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(handle) = freeze_handle {
        let _ = handle.join();
    }
    super::freeze::thaw_group(child_pid);

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

    let ended_at = crate::procfs::unix_now();
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

    // Keep reaping for a few seconds: Wine stragglers dying after the main
    // process would otherwise sit defunct in htop/btop.
    if let Some(pid) = child_pid {
        for _ in 0..10 {
            std::thread::sleep(Duration::from_secs(1));
            reap_zombies(pid);
        }
    }
}

/// Monitor for a game handed to the resident input daemon: no child of our own.
/// The daemon supervises the game and reports back over the IPC socket.
pub fn monitor_session(mut client: DaemonClient, ctx: MonitorContext) {
    let game_id = ctx.game_id;
    clear_game_log(game_id);
    let log_buf = get_game_log(game_id);
    log_launch_header(&ctx.command, &ctx.env, &log_buf, "Started via the input daemon");

    // Freeze watcher once the daemon reports the real pid; no thaw_group — daemon
    // pids are not ours to signal, the watcher thaws its own scope on shutdown.
    let freeze_exit = Arc::new(AtomicBool::new(false));
    let mut freeze_handle: Option<std::thread::JoinHandle<()>> = None;
    let mut child_pid: Option<i32> = None;
    let result = client.wait_session(|event| match event {
        Event::SessionStarted { child_pid: pid, .. } => {
            child_pid = Some(pid);
            ctx.running_games.lock().unwrap().insert(ctx.game_id, pid);
            // Handoff proof: the toggle marker made it into the session env (whether
            // the daemon could arm only shows on the terminal or first flip).
            if ctx
                .env
                .iter()
                .any(|(k, v)| k == "IRA_OVERLAY_DAEMON_TOGGLE" && v == "1")
            {
                log_buf.lock().unwrap().push(
                    "daemon owns the overlay toggle chord (marker in session env)".to_string(),
                );
            }
            if freeze_handle.is_none() {
                if let Some(cfg) = ctx.pause_watch.clone() {
                    let exit = freeze_exit.clone();
                    freeze_handle = Some(std::thread::spawn(move || {
                        super::freeze::watch(&cfg, pid, exit)
                    }));
                }
            }
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
                format!("Controller disconnected: {name} ({path})")
            };
            log_buf.lock().unwrap().push(message);
        }
        Event::ProfileReloaded { path, .. } => {
            log_buf
                .lock()
                .unwrap()
                .push(format!("Controller profile reloaded: {path}"));
        }
        Event::SessionEnded { .. } => {
            freeze_exit.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    // Stop the watcher first and let it thaw: finalize must never meet a
    // frozen game. No group thaw here — daemon pids are not ours to signal.
    freeze_exit.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(handle) = freeze_handle {
        let _ = handle.join();
    }
    let exit_code = match result {
        Ok(code) => Some(code),
        Err(error) => {
            // The daemon connection dropped mid-session; the game itself may still
            // be running, so report the drop instead of a clean end.
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

/// Runs the game's post-exit script synchronously, mirroring pre-launch "wait".
/// Output lands in the in-memory game log next to the game's own output.
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
