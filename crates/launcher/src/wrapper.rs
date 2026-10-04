//! Process supervision: spawning (`spawn_proc`), stopping (`stop_proc`),
//! session monitors (`monitor`), and log buffers (`logs`). Re-exported here
//! so the `wrapper::` paths used across the app keep working.

pub use crate::logs::{clear_game_log, game_log_path, get_game_log};
pub use crate::monitor::{MonitorContext, monitor_process, monitor_session};
pub use crate::spawn_proc::{spawn_detached, spawn_game};
pub use crate::stop_proc::stop_game;
pub(crate) use crate::logs::{log_launch_header, pipe_lines_to_log};
pub(crate) use crate::monitor::finalize_game;
