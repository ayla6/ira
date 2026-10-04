//! Pause-while-overlay-visible: freeze for external-host mode.
//!
//! Two backends, same trigger points. The cgroup freezer is primary: one
//! write stops the whole game subtree atomically with zero signals, so
//! supervisors never see stop/continue traffic and reparented children
//! cannot escape (membership survives fork and setsid). The per-PID
//! SIGSTOP/SIGCONT walk stays as the fallback where scopes are
//! unavailable (foreign cgroup setup, v1 kernels).
//!
//! Only safe when the un-toggle path survives the freeze — i.e. gamescope /
//! external-host mode, where `ira-overlay-ui` owns the visible window and
//! mirrors hide back into SHM. In-process mode must never freeze: the frozen
//! game could never receive the un-toggle hotkey again (deadlock), so those
//! games pause cooperatively (Goldberg fork) or not at all.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::procfs::process_comm;

pub use super::freeze_scope::Scope;
use super::freeze_scope::freeze_set;

/// What the watcher needs. Built by the caller only when freezing is both
/// enabled and safe; presence of this struct is the freeze permit.
#[derive(Clone, Debug)]
pub struct FreezeCfg {
    /// Game SHM name (`/ira_overlay_<db_id>`) carrying `overlay_visible`.
    pub shm_name: String,
    /// Database game id: names the cgroup scope (`ira-game-<id>`).
    pub game_id: i64,
    /// Whether the SIGSTOP walk may run. True where we own the pid
    /// (wrapper launches); false where the daemon supervises it — signals
    /// from outside could race session teardown, so there it is freezer
    /// or nothing. Thaw always falls back to targeted SIGCONT: continue
    /// cannot stop anything, so it stays safe everywhere.
    pub signals_allowed: bool,
}

/// Poll interval for the visibility flag: fast enough to feel instant,
/// slow enough to cost nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Decides whether a launch earns a freeze watcher. Pure for testability:
/// enabled = resolved 3-level setting; external = gamescope/external-host
/// overlay (un-toggle survives); owned_pid = we spawned the process group
/// (excludes Steam launches and daemon placeholders — and selects the
/// SIGSTOP fallback: the daemon supervises its own pids, so from outside
/// only the signal-free freezer may touch them); shm = visibility signal
/// exists.
pub fn watch_config(
    enabled: bool,
    external: bool,
    owned_pid: bool,
    shm: Option<&str>,
    game_id: i64,
) -> Option<FreezeCfg> {
    if !(enabled && external && owned_pid) {
        return None;
    }
    let shm = shm.filter(|s| !s.is_empty())?;
    Some(FreezeCfg {
        shm_name: shm.to_string(),
        game_id,
        signals_allowed: owned_pid,
    })
}

