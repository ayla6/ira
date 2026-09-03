//! The resident input daemon. One process owns the physical controllers and
//! virtual devices across game sessions: clients hand it fully built game
//! commands over the IPC socket, it runs the same session loop the wrapper
//! binary uses, and it retires once nothing needs it — no stray grabs, no
//! squat udp/26760, no stale socket after its last client goes away.

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use ira_input_ipc::{
    socket_path, DaemonStatus, Event, LaunchRequest, Request, Response, Wire, PROTOCOL_VERSION,
};

use super::args::Arguments;
use super::session::run_session;
use super::signals::{install_signal_handlers, STOP_REQUESTED};
use super::SessionEvent;

/// How long an unused daemon lingers before exiting. Long enough to cover a
/// launcher restart between games; short enough that a dead owner leaves
/// nothing behind.
const IDLE_EXIT: Duration = Duration::from_secs(10);
/// Cadence for pumping session events and re-checking sockets.
const PUMP_INTERVAL: Duration = Duration::from_millis(50);
/// One last chance for a mid-windup session to observe a shutdown request.
const SESSION_STOP_PATIENCE: Duration = Duration::from_secs(5);

pub fn run_daemon() -> Result<i32, String> {
    run_daemon_on(&socket_path())
}

struct Client {
    stream: UnixStream,
    buffer: Vec<u8>,
}

struct SessionHandle {
    events: Receiver<SessionEvent>,
    done: Receiver<Result<i32, String>>,
}

pub fn run_daemon_on(path: &Path) -> Result<i32, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    // A connectable socket means another daemon already owns the devices;
    // exit quietly instead of fighting over the input paths.
    if UnixStream::connect(path).is_ok() {
        eprintln!("ira-input: daemon already listening on {}", path.display());
        return Ok(0);
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("bind {}: {error}", path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    install_signal_handlers();
    eprintln!("ira-input: daemon listening on {}", path.display());

    let mut clients: Vec<Client> = Vec::new();
    let mut session: Option<SessionHandle> = None;
    let mut shutdown = false;
    let mut idle_since: Option<Instant> = None;

    loop {
        if STOP_REQUESTED.load(Ordering::Relaxed) {
            shutdown = true;
        }
        if shutdown && session.is_none() {
            break;
        }
        pump_session(&mut session, &mut clients);
        let timeout = loop_timeout(session.is_some(), idle_since);
        let ready = poll_sockets(&listener, &clients, timeout)?;
        if ready[0] {
            accept(&listener, &mut clients);
        }
        for index in (0..clients.len()).rev() {
            let requests = read_client(&mut clients, index);
            for request in requests {
                process_request(&mut clients, index, &mut session, &mut shutdown, request);
            }
            if index >= clients.len() {
                // The client was dropped while its requests were processed.
                continue;
            }
        }
        if clients.is_empty() && session.is_none() {
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= IDLE_EXIT {
                eprintln!("ira-input: idle with no clients or sessions; exiting");
                break;
            }
        } else {
            idle_since = None;
        }
    }

    stop_running_session(&mut session);
    let _ = std::fs::remove_file(path);
    eprintln!("ira-input: daemon exited");
    Ok(0)
}

fn loop_timeout(session_active: bool, idle_since: Option<Instant>) -> Duration {
    if session_active {
        return PUMP_INTERVAL;
    }
    match idle_since {
        Some(since) => IDLE_EXIT
            .saturating_sub(since.elapsed())
            .min(PUMP_INTERVAL)
            .max(Duration::from_millis(1)),
        None => PUMP_INTERVAL,
    }
}

/// Drains a live session's event channel, broadcasts new events, and moves
/// ended sessions into history.
fn pump_session(session: &mut Option<SessionHandle>, clients: &mut Vec<Client>) {
    let Some(handle) = session.as_ref() else {
        return;
    };
    while let Ok(event) = handle.events.try_recv() {
        broadcast(clients, &session_event_to_protocol(&event));
    }
    if let Ok(result) = handle.done.try_recv() {
        let code = result.unwrap_or(-1);
        eprintln!("ira-input: session ended with code {code}");
        broadcast(clients, &Event::SessionEnded { code });
        *session = None;
    }
}

fn session_event_to_protocol(event: &SessionEvent) -> Event {
    match event {
        SessionEvent::SessionStarted { child_pid, command } => Event::SessionStarted {
            child_pid: *child_pid,
            command: command.clone(),
        },
        SessionEvent::Output(line) => Event::Output { line: line.clone() },
        SessionEvent::Controller {
            connected,
            name,
            path,
        } => Event::Controller {
            connected: *connected,
            name: name.clone(),
            path: path.clone(),
        },
        SessionEvent::ProfileReloaded { path } => {
            Event::ProfileReloaded { path: path.clone() }
        }
    }
}

fn poll_sockets(
    listener: &UnixListener,
    clients: &[Client],
    timeout: Duration,
) -> Result<Vec<bool>, String> {
    let mut descriptors = vec![libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    descriptors.extend(clients.iter().map(|client| libc::pollfd {
        fd: client.stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }));
    let timeout_ms = timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
    let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as u64, timeout_ms) };
    if result < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
        return Err(format!("daemon poll failed: {}", std::io::Error::last_os_error()));
    }
    Ok(descriptors.iter().map(|d| d.revents & libc::POLLIN != 0).collect())
}

