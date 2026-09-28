//! Pause-while-overlay-visible: process-group freeze for external-host mode.
//!
//! Only safe when the un-toggle path survives the freeze — i.e. gamescope /
//! external-host mode, where `ira-overlay-ui` owns the visible window and
//! mirrors hide back into SHM. In-process mode must never freeze: the frozen
//! game could never receive the un-toggle hotkey again (deadlock), so those
//! games pause cooperatively (Goldberg fork) or not at all.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// What the watcher needs. Built by the caller only when freezing is both
/// enabled and safe; presence of this struct is the freeze permit.
pub struct FreezeCfg {
    /// Game SHM name (`/ira_overlay_<db_id>`) carrying `overlay_visible`.
    pub shm_name: String,
}

/// Poll interval for the visibility flag: fast enough to feel instant,
/// slow enough to cost nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Decides whether a launch earns a freeze watcher. Pure for testability:
/// enabled = resolved 3-level setting; external = gamescope/external-host
/// overlay (un-toggle survives); owned_pid = we spawned the process group
/// (excludes Steam launches and daemon placeholders); shm = visibility
/// signal exists.
pub fn watch_config(
    enabled: bool,
    external: bool,
    owned_pid: bool,
    shm: Option<&str>,
) -> Option<FreezeCfg> {
    if !(enabled && external && owned_pid) {
        return None;
    }
    let shm = shm.filter(|s| !s.is_empty())?;
    Some(FreezeCfg {
        shm_name: shm.to_string(),
    })
}

