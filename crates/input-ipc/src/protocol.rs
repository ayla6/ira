//! Wire protocol for the resident ira-input daemon: JSON lines over a Unix
//! stream socket. The client sends [`Request`]s; the daemon answers with
//! [`Response`]s and broadcasts [`Event`]s to every connected client. Both
//! travel as [`Wire`] so a reader can always parse a line the same way.

use serde::{Deserialize, Serialize};

/// Bumped on any breaking message change; the daemon answers `status` with
/// its version so a stale client can bail out cleanly.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Launch(LaunchRequest),
    Status,
    /// `stop_running` also stops an active game session (SIGTERM semantics).
    Shutdown { stop_running: bool },
    /// Switch a running session's controller layout: the session reloads
    /// from `profile` as if that file had been edited in place. Addressed
    /// by the launch `tag`, so the sender needs no connection to the
    /// session's launch.
    ReloadProfile { tag: i64, profile: String },
}

/// A game session handed to the daemon. `command` is the fully built game
/// command line (wine/gamescope layers included) — the daemon is its last
/// stop before exec. `env` is the complete environment for the child, the
/// same list the launcher would apply itself.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaunchRequest {
    pub command: Vec<String>,
    /// Pin discovery to one controller path instead of the first pad found.
    pub device: Option<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<String>,
    pub profile: Option<String>,
    pub calibration: Option<String>,
    pub pause_unfocused: bool,
    pub trace: bool,
    pub motion_port: Option<u16>,
    pub steam_app_id: Option<String>,
    /// Client-chosen routing key (Ira's game id) so later requests can
    /// address this session without holding the launch connection open.
    pub tag: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// The session was accepted; lifecycle arrives as events, tagged with
    /// the session id that identifies them to their owner.
    Launched { session: u64 },
    Status(DaemonStatus),
    /// A reload request reached its session.
    Reloaded,
    Bye,
    /// A named-field variant on purpose: serde cannot serialize an
    /// internally tagged newtype variant that holds a bare string, so the
    /// errors below would never reach a client and leave it waiting.
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub pid: u32,
    pub protocol_version: u32,
    pub session_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The session's child was spawned; `child_pid` is the game process.
    SessionStarted {
        session: u64,
        child_pid: i32,
        command: Vec<String>,
    },
    SessionEnded { session: u64, code: i32 },
    /// One line of the game's stdout or stderr, forwarded for the in-app
    /// game log.
    Output { session: u64, line: String },
    /// Physical controller presence, daemon-wide (not session-scoped).
    Controller {
        connected: bool,
        name: String,
        path: String,
    },
    ProfileReloaded { session: u64, path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Wire {
    Request(Request),
    Response(Response),
    Event(Event),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_request_roundtrip() {
        let request = Wire::Request(Request::Launch(LaunchRequest {
            command: vec!["game".into(), "--fullscreen".into()],
            device: Some("/dev/input/event42".into()),
            env: vec![("DISPLAY".into(), ":0".into())],
            working_dir: Some("/games/dir".into()),
            profile: Some("/profiles/x.json".into()),
            calibration: None,
            pause_unfocused: true,
            trace: false,
            motion_port: Some(26760),
            steam_app_id: None,
            tag: Some(12),
        }));
        let line = serde_json::to_string(&request).unwrap();
        let parsed: Wire = serde_json::from_str(&line).unwrap();
        match parsed {
            Wire::Request(Request::Launch(launch)) => {
                assert_eq!(launch.command, ["game", "--fullscreen"]);
                assert_eq!(launch.device.as_deref(), Some("/dev/input/event42"));
                assert_eq!(launch.working_dir.as_deref(), Some("/games/dir"));
                assert!(launch.pause_unfocused);
                assert_eq!(launch.motion_port, Some(26760));
                assert_eq!(launch.tag, Some(12));
            }
            other => panic!("wrong wire message: {other:?}"),
        }
    }

    #[test]
    fn test_wire_reload_request_roundtrip() {
        let request = Wire::Request(Request::ReloadProfile {
            tag: 12,
            profile: "/profiles/y.json".into(),
        });
        let line = serde_json::to_string(&request).unwrap();
        match serde_json::from_str::<Wire>(&line).unwrap() {
            Wire::Request(Request::ReloadProfile { tag, profile }) => {
                assert_eq!(tag, 12);
                assert_eq!(profile, "/profiles/y.json");
            }
            other => panic!("wrong wire message: {other:?}"),
        }
    }

    #[test]
    fn test_wire_error_response_serializes_with_a_message() {
        // Regression: internally tagged newtype variants with a bare string
        // fail at serialization time, so errors never reached clients.
        let response = Wire::Response(Response::Error {
            message: "no running session for game 41".into(),
        });
        let line = serde_json::to_string(&response).expect("error responses must serialize");
        match serde_json::from_str::<Wire>(&line).unwrap() {
            Wire::Response(Response::Error { message }) => {
                assert_eq!(message, "no running session for game 41");
            }
            other => panic!("wrong wire message: {other:?}"),
        }
    }

    #[test]
    fn test_wire_event_roundtrip() {
        for event in [
            Event::SessionStarted {
                session: 1,
                child_pid: 42,
                command: vec!["game".into()],
            },
            Event::SessionEnded { session: 1, code: 3 },
            Event::Output {
                session: 1,
                line: "game: started".into(),
            },
            Event::Controller {
                connected: true,
                name: "8BitDo Ultimate 2".into(),
                path: "/dev/input/event42".into(),
            },
            Event::ProfileReloaded {
                session: 1,
                path: "/profiles/x.json".into(),
            },
        ] {
            let line = serde_json::to_string(&Wire::Event(event)).unwrap();
            match serde_json::from_str::<Wire>(&line).unwrap() {
                Wire::Event(_) => {}
                other => panic!("wrong wire message: {other:?}"),
            }
        }
    }
}