fn accept(listener: &UnixListener, clients: &mut Vec<Client>) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                eprintln!("ira-input: client connected");
                clients.push(Client {
                    stream,
                    buffer: Vec::new(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

/// Reads everything currently available from one client and returns the
/// complete requests found. The client is dropped from the list on EOF or
/// socket error.
fn read_client(clients: &mut Vec<Client>, index: usize) -> Vec<Request> {
    let mut chunk = [0u8; 4096];
    loop {
        match clients[index].stream.read(&mut chunk) {
            Ok(0) => {
                eprintln!("ira-input: client disconnected");
                clients.remove(index);
                return Vec::new();
            }
            Ok(read) => clients[index].buffer.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => {
                clients.remove(index);
                return Vec::new();
            }
        }
    }
    let mut requests = Vec::new();
    while let Some(newline) = clients[index]
        .buffer
        .iter()
        .position(|byte| *byte == b'\n')
    {
        let line: Vec<u8> = clients[index].buffer.drain(..=newline).collect();
        if let Ok(Wire::Request(request)) = serde_json::from_slice(&line[..line.len() - 1]) {
            requests.push(request);
        }
    }
    requests
}

fn process_request(
    clients: &mut Vec<Client>,
    index: usize,
    session: &mut Option<SessionHandle>,
    shutdown: &mut bool,
    request: Request,
) {
    match request {
        Request::Status => respond(
            clients,
            index,
            Response::Status(DaemonStatus {
                pid: std::process::id(),
                protocol_version: PROTOCOL_VERSION,
                session_active: session.is_some(),
            }),
        ),
        Request::Launch(launch) => {
            if session.is_some() {
                respond(
                    clients,
                    index,
                    Response::Error("a game session is already running".to_string()),
                );
                return;
            }
            match start_session(launch) {
                Ok(handle) => {
                    *session = Some(handle);
                    respond(clients, index, Response::Launched);
                }
                Err(error) => respond(clients, index, Response::Error(error)),
            }
        }
        Request::Shutdown { stop_running } => {
            if session.is_some() && !stop_running {
                respond(
                    clients,
                    index,
                    Response::Error("a game session is running".to_string()),
                );
                return;
            }
            if session.is_some() {
                // The session loop watches this flag and stops the game the
                // same way a SIGTERM to the old wrapper did.
                STOP_REQUESTED.store(true, Ordering::Relaxed);
            }
            *shutdown = true;
            respond(clients, index, Response::Bye);
        }
    }
}

fn start_session(launch: LaunchRequest) -> Result<SessionHandle, String> {
    if launch.command.is_empty() {
        return Err("launch request has an empty command".to_string());
    }
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let arguments = Arguments {
        device: launch
            .device
            .as_deref()
            .map(std::path::PathBuf::from),
        profile: launch.profile.as_deref().map(Path::new).map(Path::to_path_buf),
        calibration: launch.calibration.as_deref().map(Path::new).map(Path::to_path_buf),
        pause_unfocused: launch.pause_unfocused,
        motion_port: launch.motion_port,
        vdf_import: None,
        list: false,
        probe_sensors: false,
        steam_app_id: launch.steam_app_id.clone(),
        trace: false,
        command: launch.command.clone(),
        daemon: false,
        no_daemon: false,
        env: Some(launch.env.clone()),
        working_dir: launch.working_dir.clone(),
        events: Some(event_tx),
    };
    std::thread::Builder::new()
        .name("ira-session".to_string())
        .spawn(move || {
            let _ = done_tx.send(run_session(arguments));
        })
        .map_err(|error| format!("spawn session thread: {error}"))?;
    Ok(SessionHandle {
        events: event_rx,
        done: done_rx,
    })
}

fn respond(clients: &mut Vec<Client>, index: usize, response: Response) {
    if index >= clients.len() {
        return;
    }
    let message = match serde_json::to_string(&Wire::Response(response)) {
        Ok(message) => message,
        Err(_) => return,
    };
    if send_line(&mut clients[index].stream, &message).is_err() {
        clients.remove(index);
    }
}

fn broadcast(clients: &mut Vec<Client>, event: &Event) {
    let Ok(message) = serde_json::to_string(&Wire::Event(event.clone())) else {
        return;
    };
    clients.retain_mut(|client| send_line(&mut client.stream, &message).is_ok());
}

fn send_line(stream: &mut UnixStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")
}

/// On shutdown, gives a live session the same treatment the wrapper's
/// SIGTERM handler did: the session loop sees the stop flag and stops the
/// game before the daemon goes away.
fn stop_running_session(session: &mut Option<SessionHandle>) {
    if session.is_none() {
        return;
    }
    STOP_REQUESTED.store(true, Ordering::Relaxed);
    let Some(handle) = session.take() else {
        return;
    };
    let deadline = Instant::now() + SESSION_STOP_PATIENCE;
    loop {
        match handle.done.recv_timeout(Duration::from_millis(100)) {
            Ok(_) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                eprintln!("ira-input: session did not acknowledge shutdown in time");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_input_ipc::DaemonClient;

    fn temp_test_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ira-input-daemon-test-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Connects once the server thread has bound its socket.
    fn wait_for_server(path: &Path) -> DaemonClient {
        loop {
            match DaemonClient::connect(path) {
                Ok(client) => return client,
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    fn session_request(command: Vec<String>, profile: Option<String>) -> LaunchRequest {
        LaunchRequest {
            device: None,
            command,
            env: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
            working_dir: None,
            profile,
            calibration: None,
            pause_unfocused: false,
            motion_port: Some(0),
            steam_app_id: None,
        }
    }

    fn write_profile(path: &Path, backend: crate::VirtualGamepadBackend) {
        std::fs::write(
            path,
            serde_json::to_vec(&crate::InputProfile::default_gamepad_for_backend(backend))
                .unwrap(),
        )
        .unwrap();
    }

    fn shutdown_and_join(
        client: &mut DaemonClient,
        server: std::thread::JoinHandle<Result<i32, String>>,
    ) {
        match client
            .request(Request::Shutdown { stop_running: true })
            .unwrap()
        {
            Response::Bye => {}
            other => panic!("expected bye, got {other:?}"),
        }
        let _ = server.join();
    }

    #[test]
    fn test_daemon_serves_status_and_full_session() {
        let dir = temp_test_dir("session");
        let path = dir.join("test.sock");
        let server = std::thread::spawn({
            let path = path.clone();
            move || run_daemon_on(&path)
        });
        let mut client = wait_for_server(&path);

        let status = client.status().unwrap();
        assert_eq!(status.protocol_version, PROTOCOL_VERSION);
        assert!(!status.session_active);

        let mut started = None;
        let code = client
            .launch_and_wait(
                session_request(vec!["sleep".into(), "0.3".into()], None),
                |event| {
                    if let Event::SessionStarted { child_pid, .. } = event {
                        started = Some(child_pid);
                    }
                },
            )
            .unwrap();
        assert_eq!(code, 0);
        assert!(started.is_some(), "session start event must arrive");

        let status = client.status().unwrap();
        assert!(!status.session_active);

        shutdown_and_join(&mut client, server);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_daemon_hot_swaps_controller_kind_on_profile_reload() {
        let dir = temp_test_dir("hotswap");
        let path = dir.join("test.sock");
        let profile_path = dir.join("profile.json");
        write_profile(&profile_path, crate::VirtualGamepadBackend::XInput);
        let server = std::thread::spawn({
            let path = path.clone();
            move || run_daemon_on(&path)
        });
        let mut client = wait_for_server(&path);

        // Rewrite the profile with a different controller kind mid-session.
        // The first write happens once the session is live; a second flip
        // back exercises a rebuild in the other direction. Establishing the
        // watch also triggers one same-content reload, hence the >= 2 bar.
        let rewrites = profile_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            write_profile(&rewrites, crate::VirtualGamepadBackend::DirectInput);
            std::thread::sleep(Duration::from_millis(300));
            write_profile(&rewrites, crate::VirtualGamepadBackend::XInput);
        });

        let mut reloads = 0;
        let code = client
            .launch_and_wait(
                session_request(
                    vec!["sleep".into(), "2".into()],
                    Some(profile_path.display().to_string()),
                ),
                |event| {
                    if matches!(event, Event::ProfileReloaded { .. }) {
                        reloads += 1;
                    }
                },
            )
            .unwrap();
        assert_eq!(code, 0);
        assert!(
            reloads >= 2,
            "controller-kind changes must hot-reload, not refuse (saw {reloads})"
        );

        shutdown_and_join(&mut client, server);
        std::fs::remove_dir_all(&dir).ok();
    }
}
