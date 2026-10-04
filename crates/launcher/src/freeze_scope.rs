//! The cgroup scope behind pause-while-overlay-visible: `ira-game-<id>`
//! under our own slice. The game tree lives here; freezing the scope stops
//! the whole subtree (nested sandboxes included) with a single write.
//! Rmdir'd on drop — best-effort, since a lingering scope with no members is
//! harmless and reused thawed by the next launch.

use std::time::Duration;

use crate::procfs::{proc_children, process_comm};

/// One game launch's cgroup scope: `ira-game-<id>` under our own slice.
#[derive(Clone, Debug)]
pub struct Scope {
    pub(crate) dir: std::path::PathBuf,
}

impl Scope {
    /// Creates (or reuses) our scope and migrates `pid` into it. Any
    /// failure returns Err and the caller falls back to SIGSTOP — scopes
    /// are an optimization, never a requirement.
    pub fn create(game_id: i64, pid: i32) -> Result<Self, String> {
        Self::create_in(&own_cgroup_dir()?, &format!("ira-game-{game_id}"), pid)
    }

    fn create_in(parent: &std::path::Path, name: &str, pid: i32) -> Result<Self, String> {
        let dir = parent.join(name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(format!("scope mkdir failed: {e}"));
        }
        let scope = Self { dir };
        // A reused scope may be left frozen: always start thawed, or the
        // game would launch already stopped with nobody scheduled to thaw.
        let _ = scope.write_frozen(false);
        scope.migrate(pid)?;
        // Evict strays: a reused scope can hold processes from a previous
        // launch that no longer belong to this tree (e.g. a session
        // leader migrated before it was excluded). They move back to the
        // parent; anything still in the tree stays.
        scope.evict_strays(&freeze_set(pid));
        Ok(scope)
    }

    /// Moves scope members that are not in `keep` back to the parent
    /// cgroup. Best-effort per pid; returns the evicted comms for logging.
    fn evict_strays(&self, keep: &[i32]) -> Vec<String> {
        let mut evicted = Vec::new();
        let Some(parent) = self.dir.parent() else {
            return evicted;
        };
        let target = parent.join("cgroup.procs");
        for member in self.members() {
            if keep.contains(&member) {
                continue;
            }
            if std::fs::write(&target, member.to_string()).is_ok() {
                evicted.push(
                    process_comm(member)
                        .map_or_else(|| member.to_string(), |c| format!("{c}({member})")),
                );
            }
        }
        if !evicted.is_empty() {
            eprintln!("ira-pause: evicted strays [{}]", evicted.join(" "));
        }
        evicted
    }

    /// Moves one pid into the scope. Children born inside inherit it, so
    /// only pre-existing members need (re-)migration.
    pub fn migrate(&self, pid: i32) -> Result<(), String> {
        std::fs::write(self.dir.join("cgroup.procs"), pid.to_string())
            .map_err(|e| format!("scope migrate failed: {e}"))
    }

    /// Freezes (`true`) or thaws the whole subtree, waiting up to a second
    /// for the kernel's `frozen` confirmation. Fire-and-forget past the
    /// timeout: the write itself took effect, only the confirmation lags
    /// (usually behind a task in uninterruptible sleep).
    pub fn set_frozen(&self, frozen: bool) -> Result<(), String> {
        self.write_frozen(frozen)?;
        self.wait_frozen(frozen)
    }

    fn write_frozen(&self, frozen: bool) -> Result<(), String> {
        std::fs::write(
            self.dir.join("cgroup.freeze"),
            if frozen { "1" } else { "0" },
        )
        .map_err(|e| format!("scope freeze write failed: {e}"))
    }

