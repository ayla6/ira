//! Host-side input engine: argument parsing, the mapping session loop, and
//! the subsystems it drives (steam supervision, profile watching, sensor
//! pipeline). Shared by the wrapper binary and, later, the daemon server.

pub mod args;
pub mod server;
pub mod session;

mod profile_monitor;
mod signals;
mod steam;
mod target_env;
mod trace;

pub use args::{parse_arguments, Arguments};
pub use session::run_session;

/// Events a session reports to its host while it runs; the daemon server
/// broadcasts them to every connected client.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// The session's game process was spawned.
    SessionStarted {
        child_pid: i32,
        command: Vec<String>,
    },
    /// One line of the game's stdout or stderr.
    Output(String),
    Controller {
        connected: bool,
        name: String,
        path: String,
    },
    ProfileReloaded { path: String },
}

pub(crate) use profile_monitor::ProfileMonitor;
pub(crate) use steam::SteamWatcher;
pub(crate) use target_env::{
    ignored_device_for_target, inject_flatpak_target_env, sdl_mapping_for_backend,
};
pub(crate) use trace::TraceState;
