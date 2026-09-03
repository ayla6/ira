//! Client side of the daemon protocol: connect (spawning the daemon on
//! demand), send requests, and wait out a game session.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::protocol::{DaemonStatus, Event, LaunchRequest, Request, Response, Wire};

/// Where the daemon listens. Under `XDG_RUNTIME_DIR` (a normal desktop
/// session) the socket lives in a per-user runtime directory; otherwise the
/// temp directory carries the uid so two users never collide.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("ira").join("input.sock");
    }
    let uid = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!("ira-input-{uid}.sock"))
}

pub struct DaemonClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl DaemonClient {
    pub fn connect(path: &Path) -> Result<Self, String> {
        let stream =
            UnixStream::connect(path).map_err(|error| format!("connect {}: {error}", path.display()))?;
        let reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        Ok(Self { stream, reader })
    }

    /// Connects to the daemon, starting one first when the socket is dead.
    /// `binary` is the ira-input executable; the daemon is spawned detached
    /// (its own session) so it can outlive short-lived callers.
    pub fn ensure_connected(binary: &str) -> Result<Self, String> {
        let path = socket_path();
        if let Ok(client) = Self::connect(&path) {
            return Ok(client);
        }
        spawn_daemon(binary)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(client) = Self::connect(&path) {
                return Ok(client);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("ira-input daemon did not accept connections in time".to_string())
    }

    fn send(&mut self, message: &Wire) -> Result<(), String> {
        let mut line = serde_json::to_string(message).map_err(|error| error.to_string())?;
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .map_err(|error| format!("daemon write failed: {error}"))
    }

    fn read(&mut self) -> Result<Wire, String> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|error| format!("daemon read failed: {error}"))?;
        if read == 0 {
            return Err("daemon closed the connection".to_string());
        }
        serde_json::from_str(&line).map_err(|error| format!("bad daemon message: {error}"))
    }

    /// Sends a request and waits for its response, skipping any events that
    /// arrive meanwhile.
    pub fn request(&mut self, request: Request) -> Result<Response, String> {
        self.send(&Wire::Request(request))?;
        loop {
            match self.read()? {
                Wire::Response(response) => return Ok(response),
                Wire::Event(_) => {}
                Wire::Request(_) => {}
            }
        }
    }

    pub fn status(&mut self) -> Result<DaemonStatus, String> {
        match self.request(Request::Status)? {
            Response::Status(status) => Ok(status),
            Response::Error(error) => Err(error),
            _ => Err("unexpected response to status".to_string()),
        }
    }

    /// Hands a game to the daemon. `Err` means the daemon refused (busy,
    /// stale) and the caller should fall back to the wrapper launch.
    pub fn begin_launch(&mut self, request: LaunchRequest) -> Result<(), String> {
        match self.request(Request::Launch(request))? {
            Response::Launched => Ok(()),
            Response::Error(error) => Err(error),
            _ => Err("unexpected response to launch".to_string()),
        }
    }

    /// Hands a game to the daemon and waits it out in one call.
    pub fn launch_and_wait(
        &mut self,
        request: LaunchRequest,
        on_event: impl FnMut(Event),
    ) -> Result<i32, String> {
        self.begin_launch(request)?;
        self.wait_session(on_event)
    }

    /// Pumps events until the session ends; returns the game's exit code.
    /// An unexpected connection loss surfaces as `Err` — the game may still
    /// be running, so callers must not treat it as a clean end.
    pub fn wait_session(&mut self, mut on_event: impl FnMut(Event)) -> Result<i32, String> {
        loop {
            match self.read()? {
                Wire::Event(Event::SessionEnded { code }) => return Ok(code),
                Wire::Event(event) => on_event(event),
                Wire::Response(Response::Error(error)) => return Err(error),
                Wire::Response(_) => {}
                Wire::Request(_) => {}
            }
        }
    }
}

fn spawn_daemon(binary: &str) -> Result<(), String> {
    let path = socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut command = Command::new(binary);
    command.arg("--daemon").stdin(std::process::Stdio::null());
    // Detach into its own session: the daemon must survive the terminal's
    // Ctrl-C and the launcher's process group, and retire on its own idle
    // timer instead.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|error| format!("spawn {binary} --daemon: {error}"))?;
    Ok(())
}