    fn wait_frozen(&self, want: bool) -> Result<(), String> {
        for _ in 0..20 {
            if self.is_frozen() == want {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("scope freeze confirmation timed out".to_string())
    }

    /// Reads the kernel's frozen state (not our last write — the truth).
    pub fn is_frozen(&self) -> bool {
        std::fs::read_to_string(self.dir.join("cgroup.events"))
            .map(|events| events.lines().any(|line| line.trim() == "frozen 1"))
            .unwrap_or(false)
    }

    /// Pids currently in the scope (one per line in cgroup.procs).
    pub fn members(&self) -> Vec<i32> {
        std::fs::read_to_string(self.dir.join("cgroup.procs"))
            .map(|list| {
                list.split_whitespace()
                    .filter_map(|pid| pid.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Best-effort: fails while members remain, and an empty leftover
        // is reused thawed by the next launch anyway.
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Our scope parent: ira's own cgroup directory, under which the user may
/// freely create sub-cgroups (systemd delegates user slices).
fn own_cgroup_dir() -> Result<std::path::PathBuf, String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .map_err(|e| format!("cannot read own cgroup: {e}"))?;
    let path = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| "no unified cgroup hierarchy".to_string())?;
    Ok(std::path::PathBuf::from("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

/// Processes that must never freeze. The walk roots at the session leader,
/// so besides the game tree it also reaches session infrastructure — and
/// freezing any of that breaks the session instead of pausing the game:
/// - `gamescope*`: the compositor itself, under whatever name it runs
///   (`gamescope`, `gamescope-wl`, …). Freezing it stops all presents and
///   kills every session connection — which reads as both "game
///   destroyed" and "UI paused". Prefix-matched on purpose.
/// - `gamescopereaper`: owns session teardown; a frozen reaper cannot reap.
/// - `Xwayland`: the session's display server.
/// - `xdg-dbus-proxy` / `dbus-proxy`: sandboxed portals; freezing stalls
///   file dialogs and saves instead of the game.
/// - `ira-overlay-ui`: owns the un-toggle while visible.
/// - Everyone else under the launch — game, Wine services, wrapper shells,
///   flatpak/bwrap plumbing — freezes and thaws as a unit.
pub(crate) fn freeze_excluded(comm: &str) -> bool {
    comm == "gamescope"
        || comm == "gamescopereaper"
        || comm.starts_with("gamescope-")
        || matches!(
            comm,
            "Xwayland" | "xdg-dbus-proxy" | "dbus-proxy" | "ira-overlay-ui"
        )
}

/// The freezable set: `root` plus all descendants, minus exclusions.
/// Recomputed every poll so Wine services spawned mid-freeze are caught on
/// the next tick. Shared with the SIGSTOP fallback in `freeze`.
pub(crate) fn freeze_set(root: i32) -> Vec<i32> {
    let mut set = Vec::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if set.contains(&pid) {
            continue;
        }
        let excluded = process_comm(pid).is_some_and(|c| freeze_excluded(&c));
        if !excluded {
            set.push(pid);
        }
        // Descend even through excluded processes: the game hangs under
        // gamescope/sh, and its subtree is exactly what we want.
        stack.extend(proc_children(pid));
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scratch scope name unique to this process, so reruns never collide
    /// with a leftover directory.
    fn test_scope_id() -> i64 {
        -(std::process::id() as i64)
    }

    #[test]
    fn test_scope_evicts_non_tree_strays() {
        // Same skip contract: needs a real cgroup.
        let mut keeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let mut stray = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let keeper_pid = keeper.id() as i32;
        let stray_pid = stray.id() as i32;
        let Ok(scope) = Scope::create(test_scope_id() - 2, keeper_pid) else {
            eprintln!("skipping: no writable cgroup here");
            let _ = keeper.kill();
            let _ = keeper.wait();
            let _ = stray.kill();
            let _ = stray.wait();
            return;
        };
        // Arrange: a stray migrated in behind the keeper's back (what a
        // poisoned reused scope looks like).
        scope.migrate(stray_pid).expect("migrate must write");
        assert!(scope.members().contains(&stray_pid));
        // Act: evict everything outside the fresh tree.
        let evicted = scope.evict_strays(&[keeper_pid]);
        // Assert: the stray left, the keeper stayed, both alive.
        assert!(!scope.members().contains(&stray_pid));
        assert!(scope.members().contains(&keeper_pid));
        assert!(evicted.iter().any(|e| e.contains(&stray_pid.to_string())));
        let _ = keeper.kill();
        let _ = keeper.wait();
        let _ = stray.kill();
        let _ = stray.wait();
    }

    /// A fake scope root (plain tmpdir): file plumbing works, kernel
    /// semantics don't. For write-format, parsing, and cleanup tests.
    fn fake_scope() -> (tempfile::TempDir, Scope) {
        let tmp = tempfile::TempDir::new().expect("test needs tmpdir");
        std::fs::write(tmp.path().join("cgroup.procs"), "").unwrap();
        std::fs::write(tmp.path().join("cgroup.freeze"), "0").unwrap();
        std::fs::write(tmp.path().join("cgroup.events"), "populated 0\nfrozen 0\n").unwrap();
        let scope = Scope {
            dir: tmp.path().to_path_buf(),
        };
        (tmp, scope)
    }

    #[test]
    fn test_scope_migrate_writes_pid_file() {
        // Arrange: fake scope plus a scratch child we own.
        let (_tmp, scope) = fake_scope();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let pid = child.id() as i32;
        // Act.
        scope.migrate(pid).expect("migrate must write");
        // Assert: the pid landed in cgroup.procs.
        let written = std::fs::read_to_string(scope.dir.join("cgroup.procs")).unwrap();
        assert_eq!(written, pid.to_string());
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn test_scope_write_and_parse_frozen() {
        // Arrange: fake scope.
        let (_tmp, scope) = fake_scope();
        // Act + Assert: writes land verbatim…
        scope.write_frozen(true).expect("freeze must write");
        assert_eq!(
            std::fs::read_to_string(scope.dir.join("cgroup.freeze")).unwrap(),
            "1"
        );
        // …and the events parser only accepts an exact `frozen 1` line.
        assert!(!scope.is_frozen());
        std::fs::write(scope.dir.join("cgroup.events"), "populated 1\nfrozen 1\n").unwrap();
        assert!(scope.is_frozen());
        std::fs::write(scope.dir.join("cgroup.events"), "populated 1\nfrozen 0\n").unwrap();
        assert!(!scope.is_frozen());
    }

    #[test]
    fn test_scope_wait_times_out_without_kernel() {
        // Arrange: fake scope can never confirm (no kernel behind it).
        let (_tmp, scope) = fake_scope();
        // Act + Assert: the waiter gives up after ~1s instead of hanging.
        assert!(scope.wait_frozen(true).is_err());
    }

    #[test]
    fn test_scope_freeze_roundtrip_on_live_cgroup() {
        // Arrange: a scratch child migrated into a real scope. Skipped
        // where cgroups aren't writable (containers): nothing to prove
        // without the kernel behind the files.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let pid = child.id() as i32;
        let Ok(scope) = Scope::create(test_scope_id(), pid) else {
            eprintln!("skipping: no writable cgroup here");
            let _ = child.kill();
            let _ = child.wait();
            return;
        };
        // Act: freeze through the real code path.
        scope.set_frozen(true).expect("freeze must write");
        // Assert: the kernel reports frozen.
        assert!(scope.is_frozen());
        // Act: thaw through the real code path.
        scope.set_frozen(false).expect("thaw must write");
        // Assert: running again; scope drops (rmdir) on the way out.
        assert!(!scope.is_frozen());
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn test_scope_create_starts_thawed_and_cleans_up() {
        // Same skip contract as the roundtrip above.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let pid = child.id() as i32;
        let Ok(scope) = Scope::create(test_scope_id() - 1, pid) else {
            eprintln!("skipping: no writable cgroup here");
            let _ = child.kill();
            let _ = child.wait();
            return;
        };
        // A reused scope must never start frozen.
        assert!(!scope.is_frozen());
        let dir = scope.dir.clone();
        // Cleanup first: an occupied scope cannot rmdir, and Drop is a
        // silent no-op while occupied — so reap, then drop for real.
        let _ = child.kill();
        let _ = child.wait();
        drop(scope);
        assert!(!dir.exists(), "scope dir leaked: {}", dir.display());
    }
}