/// Daemon-supervised variant: the daemon owns the pid, so the SIGSTOP
/// fallback is forbidden outright — freezer or nothing. Same visibility
/// preconditions otherwise. Safe where the wrapper path is: the freezer
/// sends no signals, so there is no stop/continue traffic to race
/// session teardown, and fatal signals still land on frozen tasks.
pub fn watch_config_daemon(
    enabled: bool,
    external: bool,
    shm: Option<&str>,
    game_id: i64,
) -> Option<FreezeCfg> {
    if !(enabled && external) {
        return None;
    }
    let shm = shm.filter(|s| !s.is_empty())?;
    Some(FreezeCfg {
        shm_name: shm.to_string(),
        game_id,
        signals_allowed: false,
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
        if process_comm(pid).as_deref() != Some("ira-overlay-ui") {
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

/// Stops (`freeze = true`) or continues every process in the freezable set.
/// Idempotent: re-freezing an already-stopped process is a no-op, and dead
/// pids (ESRCH) are skipped. Returns how many signals landed.
fn apply(root: i32, freeze: bool) -> usize {
    freeze_set(root)
        .into_iter()
        .filter(|pid| signal_pid(*pid, freeze).is_ok())
        .count()
}

/// Stops (`freeze = true`) or continues the freezable set: one scope
/// write when the cgroup backend is up, otherwise the SIGSTOP/SIGCONT
/// walk where allowed. The scope is created lazily on the first freeze
/// edge (no spawn-path side effects for games that never pause) and
/// reused after; creation failure degrades once, loudly.
///
/// Migration runs only when `migrate` is set (edges, plus a slow cadence
/// while held): every `cgroup.procs` write can block on an
/// uninterruptible task, so the 250ms hold loop must not sweep. Thaw
/// always falls back to targeted SIGCONT.
///
/// Edge outcome logging lives with the callers (`log_scope`), so the
/// hold stays silent.
fn freeze_with(
    cfg: &FreezeCfg,
    root_pid: i32,
    scope: &mut Option<Scope>,
    freeze: bool,
    migrate: bool,
) {
    if scope.is_none() && freeze {
        match Scope::create(cfg.game_id, root_pid) {
            Ok(s) => {
                *scope = Some(s);
            }
            Err(e) if cfg.signals_allowed => {
                eprintln!("ira-pause: {e}; SIGSTOP fallback for {}", cfg.shm_name);
            }
            Err(e) => {
                eprintln!("ira-pause: {e}; pausing unavailable for {}", cfg.shm_name);
            }
        }
    }
    if let Some(scope) = scope {
        if migrate {
            // Migrate the current tree so late spawns join the scope
            // (children born inside inherit it and need no migration).
            for pid in freeze_set(root_pid) {
                let _ = scope.migrate(pid);
            }
        }
        match scope.set_frozen(freeze) {
            Ok(()) => return,
            Err(e) => eprintln!("ira-pause: scope freeze failed ({e}), falling back"),
        }
    }
    if cfg.signals_allowed || !freeze {
        apply(root_pid, freeze);
    }
}

/// One terse line per edge: member count plus the kernel's own frozen
/// state — not a migrated count, which never measured the thaw.
fn log_scope(what: &str, scope: &Option<Scope>) {
    match scope {
        Some(scope) => eprintln!(
            "ira-pause: {what} scope ({} members, frozen={})",
            scope.members().len(),
            scope.is_frozen(),
        ),
        None => eprintln!("ira-pause: {what} via signals"),
    }
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
pub fn watch(cfg: &FreezeCfg, root_pid: i32, exit: Arc<AtomicBool>) {
    // Read-write: the watchdog below clears a flag orphaned by a dead
    // host (its Alt+F4, a crash) so the freeze cannot outlive its owner.
    let shm = match ira_overlay_ipc::MappedShm::open_rw(&cfg.shm_name) {
        Ok(shm) => shm,
        Err(e) => {
            eprintln!("ira-pause: cannot watch {}: {e}", cfg.shm_name);
            return;
        }
    };
    // The cgroup scope lives here (created lazily on the first freeze
    // edge) and drops at shutdown, removing its directory when empty.
    let mut scope: Option<Scope> = None;
    // Last tree→scope migration: the hold loop re-sweeps on a slow
    // cadence instead of every poll, because each cgroup.procs write can
    // block on an uninterruptible task and must never stall the 250ms
    // watcher. Wine services spawning mid-visibility join on the next
    // sweep at the latest.
    let mut last_sweep = Instant::now();
    use std::sync::atomic::Ordering;
    let mut was_visible = false;
    loop {
        if exit.load(Ordering::SeqCst) {
            break;
        }
        let mut visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
        if visible && !host_alive_for(&cfg.shm_name) {
            // The host is gone with the flag up: nobody will ever lower
            // it, and the held freeze plus the daemon's input consume
            // would trap the game (notably its Alt+F4). Clear it once,
            // loudly; the thaw below follows on this same pass.
            shm.header().overlay_visible.store(0, Ordering::SeqCst);
            eprintln!(
                "ira-pause: overlay host gone with flag up; cleared for {}",
                cfg.shm_name
            );
            visible = false;
        }
        if visible && !was_visible {
            freeze_with(cfg, root_pid, &mut scope, true, true);
            last_sweep = Instant::now();
            log_scope("froze", &scope);
        } else if visible {
            // Hold the freeze across spawns: Wine services start lazily.
            let sweep = last_sweep.elapsed() >= Duration::from_secs(5);
            freeze_with(cfg, root_pid, &mut scope, true, sweep);
            if sweep {
                last_sweep = Instant::now();
            }
        } else if was_visible {
            freeze_with(cfg, root_pid, &mut scope, false, false);
            log_scope("thawed", &scope);
        }
        was_visible = visible;
        std::thread::sleep(POLL_INTERVAL);
    }
    if was_visible {
        freeze_with(cfg, root_pid, &mut scope, false, false);
        log_scope("thawed on watcher shutdown", &scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::freeze_scope::freeze_excluded;

    #[test]
    fn test_watch_config_needs_everything() {
        // Arrange: the golden path.
        // Act + Assert: present iff all preconditions hold.
        assert!(watch_config(true, true, true, Some("/ira_overlay_7"), 7).is_some());
        assert!(watch_config(false, true, true, Some("/ira_overlay_7"), 7).is_none());
        assert!(watch_config(true, false, true, Some("/ira_overlay_7"), 7).is_none());
        assert!(watch_config(true, true, false, Some("/ira_overlay_7"), 7).is_none());
        assert!(watch_config(true, true, true, None, 7).is_none());
        assert!(watch_config(true, true, true, Some(""), 7).is_none());
    }

    #[test]
    fn test_watch_config_owned_pid_selects_signal_fallback() {
        // Wrapper launches own the pid: signals allowed.
        let cfg = watch_config(true, true, true, Some("/ira_overlay_7"), 7).unwrap();
        assert!(cfg.signals_allowed);
        assert_eq!(cfg.game_id, 7);
    }

    #[test]
    fn test_watch_config_daemon_forbids_signals() {
        // Arrange: daemon-supervised launch, same visibility gates.
        // Act + Assert: arms without pid ownership, but never signals.
        let cfg = watch_config_daemon(true, true, Some("/ira_overlay_7"), 7).unwrap();
        assert!(!cfg.signals_allowed);
        assert_eq!(cfg.shm_name, "/ira_overlay_7");
        assert!(watch_config_daemon(false, true, Some("/ira_overlay_7"), 7).is_none());
        assert!(watch_config_daemon(true, false, Some("/ira_overlay_7"), 7).is_none());
        assert!(watch_config_daemon(true, true, None, 7).is_none());
        assert!(watch_config_daemon(true, true, Some(""), 7).is_none());
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
    fn test_freeze_exclusions() {
        assert!(freeze_excluded("gamescope"));
        assert!(freeze_excluded("gamescope-wl"));
        assert!(freeze_excluded("gamescopereaper"));
        assert!(freeze_excluded("Xwayland"));
        assert!(freeze_excluded("xdg-dbus-proxy"));
        assert!(freeze_excluded("dbus-proxy"));
        assert!(freeze_excluded("ira-overlay-ui"));
        assert!(!freeze_excluded("wine"));
        assert!(!freeze_excluded("MyGame.exe"));
        assert!(!freeze_excluded("retroarch"));
        assert!(!freeze_excluded("bwrap"));
        assert!(!freeze_excluded("flatpak"));
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
            let excluded = process_comm(*pid).is_some_and(|c| freeze_excluded(&c));
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
