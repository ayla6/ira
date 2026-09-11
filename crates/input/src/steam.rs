//! Detecting running Steam game processes by app id, and asking Steam to
//! stop one. Steam (and Proton) export the app id in the game's environment
//! (`SteamAppId`, `SteamGameId`, `STEAM_COMPAT_APP_ID`), so a /proc walk that
//! reads each process's environ finds every process belonging to a game
//! without knowing its executable name. Shared by the input daemon's session
//! supervision and by game launch tracking in ira-launcher.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub start_time: u64,
}

pub struct SteamProcessSnapshot {
    pub processes: HashSet<ProcessIdentity>,
    pub complete: bool,
}

/// Classifies each user process against the Steam app id exactly once: an
/// environment never changes after exec, so re-reading it on every poll only
/// re-pays the read's cost — and /proc/<pid>/environ serializes on the
/// target's mmap lock, which a running game holds constantly. Pids are
/// re-stat'd every poll (cheap, no mmap lock); only pids born since the
/// last poll — or recycled ones whose start_time moved — get their
/// environment read.
#[derive(Default)]
pub struct SteamProcessScanner {
    /// The app id the classifications were made against; a different one
    /// invalidates the whole cache.
    app_id: String,
    /// pid -> (start_time at classification, matches the app id).
    classified: HashMap<i32, (u64, bool)>,
}

impl SteamProcessScanner {
    pub fn scan(&mut self, app_id: &str) -> SteamProcessSnapshot {
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

/// Asks Steam to shut a running app down through its own front door. Far
/// safer than signalling the game's process tree, which Steam owns and may
/// share with its runtime components.
pub fn request_steam_stop(app_id: &str) {
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
