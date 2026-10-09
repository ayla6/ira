//! Stopping games: native process-group teardown and the Wine graceful
//! shutdown sequence (SIGTERM → wineserver -k → straggler sweep → SIGKILL).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::procfs::{collect_descendants, process_comm};

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

/// Whether `pid` is Wine background infrastructure, matched by process name
/// (truncated to the 15-char comm limit both sides, compared case-insensitively).
fn is_wine_bg(pid: i32) -> bool {
    if let Some(name) = process_comm(pid) {
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

/// Stop a native game process group (no Wine cleanup): gamescope and emulator
/// children share the group, so this also stops the standalone overlay.
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

/// Replaces blind sleeps so a clean exit never pays the full grace period.
fn wait_all_gone(pids: &[i32], deadline: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if pids.iter().all(|p| unsafe { libc::kill(*p, 0) } != 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Reap ALL children regardless of group; safe now that wineserver -k
/// completed, so no conflicting waitpid calls remain.
fn reap_all() {
    loop {
        let ret = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if ret <= 0 {
            break;
        }
    }
}

pub fn stop_game(
    pid: i32,
    wine_exe: Option<&str>,
    wine_prefix: Option<&str>,
    env: &[(String, String)],
) {
    // A pause-frozen game holds SIGTERM pending forever: thaw first so the
    // stop sequence below can actually kill it. Harmless when not frozen.
    super::freeze::thaw_group(pid);
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
        stop_wine_process_group(pid, wine_exe.as_deref(), wine_prefix.as_deref(), &env, &descendants);
    });
}

/// Graceful Wine shutdown on a thread so `stop_game` returns immediately.
fn stop_wine_process_group(
    pid: i32,
    wine_exe: Option<&str>,
    wine_prefix: Option<&str>,
    env: &[(String, String)],
    descendants: &[i32],
) {
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
    if let (Some(exe), Some(prefix)) = (wine_exe, wine_prefix) {
        let wineserver = find_wineserver(exe);
        if let Some(ws) = &wineserver {
            eprintln!("stop: running wineserver -k for prefix {}", prefix);
            let mut cmd = Command::new(ws);
            cmd.arg("-k");
            cmd.env("WINEPREFIX", prefix);
            for (k, v) in env {
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
    watched.extend(descendants);
    wait_all_gone(&watched, Duration::from_secs(3));

    // Step 5: SIGTERM any remaining stragglers.
    for d in descendants {
        let alive = unsafe { libc::kill(*d, 0) } == 0;
        if alive {
            let name = process_comm(*d).unwrap_or_default();
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
    wait_all_gone(descendants, Duration::from_secs(2));

    sigkill_leftovers(pid, descendants);

    // Step 8: Reap all zombies; safe now (wineserver -k done, no blocking
    // waits in flight).
    reap_all();
}

/// Step 7: final fallback SIGKILL.
fn sigkill_leftovers(pid: i32, descendants: &[i32]) {
    if pid > 0 {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    for d in descendants {
        let alive = unsafe { libc::kill(*d, 0) } == 0;
        if alive {
            eprintln!(
                "stop: force killing pid {} ({}) wine_bg={}",
                d,
                process_comm(*d).unwrap_or_default(),
                is_wine_bg(*d)
            );
            unsafe {
                libc::kill(*d, libc::SIGKILL);
            }
        }
    }

    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if alive {
        eprintln!("stop: force killing game pid {}", pid);
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
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
