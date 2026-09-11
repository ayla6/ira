//! Steam session supervision for daemon-run launches: a background thread
//! watches for the game's processes appearing and disappearing, and flags
//! the session over when they do. The /proc scanning itself lives in
//! `crate::steam`, shared with ira-launcher's launch tracking.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::steam::{request_steam_stop, ProcessIdentity, SteamProcessScanner};

/// Steam session poll cadence. Off the input loop, the scan's /proc walks
/// are the only cost, and nothing user-facing waits on it: game-exit
/// detection already tolerates the 2 s exit grace, so a relaxed interval
/// keeps the watcher near-idle.
const STEAM_POLL_INTERVAL: Duration = Duration::from_millis(250);

const STEAM_START_TIMEOUT: Duration = Duration::from_secs(60);
const STEAM_EXIT_GRACE: Duration = Duration::from_secs(2);

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
            return self
                .empty_since
                .get_or_insert_with(Instant::now)
                .elapsed()
                >= STEAM_EXIT_GRACE;
        }
        self.stop_sent || (launcher_exited && self.started_at.elapsed() >= STEAM_START_TIMEOUT)
    }
}
