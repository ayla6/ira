//! Shared `/proc` and clock helpers for the freeze watcher and supervisor.
//! A wrong field here silently disables Wine background-process detection.
use std::time::{SystemTime, UNIX_EPOCH};

/// Session clocks in both monitors use this.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Parses only up to the closing paren, so names with spaces or parens
/// still resolve.
pub(crate) fn process_comm(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let start = stat.find('(')?;
    let end = stat.rfind(')')?;
    if end <= start + 1 {
        return None;
    }
    Some(stat[start + 1..end].to_string())
}

pub(crate) fn proc_children(pid: i32) -> Vec<i32> {
    let mut children = Vec::new();
    let task_dir = format!("/proc/{pid}/task");
    if let Ok(tasks) = std::fs::read_dir(&task_dir) {
        for task in tasks.flatten() {
            if let Ok(data) = std::fs::read_to_string(task.path().join("children")) {
                children.extend(
                    data.split_whitespace()
                        .filter_map(|s| s.parse::<i32>().ok()),
                );
            }
        }
    }
    children
}

/// Recomputed per call so freshly spawned processes are visible.
pub(crate) fn collect_descendants(pid: i32) -> Vec<i32> {
    let mut all = Vec::new();
    let mut stack = vec![pid];
    while let Some(p) = stack.pop() {
        for child in proc_children(p) {
            if !all.contains(&child) {
                all.push(child);
                stack.push(child);
            }
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_comm_reads_own_name_not_ppid() {
        let me = std::process::id() as i32;
        let comm = process_comm(me);
        // Assert: the real comm — never a bare number (the ppid).
        let comm = comm.expect("must read own comm");
        assert!(!comm.is_empty());
        assert!(comm.parse::<i32>().is_err(), "comm is a ppid: {comm}");
        assert!(process_comm(1_000_000_007).is_none());
    }

    #[test]
    fn test_collect_descendants_contains_children() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("test needs sleep");
        let pid = child.id() as i32;
        // Act
        let descendants = collect_descendants(std::process::id() as i32);
        // Assert: the child is reachable from the test runner.
        assert!(descendants.contains(&pid));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn test_unix_now_is_plausible() {
        assert!(unix_now() > 1_700_000_000);
    }
}
