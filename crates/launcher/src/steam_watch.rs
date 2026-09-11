//! Tracking a Steam game launched via `steam -applaunch`: Steam hands the
//! launch off to (or boots) its own client, so the spawned process is never
//! the game and exits as soon as the handoff completes. The supervisor
//! watches /proc for the game's real processes (see `ira_input::steam`) and
//! drives the same session bookkeeping as every other launch: the
//! running-games registry, the in-app log buffer, and the end-of-session
//! messages. Steam records playtime in its own appmanifests, which Ira
//! reads back — so sessions are never counted here.

use std::collections::HashSet;
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use ira_input::steam::{ProcessIdentity, SteamProcessScanner};

use crate::wrapper::{
    clear_game_log, finalize_game, get_game_log, log_launch_header, pipe_lines_to_log,
    MonitorContext,
};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// A game whose processes have all been gone this long has ended its session.
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// When the spawned launcher process is gone and no game process ever
/// appeared, nothing will: stop tracking so the game does not stay marked
/// as running after a failed launch.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// Session state for one tracked Steam launch. The game counts as running
/// from the first poll on which any matching process exists — including
/// processes that were already running before the launch, because Steam
/// answers a launch request for a running game by focusing it, and that
/// focus is the session the user asked for.
struct SteamLaunchSession {
    seen: bool,
    empty_since: Option<Instant>,
}

impl SteamLaunchSession {
    fn new() -> Self {
        Self {
            seen: false,
            empty_since: None,
        }
    }

    /// One poll: `complete` is whether the process scan could read every
    /// process, `matched` whether any live game process exists. Returns
    /// true when the session is over.
    fn poll(&mut self, complete: bool, matched: bool, now: Instant) -> bool {
        if !complete {
            // A scan that could not read every process proves nothing about
            // the game being gone; never end a session on one.
            self.empty_since = None;
            return false;
        }
        if matched {
            self.seen = true;
            self.empty_since = None;
            return false;
        }
        if !self.seen {
            return false;
        }
        let Some(grace_start) = self.empty_since else {
            self.empty_since = Some(now);
            return false;
        };
        now.duration_since(grace_start) >= EXIT_GRACE
    }

    /// The launch never produced a game process and the spawned launcher
    /// process has exited: give up.
    fn launch_abandoned(&self, started_at: Instant, launcher_exited: bool, now: Instant) -> bool {
        !self.seen && launcher_exited && now.duration_since(started_at) >= START_TIMEOUT
    }
}

/// Supervises one Steam launch on a background thread. `child` is the
/// spawned `steam` (or `xdg-open` fallback) process — its output streams
/// into the game log and its exit marks the launcher as handed off.
///
/// The session ends — running-games cleanup and the `GameStopped` message,
/// no playtime — when the game's processes have been gone for the exit
/// grace, when the launch is abandoned, or when `stop_game` drops the game
/// from the running registry (it has already asked Steam to stop the app).
pub fn monitor_steam_launch(child: Option<Child>, app_id: String, ctx: MonitorContext) {
    thread::spawn(move || {
        let game_id = ctx.game_id;
        clear_game_log(game_id);
        let log_buf = get_game_log(game_id);
        log_launch_header(&ctx, &log_buf, "Started via Steam");

        let mut child = child;
        if let Some(child) = child.as_mut() {
            pipe_lines_to_log(child.stdout.take(), log_buf.clone());
            pipe_lines_to_log(child.stderr.take(), log_buf.clone());
        }

        let mut scanner = SteamProcessScanner::default();
        let mut session = SteamLaunchSession::new();
        let started_at = Instant::now();
        loop {
            if !ctx.running_games.lock().unwrap().contains_key(&game_id) {
                // stop_game removed us: it asked Steam to stop the app, so
                // report the session over instead of waiting for processes
                // that may never have started.
                break;
            }
            let launcher_exited = child
                .as_mut()
                .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some()));
            let snapshot = scanner.scan(&app_id);
            let matched = !snapshot.processes.is_empty();
            if matched {
                adopt_game_pid(&ctx, &snapshot.processes);
            }
            let now = Instant::now();
            if session.poll(snapshot.complete, matched, now)
                || session.launch_abandoned(started_at, launcher_exited, now)
            {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }

        finalize_game(&ctx, &log_buf, None, None);
        // The Steam client typically outlives the game; reap the launcher
        // whenever it exits so it never sits around as a zombie.
        if let Some(child) = child.as_mut() {
            let _ = child.wait();
        }
    });
}

/// A placeholder pid (0) marks the session as running until the first game
/// process shows up; store its pid so stop diagnostics see something real.
/// Never replacing a real pid keeps the entry stable across the game's own
/// process churn (launchers, restarts, Proton stubs).
fn adopt_game_pid(ctx: &MonitorContext, processes: &HashSet<ProcessIdentity>) {
    let Some(pid) = processes.iter().map(|identity| identity.pid).min() else {
        return;
    };
    let mut running = ctx.running_games.lock().unwrap();
    if let Some(slot) = running.get_mut(&ctx.game_id) {
        if *slot <= 0 {
            *slot = pid;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SteamLaunchSession, START_TIMEOUT};
    use std::time::{Duration, Instant};

    #[test]
    fn test_poll_marks_session_over_after_exit_grace() {
        let now = Instant::now();
        let mut session = SteamLaunchSession::new();
        assert!(!session.poll(true, true, now));
        // The first empty poll starts the grace clock; a later one expires it.
        assert!(!session.poll(true, false, now + Duration::from_secs(1)));
        assert!(!session.poll(true, false, now + Duration::from_millis(1500)));
        assert!(session.poll(true, false, now + Duration::from_millis(3500)));
    }

    #[test]
    fn test_poll_never_ends_on_incomplete_scan_or_before_first_match() {
        let now = Instant::now();
        let mut session = SteamLaunchSession::new();
        // An unreadable process must not end a session that had a game.
        assert!(!session.poll(true, true, now));
        assert!(!session.poll(false, false, now + Duration::from_secs(60)));
        assert!(!session.poll(true, false, now + Duration::from_secs(60)));
        // A game that never appeared is never "over" on its own either —
        // only launch_abandoned ends that wait.
        let mut fresh = SteamLaunchSession::new();
        assert!(!fresh.poll(true, false, now + Duration::from_secs(3600)));
    }

    #[test]
    fn test_poll_recovers_when_processes_return_within_grace() {
        let now = Instant::now();
        let mut session = SteamLaunchSession::new();
        assert!(!session.poll(true, true, now));
        // Games restart their own processes; a reappearance during the
        // grace window restarts it.
        assert!(!session.poll(true, false, now + Duration::from_secs(1)));
        assert!(!session.poll(true, true, now + Duration::from_millis(1800)));
        assert!(!session.poll(true, false, now + Duration::from_secs(3)));
        assert!(session.poll(true, false, now + Duration::from_millis(5200)));
    }

    #[test]
    fn test_launch_abandoned_requires_timeout_and_launcher_exit() {
        let now = Instant::now();
        let started_at = now - Duration::from_secs(61);
        let session = SteamLaunchSession::new();
        assert!(!session.launch_abandoned(started_at, false, now));
        assert!(session.launch_abandoned(started_at, true, now));
        // Inside the timeout the launch may still be booting Steam.
        let young = now - Duration::from_secs(START_TIMEOUT.as_secs() - 10);
        assert!(!session.launch_abandoned(young, true, now));
        // A game that did appear is a live session, never abandoned.
        let mut seen = SteamLaunchSession::new();
        assert!(!seen.poll(true, true, now));
        assert!(!seen.launch_abandoned(started_at, true, now));
    }
}