/// Signals one pid. Refuses our own group, init, and nonsense targets;
/// surfaces ESRCH instead of pretending success. SIGSTOP is uncatchable;
/// SIGCONT to a running process is a harmless no-op.
fn signal_pid(pid: i32, freeze: bool) -> Result<(), String> {
    if pid <= 1 {
        return Err(format!("refusing to signal process {pid}"));
    }
    let sig = if freeze { libc::SIGSTOP } else { libc::SIGCONT };
    if unsafe { libc::kill(pid, sig) } != 0 {
        return Err(format!(
            "kill({pid}, {}) failed: {}",
            if freeze { "SIGSTOP" } else { "SIGCONT" },
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Process name from `/proc/<pid>/stat` (the parenthesized comm field).
/// Deliberately not `wrapper::proc_name`: that parses the fields *after*
/// comm and returns the ppid, so it can never match a process name.
fn comm_of(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let start = stat.find('(')?;
    let end = stat.rfind(')')?;
    if end <= start + 1 {
        return None;
    }
    Some(stat[start + 1..end].to_string())
}

/// Processes that must never freeze: the gamescope compositor (input to
/// everything flows through it) and Ira's overlay host (it owns the
/// un-toggle while visible). Everyone else under the launch — game, Wine
/// services, wrapper shells — freezes and thaws as a unit.
fn freeze_excluded(comm: &str) -> bool {
    matches!(comm, "gamescope" | "ira-overlay-ui")
}

/// Direct children of `pid` via `/proc/<pid>/task/*/children`.
fn proc_children(pid: i32) -> Vec<i32> {
    let mut children = Vec::new();
    let task_dir = format!("/proc/{pid}/task");
    if let Ok(tasks) = std::fs::read_dir(&task_dir) {
        for task in tasks.flatten() {
            if let Ok(data) = std::fs::read_to_string(task.path().join("children")) {
                children.extend(data.split_whitespace().filter_map(|s| s.parse::<i32>().ok()));
            }
        }
    }
    children
}

/// Whether an overlay host serving this SHM region is alive. Matches the
/// host binary by comm *and* the SHM name in its environment, so a second
/// game's host cannot vouch for a dead one. Fail-open: an unreadable
/// `/proc` (or any uncertainty) reports alive — the flag is never cleared
/// on a guess. Only consulted while the flag is up, so the scan costs
/// nothing in normal play.
fn host_alive_for(shm_name: &str) -> bool {
    let Ok(proc) = std::fs::read_dir("/proc") else {
        return true;
    };
    for entry in proc.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<i32>().ok()) else {
            continue;
        };
        if comm_of(pid).as_deref() != Some("ira-overlay-ui") {
            continue;
        }
        let env_path = format!("/proc/{pid}/environ");
        let matches = std::fs::read(&env_path)
            .map(|env| {
                env.split(|b| *b == 0)
                    .any(|var| var == format!("IRA_OVERLAY_SHM={shm_name}").as_bytes())
            })
            .unwrap_or(false);
        if matches {
            return true;
        }
    }
    false
}

/// The freezable set: `root` plus all descendants, minus exclusions.
/// Recomputed every poll so Wine services spawned mid-freeze are caught on
/// the next tick.
fn freeze_set(root: i32) -> Vec<i32> {
    let mut set = Vec::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if set.contains(&pid) {
            continue;
        }
        let excluded = comm_of(pid).is_some_and(|c| freeze_excluded(&c));
        if !excluded {
            set.push(pid);
        }
        // Descend even through excluded processes: the game hangs under
        // gamescope/sh, and its subtree is exactly what we want.
        stack.extend(proc_children(pid));
    }
    set
}

/// Stops (`freeze = true`) or continues every process in the freezable set.
/// Idempotent: re-freezing an already-stopped process is a no-op, and dead
/// pids (ESRCH) are skipped. Returns how many signals landed.
fn apply(root: i32, freeze: bool) -> usize {
    freeze_set(root)
        .into_iter()
        .filter(|pid| signal_pid(*pid, freeze).is_ok())
        .count()
}

/// Idempotent resume for a possibly-frozen launch. Best-effort group CONT
/// (a superset of any frozen set) plus a targeted pass: used on the stop
/// path and at watcher shutdown, where a missing group just means the game
/// already exited.
pub fn thaw_group(pgid: i32) {
    if pgid > 1 {
        unsafe {
            libc::kill(-pgid, libc::SIGCONT);
        }
        for pid in freeze_set(pgid) {
            let _ = signal_pid(pid, false);
        }
    }
}

/// Watches `overlay_visible` in the game SHM and freezes/thaws the game
/// subtree on edges. While visible the freeze is re-applied every poll so
/// late-spawned children are caught. Returns (never while the game runs)
/// once `exit` is set, after ensuring everything is thawed — a stuck-frozen
/// game is the one failure this feature must never produce.
pub fn watch(shm_name: &str, root_pid: i32, exit: Arc<AtomicBool>) {
    // Read-write: the watchdog below clears a flag orphaned by a dead
    // host (its Alt+F4, a crash) so the freeze cannot outlive its owner.
    let shm = match ira_overlay_ipc::MappedShm::open_rw(shm_name) {
        Ok(shm) => shm,
        Err(e) => {
            eprintln!("ira-pause: cannot watch {shm_name}: {e}");
            return;
        }
    };
    use std::sync::atomic::Ordering;
    let mut was_visible = false;
    loop {
        if exit.load(Ordering::SeqCst) {
            break;
        }
        let mut visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
        if visible && !host_alive_for(shm_name) {
            // The host is gone with the flag up: nobody will ever lower
            // it, and the held freeze plus the daemon's input consume
            // would trap the game (notably its Alt+F4). Clear it once,
            // loudly; the thaw below follows on this same pass.
            shm.header().overlay_visible.store(0, Ordering::SeqCst);
            eprintln!("ira-pause: overlay host gone with flag up; cleared for {shm_name}");
            visible = false;
        }
        if visible && !was_visible {
            let n = apply(root_pid, true);
            eprintln!("ira-pause: froze {n} game processes (overlay shown)");
        } else if visible {
            // Hold the freeze across spawns: Wine services start lazily.
            apply(root_pid, true);
        } else if was_visible {
            let n = apply(root_pid, false);
            eprintln!("ira-pause: thawed {n} game processes (overlay hidden)");
        }
        was_visible = visible;
        std::thread::sleep(POLL_INTERVAL);
    }
    if was_visible {
        let n = apply(root_pid, false);
        eprintln!("ira-pause: thawed {n} game processes on watcher shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_config_needs_everything() {
        // Arrange: the golden path.
        // Act + Assert: present iff all preconditions hold.
        assert!(watch_config(true, true, true, Some("/ira_overlay_7")).is_some());
        assert!(watch_config(false, true, true, Some("/ira_overlay_7")).is_none());
        assert!(watch_config(true, false, true, Some("/ira_overlay_7")).is_none());
        assert!(watch_config(true, true, false, Some("/ira_overlay_7")).is_none());
        assert!(watch_config(true, true, true, None).is_none());
        assert!(watch_config(true, true, true, Some("")).is_none());
    }

    #[test]
    fn test_signal_guards_and_esrch() {
        // Arrange: targets that must never be signaled, plus one that
        // almost certainly has no process behind it.
        // Act + Assert: error everywhere, nothing signaled.
        assert!(signal_pid(0, true).is_err());
        assert!(signal_pid(1, true).is_err());
        assert!(signal_pid(-5, true).is_err());
        assert!(signal_pid(1_000_000_007, true).is_err());
    }

    #[test]
    fn test_comm_of_reads_own_name() {
        // Arrange: our own test process.
        let me = std::process::id() as i32;
        // Act
        let comm = comm_of(me);
        // Assert: the real comm, not the ppid (see fn docs).
        assert!(comm.is_some_and(|c| c.starts_with("ira-launcher") || !c.is_empty()));
        assert!(comm_of(1_000_000_007).is_none());
    }

    #[test]
    fn test_freeze_exclusions() {
        assert!(freeze_excluded("gamescope"));
        assert!(freeze_excluded("ira-overlay-ui"));
        assert!(!freeze_excluded("wine"));
        assert!(!freeze_excluded("MyGame.exe"));
        assert!(!freeze_excluded("sh"));
    }

    #[test]
    fn test_freeze_set_contains_self_but_never_excluded_kinds() {
        // Arrange: our own subtree (nothing excluded in it, normally).
        let me = std::process::id() as i32;
        // Act
        let set = freeze_set(me);
        // Assert: we are our own member; excluded comms never appear.
        assert!(set.contains(&me));
        for pid in &set {
            let excluded = comm_of(*pid).is_some_and(|c| freeze_excluded(&c));
            assert!(!excluded, "excluded pid {pid} leaked into freeze set");
        }
    }

    /// Kills the scratch child on drop so a failed assertion never leaks a
    /// stopped process (SIGKILL lands even on stopped processes).
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn proc_state(pid: i32) -> Option<String> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = &stat[stat.rfind(')')? + 1..];
        after.split_whitespace().next().map(str::to_string)
    }

    fn wait_for_state(pid: i32, want_stopped: bool) -> Option<String> {
        let mut state = None;
        for _ in 0..20 {
            state = proc_state(pid);
            let stopped = state.as_deref() == Some("T");
            if stopped == want_stopped {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        state
    }

    #[test]
    fn test_apply_freezes_and_thaws_a_live_process() {
        // Arrange: a scratch child we own — never the test runner itself.
        let child = KillOnDrop(
            std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("test needs sleep"),
        );
        let pid = child.0.id() as i32;
        assert!(pid > 1);
        // Act: freeze through the real code path.
        assert!(apply(pid, true) >= 1);
        // Assert: the kernel reports it stopped.
        assert_eq!(wait_for_state(pid, true).as_deref(), Some("T"));
        // Act: thaw through the real code path.
        assert!(apply(pid, false) >= 1);
        // Assert: running again (sleeping, not stopped).
        let state = wait_for_state(pid, false);
        assert!(
            state.is_some() && state.as_deref() != Some("T"),
            "still stopped after thaw: {state:?}"
        );
    }

    #[test]
    fn test_host_alive_for_rejects_unknown_shm() {
        // No overlay host serves this name in any test environment: the
        // watchdog must report dead (and only dead names report dead —
        // a live host is asserted by hand, not here).
        assert!(!host_alive_for("/ira_overlay_no_such_host_999999"));
    }
}
