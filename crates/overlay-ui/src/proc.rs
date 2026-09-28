//! Process-tree lookup for the session leader.
//!
//! The overlay host is the session's grandchild, so no launcher plumbing
//! is needed to find whose window to watch (desktop thread) or whose
//! session to exit (exit-game action).

/// Parent pid from `/proc/<pid>/stat`, skipping the comm field (which may
/// contain spaces and parentheses).
fn ppid_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
    fields.next()?;
    fields.next()?.parse().ok()
}

/// Walks up the process tree for the session leader: `gamescope` for
/// plain wrapped launches, `gamescopereaper` when the session runs under
/// the reaper (daemon launches — the reaper outlives gamescope itself, so
/// matching only `gamescope` finds nothing).
pub(crate) fn session_leader_pid() -> Option<u32> {
    let mut pid = std::process::id();
    for _ in 0..16 {
        pid = ppid_of(pid)?;
        if pid <= 1 {
            break;
        }
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            && matches!(comm.trim(), "gamescope" | "gamescopereaper")
        {
            return Some(pid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ppid_of_self_matches_getppid() {
        let me = std::process::id();
        assert_eq!(ppid_of(me), Some(unsafe { libc::getppid() } as u32));
    }

    #[test]
    fn test_session_leader_pid_does_not_panic() {
        // Ancestors here are never a session; asserts the walk shape only.
        let _ = session_leader_pid();
    }
}
