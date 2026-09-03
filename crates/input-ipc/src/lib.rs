//! Client/protocol for the resident ira-input daemon: one process owns the
//! physical controllers and virtual devices across every game session.

mod client;
pub mod protocol;

pub use client::{socket_path, DaemonClient};
pub use protocol::{DaemonStatus, Event, LaunchRequest, Request, Response, Wire, PROTOCOL_VERSION};
