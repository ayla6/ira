use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Steam session poll cadence. Off the input loop, the scan's /proc walks
/// are the only cost, and nothing user-facing waits on it: game-exit
/// detection already tolerates the 2 s exit grace, so a relaxed interval
/// keeps the watcher near-idle.
const STEAM_POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// Classifies each user process against the Steam app id exactly once: an
/// environment never changes after exec, so re-reading it on every poll only
/// re-pays the read's cost — and /proc/<pid>/environ serializes on the
/// target's mmap lock, which a running game holds constantly. Pids are
/// re-stat'd every poll (cheap, no mmap lock); only pids born since the
/// last poll — or recycled ones whose start_time moved — get their
/// environment read.
#[derive(Default)]
struct SteamProcessScanner {
    /// The app id the classifications were made against; a different one
    /// invalidates the whole cache.
    app_id: String,
    /// pid -> (start_time at classification, matches the app id).
    classified: HashMap<i32, (u64, bool)>,
}

impl SteamProcessScanner {
    fn scan(&mut self, app_id: &str) -> SteamProcessSnapshot {
        if self.app_id != app_id {
            self.app_id = app_id.to_string();
            self.classified.clear();
        }
        let uid = unsafe { libc::geteuid() };
        let mut processes = HashSet::new();
        let mut complete = true;
        let entries = std::fs::read_dir("/proc");
        for pid in entries
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| *pid != std::process::id() as i32)
        {
            // Known pids need only the start_time check (pid recycling);
            // the uid lookup and environ read happen on classification.
            let Some(start_time) = process_start_time(pid) else {
                // Dead before we could stat it: not an incomplete scan, it
                // simply will not be in this snapshot.
                continue;
            };
            let known = self.classified.get(&pid).copied();
            if let Some((seen_start, matches)) = known {
                if seen_start == start_time {
                    if matches {
                        processes.insert(ProcessIdentity { pid, start_time });
                    }
                    continue;
                }
            }
            let owned = std::fs::metadata(format!("/proc/{pid}"))
                .map(|metadata| metadata.uid() == uid)
                .unwrap_or(false);
            if !owned {
                self.classified.insert(pid, (start_time, false));
                continue;
            }
            match std::fs::read(format!("/proc/{pid}/environ")) {
                Ok(environment) => {
                    if environment_has_steam_app(&environment, app_id) {
                        self.classified.insert(pid, (start_time, true));
                        processes.insert(ProcessIdentity { pid, start_time });
                    } else {
                        self.classified.insert(pid, (start_time, false));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.classified.insert(pid, (start_time, false));
                }
                // Non-dumpable processes (keyring and ssh agents set
                // PR_SET_DUMPABLE=0) stay unreadable for life: classify
                // them once instead of re-attempting — and never letting
                // the scan complete — on every poll.
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    self.classified.insert(pid, (start_time, false));
                }
                Err(_) => {
                    // Transiently unreadable; retry next poll.
                    complete = false;
                }
            }
        }
        SteamProcessSnapshot {
            processes,
            complete,
        }
    }
}

/// start_time of a live process from /proc/<pid>/stat — a read that never
/// takes the target's mmap lock, so polling every pid every cycle stays
/// cheap even while a game churns memory.
fn process_start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_process_start_time(&stat)
}

struct SteamSession {
    app_id: String,
    baseline: HashSet<ProcessIdentity>,
    started_at: Instant,
    seen: bool,
    empty_since: Option<Instant>,
    stop_sent: bool,
}

/// Steam session supervision on its own thread. Every poll walks /proc, and
/// even the incremental scan can stall behind a game's mmap lock — running
/// it on the input loop both burns core time at 10 Hz and freezes input
/// during game loading. The loop only reads the shared flags.
pub(crate) struct SteamWatcher {
    finished: Arc<AtomicBool>,
    launcher_exited: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    /// Set only on daemon shutdown-by-signal, matching the previous
    /// behavior: a natural end (or a daemon error) never stops the game.
    request_stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl SteamWatcher {
    pub(crate) fn spawn(app_id: &str) -> Self {
        let finished = Arc::new(AtomicBool::new(false));
        let launcher_exited = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let request_stop = Arc::new(AtomicBool::new(false));
        let flags = SteamFlags {
            finished: Arc::clone(&finished),
            launcher_exited: Arc::clone(&launcher_exited),
            stop: Arc::clone(&stop),
            request_stop: Arc::clone(&request_stop),
        };
        let app_id = app_id.to_string();
        let handle = thread::Builder::new()
            .name("ira-steam-watch".to_string())
            .spawn(move || watch_steam_session(app_id, flags))
            .ok();
        Self {
            finished,
            launcher_exited,
            stop,
            request_stop,
            handle,
        }
    }

    pub(crate) fn game_session_over(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) fn launcher_exited(&self) {
        self.launcher_exited.store(true, Ordering::Release);
    }

    /// Shutdown-by-signal: ask the watcher to stop the game through Steam
    /// on its way out.
    pub(crate) fn request_stop_and_join(&mut self) {
        self.request_stop.store(true, Ordering::Release);
        self.join();
    }

    fn join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SteamWatcher {
    fn drop(&mut self) {
        self.join();
    }
}

struct SteamFlags {
    finished: Arc<AtomicBool>,
    launcher_exited: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    request_stop: Arc<AtomicBool>,
}

fn watch_steam_session(app_id: String, flags: SteamFlags) {
    let mut scanner = SteamProcessScanner::default();
    let mut session = SteamSession::new_with(&app_id, &mut scanner);
    loop {
        if flags.stop.load(Ordering::Acquire) {
            if flags.request_stop.load(Ordering::Acquire) {
                session.request_stop();
            }
            return;
        }
        thread::sleep(STEAM_POLL_INTERVAL);
        if session.poll(&mut scanner, flags.launcher_exited.load(Ordering::Acquire)) {
            // Natural end: the game session is over, the game itself keeps
            // running as far as Steam is concerned.
            break;
        }
    }
    flags.finished.store(true, Ordering::Release);
}


impl SteamSession {
    fn new_with(app_id: &str, scanner: &mut SteamProcessScanner) -> Self {
        Self {
            app_id: app_id.to_string(),
            baseline: scanner.scan(app_id).processes,
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

    fn poll(&mut self, scanner: &mut SteamProcessScanner, launcher_exited: bool) -> bool {
        let snapshot = scanner.scan(&self.app_id);
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_scanner_classifies_steam_process_across_scans() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env("SteamAppId", "123456789")
            .spawn()
            .unwrap();
        let mut scanner = SteamProcessScanner::default();
        let snapshot = scanner.scan("123456789");
        assert!(snapshot.complete);
        assert!(
            snapshot
                .processes
                .iter()
                .any(|identity| identity.pid == child.id() as i32),
            "a process carrying SteamAppId must be detected"
        );
        // Steady state: the cached classification keeps reporting it, and a
        // different app id never matches it.
        assert!(scanner
            .scan("123456789")
            .processes
            .iter()
            .any(|identity| identity.pid == child.id() as i32));
        assert!(scanner
            .scan("999")
            .processes
            .iter()
            .all(|identity| identity.pid != child.id() as i32));
        let _ = child.kill();
        let _ = child.wait();
    }
}