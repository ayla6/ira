use std::time::{Duration, Instant};

/// Which periodic work is live this pass; every entry earns a slot in the
/// loop's wait timeout.
pub(crate) const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
pub(crate) const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const PROFILE_POLL_INTERVAL: Duration = Duration::from_millis(50);


pub(crate) struct LoopActivity {
    pub(crate) tick: bool,
    pub(crate) tick_interval: Duration,
    pub(crate) child: bool,
    pub(crate) profile: bool,
    pub(crate) cursor: bool,
    pub(crate) focus: bool,
}

pub(crate) struct LoopSchedule {
    pub(crate) sensor: Instant,
    pub(crate) process: Instant,
    pub(crate) profile: Instant,
    pub(crate) cursor: Instant,
    pub(crate) focus: Instant,
}

impl LoopSchedule {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            sensor: now,
            process: now,
            profile: now,
            cursor: now,
            focus: now,
        }
    }

    pub(crate) fn timeout(&self, activity: LoopActivity) -> Option<Duration> {
        [
            activity.tick.then(|| remaining(self.sensor, activity.tick_interval)),
            activity.child.then(|| remaining(self.process, PROCESS_POLL_INTERVAL)),
            activity.profile.then(|| remaining(self.profile, PROFILE_POLL_INTERVAL)),
            activity.cursor.then(|| remaining(self.cursor, FOCUS_POLL_INTERVAL)),
            activity.focus.then(|| remaining(self.focus, FOCUS_POLL_INTERVAL)),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

pub(crate) fn remaining(last_run: Instant, interval: Duration) -> Duration {
    interval.saturating_sub(last_run.elapsed())
}

/// The minimum time the loop parks between passes when scheduled work is due.
/// A pass that outlasts the tick interval leaves its deadline already
/// elapsed, and polling on a due deadline returns instantly — the loop would
/// re-run the tick, miss the deadline again, and busy-spin one core. One
/// poll quantum is free: `poll_timeout_ms` rounds every shorter wait up to
/// 1 ms anyway.
const POLL_FLOOR: Duration = Duration::from_millis(1);

pub(crate) fn floor_poll_timeout(timeout: Option<Duration>) -> Option<Duration> {
    timeout.map(|timeout| timeout.max(POLL_FLOOR))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_loop_schedule_blocks_without_periodic_work() {
        let schedule = LoopSchedule::new();
        assert_eq!(
            schedule.timeout(LoopActivity {
                tick: false,
                tick_interval: Duration::from_millis(1),
                child: false,
                profile: false,
                cursor: false,
                focus: false,
            }),
            None
        );
    }

    #[test]
    fn test_floor_poll_timeout_bumps_due_deadline_to_one_quantum() {
        // A due deadline must still park the loop for one poll quantum, or a
        // pass that outlasts its tick interval busy-spins the core.
        assert_eq!(
            floor_poll_timeout(Some(Duration::ZERO)),
            Some(POLL_FLOOR)
        );
        assert_eq!(
            floor_poll_timeout(Some(Duration::from_nanos(500))),
            Some(POLL_FLOOR)
        );
        assert_eq!(
            floor_poll_timeout(Some(Duration::from_millis(5))),
            Some(Duration::from_millis(5))
        );
        // No periodic work still means "block until an input event".
        assert_eq!(floor_poll_timeout(None), None);
    }

    #[test]
    fn test_loop_schedule_wakes_for_focus_while_ticks_pause() {
        // Paused sessions skip ticks: the focus deadline must keep the loop
        // waking so input can resume, and it stays one poll interval out.
        let schedule = LoopSchedule::new();
        thread::sleep(Duration::from_millis(5));
        let timeout = schedule
            .timeout(LoopActivity {
                tick: false,
                tick_interval: Duration::from_millis(1),
                child: false,
                profile: false,
                cursor: false,
                focus: true,
            })
            .expect("focus tracking must have a deadline while paused");
        assert!(timeout <= Duration::from_millis(250));
    }

    #[test]
    fn test_loop_schedule_uses_earliest_active_deadline() {
        let schedule = LoopSchedule::new();
        let timeout = schedule
            .timeout(LoopActivity {
                tick: true,
                tick_interval: Duration::from_millis(1),
                child: true,
                profile: true,
                cursor: true,
                focus: true,
            })
            .expect("active work must have a deadline");
        assert!(timeout <= Duration::from_millis(1));
    }
}