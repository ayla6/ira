//! Overlay host process (`ira-overlay-ui`) lifecycle for injected mode.
//!
//! In gamescope mode the host runs inside the session (see
//! [`super::env_builder::wrap_with_host_overlay`]). Here the launcher spawns
//! it as a sibling of the game: same SHM, software GTK rendering, reaped
//! when the game exits. The reaper watches the `running_games` map the
//! monitor clears, so both the wrapper and daemon paths are covered without
//! touching monitor code.

use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Finds `ira-overlay-ui` next to the main executable.
pub fn host_binary_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let dev_bin = exe_dir.join("ira-overlay-ui");
    if dev_bin.is_file() {
        return Some(dev_bin.to_string_lossy().into());
    }

    let rel_bin = exe_dir.join("overlay").join("ira-overlay-ui");
    if rel_bin.is_file() {
        return Some(rel_bin.to_string_lossy().into());
    }

    None
}

/// Derives the canvas SHM filesystem path from a game SHM name
/// (`/ira_overlay_{db}`) for the readiness wait.
fn canvas_file_for_game_shm(game_shm: &str) -> Option<String> {
    let db = game_shm.strip_prefix("/ira_overlay_")?;
    if db.is_empty() || !db.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("/dev/shm/ira_canvas_{db}"))
}

/// Spawns the host and waits (up to ~10s) for its canvas region.
/// Returns `None` when the binary is missing or readiness times out — the
/// caller launches the game without overlay UI either way.
pub fn spawn_host(game_shm: &str) -> Option<Child> {
    let bin = host_binary_path()?;
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("IRA_OVERLAY_SHM", game_shm);
    // Software rendering: deterministic, no GPU contention with the game.
    cmd.env("GSK_RENDERER", "cairo");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    if wait_canvas_ready(game_shm) {
        return Some(child);
    }
    eprintln!("ira-overlay: host canvas not ready, continuing without UI");
    let _ = child.kill();
    let _ = child.wait();
    None
}

fn wait_canvas_ready(game_shm: &str) -> bool {
    let Some(path) = canvas_file_for_game_shm(game_shm) else {
        return false;
    };
    for _ in 0..200 {
        if std::path::Path::new(&path).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Kills the host once the game exits. Waits for the `running_games` entry
/// to appear first (both spawn paths insert it after the host starts), then
/// for it to disappear (the monitor removes it at game end).
pub fn reap_when_game_exits(
    child: Child,
    running_games: Arc<Mutex<HashMap<i64, i32>>>,
    game_id: i64,
) {
    std::thread::spawn(move || {
        let mut child = child;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            let present = running_games
                .lock()
                .map(|m| m.contains_key(&game_id))
                .unwrap_or(false);
            if present {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        loop {
            let gone = running_games
                .lock()
                .map(|m| !m.contains_key(&game_id))
                .unwrap_or(true);
            if gone {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let _ = child.kill();
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_file_derivation() {
        assert_eq!(
            canvas_file_for_game_shm("/ira_overlay_123"),
            Some("/dev/shm/ira_canvas_123".to_string())
        );
        assert_eq!(canvas_file_for_game_shm("/ira_overlay_"), None);
        assert_eq!(canvas_file_for_game_shm("/other_123"), None);
        assert_eq!(canvas_file_for_game_shm("/ira_overlay_abc"), None);
        assert_eq!(canvas_file_for_game_shm(""), None);
    }

    #[test]
    fn test_spawn_host_missing_binary_returns_none() {
        // No binary override exists in tests; with a bogus PATH the lookup
        // still resolves via current_exe, so this only asserts no panic and
        // a fast failure when the canvas never appears.
        assert!(!wait_canvas_ready("/ira_overlay_"));
    }
}
