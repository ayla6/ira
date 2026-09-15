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
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use ira_input_ipc::{
    socket_path, DaemonStatus, Event, LaunchRequest, Request, Response, Wire, PROTOCOL_VERSION,
};

use super::args::Arguments;
use super::hub::{self, HubCommand, HubHandle};
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
    id: u64,
    /// The launch's client-chosen routing key (Ira's game id), so requests
    /// like a layout switch can address the session later.
    tag: Option<i64>,
    events: Receiver<SessionEvent>,
    done: Receiver<Result<i32, String>>,
    /// A no-game session giving an idle controller its default behaviour
    /// while the app is open. It holds no child process and never blocks a
    /// shutdown.
    desktop: bool,
    /// The desktop session's layout path, so a changed wish can replace it.
    desktop_profile: Option<String>,
}

/// The app's standing request for how an idle controller should behave
/// while it is open. Games supersede it; it resumes when they end.
#[derive(Debug, Clone, PartialEq)]
struct DesktopWish {
    profile: Option<String>,
    calibration: Option<String>,
    motion_port: Option<u16>,
}

/// Every live session plus the state sessions are created from.
#[derive(Default)]
struct SessionTable {
    sessions: Vec<SessionHandle>,
    next_session_id: u64,
    desktop_wish: Option<DesktopWish>,
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
    // One hub for the daemon's lifetime: it owns every physical controller
    // and routes them to whichever session holds focus.
    let (controller_tx, controller_rx) = std::sync::mpsc::channel();
    let hub = hub::spawn(controller_tx);
    let mut table = SessionTable::default();
    let mut shutdown = false;
    let mut idle_since: Option<Instant> = None;

    loop {
        if STOP_REQUESTED.load(Ordering::Relaxed) {
            shutdown = true;
        }
        if shutdown && table.sessions.is_empty() {
            break;
        }
        pump_sessions(&mut table, &mut clients, &hub, shutdown);
        drain_controller_presence(&controller_rx, &mut clients);
        let timeout = loop_timeout(!table.sessions.is_empty(), idle_since);
        let ready = poll_sockets(&listener, &clients, timeout)?;
        if ready[0] {
            accept(&listener, &mut clients);
        }
        for index in (0..clients.len()).rev() {
            let (requests, disconnected) = read_client(&mut clients, index);
            for request in requests {
                process_request(
                    &mut clients,
                    index,
                    &hub,
                    &mut table,
                    &mut shutdown,
                    request,
                );
            }
            if disconnected && clients.is_empty() {
                // The last app went away: the desktop behaviour was theirs.
                // Releasing the desktop session also lets the daemon idle
                // out instead of holding the pad forever.
                table.desktop_wish = None;
                stop_desktop_session(&hub, &table.sessions);
            }
        }
        if clients.is_empty() && table.sessions.is_empty() {
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= IDLE_EXIT {
                eprintln!("ira-input: idle with no clients or sessions; exiting");
                break;
            }
        } else {
            idle_since = None;
        }
    }

    stop_running_sessions(&mut table.sessions);
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

/// Drains every live session's event channel, broadcasts new events, and
/// removes sessions that ended. When the last session ends while an app
/// still holds a desktop wish, the desktop-default session resumes.
fn pump_sessions(table: &mut SessionTable, clients: &mut Vec<Client>, hub: &HubHandle, shutdown: bool) {
    for index in (0..table.sessions.len()).rev() {
        while let Ok(event) = table.sessions[index].events.try_recv() {
            broadcast(
                clients,
                &session_event_to_protocol(table.sessions[index].id, &event),
            );
        }
        if let Ok(result) = table.sessions[index].done.try_recv() {
            let handle = table.sessions.remove(index);
            let code = result.unwrap_or(-1);
            eprintln!("ira-input: session {} ended with code {}", handle.id, code);
            hub.send(HubCommand::Unsubscribe(handle.id));
            broadcast(clients, &Event::SessionEnded { session: handle.id, code });
        }
    }
    if table.sessions.is_empty() && !shutdown {
        if let Some(wish) = table.desktop_wish.clone() {
            if !clients.is_empty() {
                start_desktop_session(&wish, hub, table);
            }
        }
    }
}

fn start_desktop_session(wish: &DesktopWish, hub: &HubHandle, table: &mut SessionTable) {
    table.next_session_id += 1;
    let launch = LaunchRequest {
        command: Vec::new(),
        device: None,
        env: Vec::new(),
        working_dir: None,
        profile: wish.profile.clone(),
        calibration: wish.calibration.clone(),
        pause_unfocused: false,
        trace: false,
        motion_port: wish.motion_port,
        steam_app_id: None,
        tag: None,
    };
    match start_session(launch, table.next_session_id, hub.clone(), true) {
        Ok(handle) => {
            eprintln!(
                "ira-input: desktop session {} running the controller's default behaviour",
                handle.id
            );
            table.sessions.push(handle);
        }
        Err(error) => eprintln!("ira-input: desktop session failed to start: {error}"),
    }
}

/// Asks the desktop-default session, if any, to finish. Returns whether one
/// was running.
fn stop_desktop_session(hub: &HubHandle, sessions: &[SessionHandle]) -> bool {
    let Some(session) = sessions.iter().find(|session| session.desktop) else {
        return false;
    };
    hub.send(HubCommand::Stop(session.id));
    true
}

/// Forwards controller presence changes from the hub to every client.
fn drain_controller_presence(
    controller_rx: &std::sync::mpsc::Receiver<(bool, String, String)>,
    clients: &mut Vec<Client>,
) {
    while let Ok((connected, name, path)) = controller_rx.try_recv() {
        broadcast(
            clients,
            &Event::Controller {
                connected,
                name,
                path,
            },
        );
    }
}

fn session_event_to_protocol(session: u64, event: &SessionEvent) -> Event {
    match event {
        SessionEvent::SessionStarted { child_pid, command } => Event::SessionStarted {
            session,
            child_pid: *child_pid,
            command: command.clone(),
        },
        SessionEvent::Output(line) => Event::Output {
            session,
            line: line.clone(),
        },
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
            Event::ProfileReloaded { session, path: path.clone() }
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
/// complete requests found, plus whether the client was dropped (EOF or
/// socket error).
fn read_client(clients: &mut Vec<Client>, index: usize) -> (Vec<Request>, bool) {
    let mut chunk = [0u8; 4096];
    loop {
        match clients[index].stream.read(&mut chunk) {
            Ok(0) => {
                eprintln!("ira-input: client disconnected");
                clients.remove(index);
                return (Vec::new(), true);
            }
            Ok(read) => clients[index].buffer.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => {
                clients.remove(index);
                return (Vec::new(), true);
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
    (requests, false)
}

fn process_request(
    clients: &mut Vec<Client>,
    index: usize,
    hub: &HubHandle,
    table: &mut SessionTable,
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
                session_active: table.sessions.iter().any(|session| !session.desktop),
            }),
        ),
        Request::Launch(launch) => {
            // A game supersedes the desktop behaviour; the wish stays on
            // file so the idle behaviour resumes when the last game ends.
            stop_desktop_session(hub, &table.sessions);
            table.next_session_id += 1;
            match start_session(launch, table.next_session_id, hub.clone(), false) {
                Ok(handle) => {
                    table.sessions.push(handle);
                    respond(
                        clients,
                        index,
                        Response::Launched {
                            session: table.next_session_id,
                        },
                    );
                }
                Err(error) => respond(clients, index, Response::Error { message: error }),
            }
        }
        Request::DesktopDefault {
            enabled,
            profile,
            calibration,
            motion_port,
        } => {
            table.desktop_wish = enabled.then_some(DesktopWish {
                profile,
                calibration,
                motion_port,
            });
            match table.desktop_wish.clone() {
                None => {
                    stop_desktop_session(hub, &table.sessions);
                }
                Some(wish) => {
                    // A running desktop session with a different layout gets
                    // replaced; a matching one just keeps running. When no
                    // game holds the pad, the session starts right away —
                    // otherwise the wish resumes after the games end.
                    let running_layout = table
                        .sessions
                        .iter()
                        .find(|session| session.desktop)
                        .map(|session| session.desktop_profile.clone());
                    if running_layout.as_ref() != Some(&wish.profile) {
                        if table.sessions.iter().all(|session| session.desktop) {
                            stop_desktop_session(hub, &table.sessions);
                        }
                        if table.sessions.is_empty() && !clients.is_empty() {
                            start_desktop_session(&wish, hub, table);
                        }
                    }
                }
            }
            respond(clients, index, Response::Applied);
        }
        Request::ReloadProfile { tag, profile } => {
            let session = table
                .sessions
                .iter()
                .find(|session| session.tag == Some(tag))
                .map(|session| session.id);
            match session {
                Some(id) => {
                    hub.send(HubCommand::ReloadProfile {
                        id,
                        path: std::path::PathBuf::from(profile),
                    });
                    respond(clients, index, Response::Reloaded);
                }
                None => respond(
                    clients,
                    index,
                    Response::Error {
                        message: format!("no running session for game {tag}"),
                    },
                ),
            }
        }
        Request::Shutdown { stop_running } => {
            if !stop_running && table.sessions.iter().any(|session| !session.desktop) {
                respond(
                    clients,
                    index,
                    Response::Error {
                    message: "game sessions are running".to_string(),
                },
                );
                return;
            }
            if table.sessions.iter().any(|session| !session.desktop) {
                // Every session loop watches this flag and stops its game
                // the same way a SIGTERM to the old wrapper did.
                STOP_REQUESTED.store(true, Ordering::Relaxed);
            }
            // The desktop session holds no game: release it immediately.
            table.desktop_wish = None;
            stop_desktop_session(hub, &table.sessions);
            *shutdown = true;
            respond(clients, index, Response::Bye);
        }
    }
}

fn start_session(
    launch: LaunchRequest,
    session_id: u64,
    hub: HubHandle,
    desktop: bool,
) -> Result<SessionHandle, String> {
    if launch.command.is_empty() && !desktop {
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
        trace: launch.trace,
        motion_port: launch.motion_port,
        vdf_import: None,
        list: false,
        probe_sensors: false,
        steam_app_id: launch.steam_app_id.clone(),
        command: launch.command.clone(),
        daemon: false,
        no_daemon: false,
        session_id,
        hub: Some(hub),
        env: Some(launch.env.clone()),
        working_dir: launch.working_dir.clone(),
        events: Some(event_tx),
    };
    std::thread::Builder::new()
        .name("ira-session".to_string())
        .spawn(move || {
            // A panicked session must still report done: without it the
            // handle never leaves `sessions`, the daemon never idles out,
            // and ira-input outlives Ira itself.
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_session(arguments)))
                    .unwrap_or_else(|panic| {
                        Err(format!("session thread panicked: {}", panic_text(&panic)))
                    });
            let _ = done_tx.send(result);
        })
        .map_err(|error| format!("spawn session thread: {error}"))?;
    Ok(SessionHandle {
        id: session_id,
        tag: launch.tag,
        events: event_rx,
        done: done_rx,
        desktop,
        desktop_profile: if desktop { launch.profile } else { None },
    })
}

/// Best-effort text for a caught panic payload, for the done report.
fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = panic.downcast_ref::<String>() {
        return text.clone();
    }
    "unknown panic payload".to_string()
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
fn stop_running_sessions(sessions: &mut Vec<SessionHandle>) {
    if sessions.is_empty() {
        return;
    }
    STOP_REQUESTED.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + SESSION_STOP_PATIENCE;
    while !sessions.is_empty() && Instant::now() < deadline {
        let mut finished = false;
        for session in sessions.iter_mut() {
            if session.done.try_recv().is_ok() {
                finished = true;
            }
        }
        if finished {
            sessions.retain(|session| session.done.try_recv().is_err());
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !sessions.is_empty() {
        eprintln!("ira-input: sessions did not acknowledge shutdown in time");
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
            trace: false,
            motion_port: Some(0),
            steam_app_id: None,
            tag: None,
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
    fn test_panic_text_reads_string_payloads() {
        let borrowed: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_text(&borrowed), "boom");
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("bang"));
        assert_eq!(panic_text(&owned), "bang");
        let opaque: Box<dyn std::any::Any + Send> = Box::new(7);
        assert_eq!(panic_text(&opaque), "unknown panic payload");
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

        // Rewrite the profile with a different controller kind only after
        // the previous reload was observed: the monitor coalesces writes
        // that land while a reload is pending, and the first loop pass can
        // lag seconds behind the launch when a switch-protocol pad must be
        // probed first. Fixed sleeps used to race that probe and miss the
        // swaps entirely. The stage cell hands the first write a fallback
        // in case the watch-establishment reload never fires.
        let rewrites = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let rewrites_for_timer = rewrites.clone();
        let timer_profile = profile_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            if rewrites_for_timer
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                write_profile(&timer_profile, crate::VirtualGamepadBackend::DirectInput);
            }
        });

        // Three seconds comfortably outlasts both swaps (the timer fires
        // at 700ms and each coalesced reload follows within a couple of
        // pump intervals) while keeping the test off the old five-second
        // session that dominated the whole suite's runtime.
        let reloads = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let code = client
            .launch_and_wait(
                session_request(
                    vec!["sleep".into(), "3".into()],
                    Some(profile_path.display().to_string()),
                ),
                {
                    let rewrites = rewrites.clone();
                    let reloads = reloads.clone();
                    let profile_path = profile_path.clone();
                    move |event| {
                        if matches!(event, Event::ProfileReloaded { .. }) {
                            reloads.fetch_add(1, Ordering::SeqCst);
                            if rewrites
                                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                                .is_ok()
                            {
                                write_profile(
                                    &profile_path,
                                    crate::VirtualGamepadBackend::DirectInput,
                                );
                            } else if rewrites
                                .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                                .is_ok()
                            {
                                write_profile(
                                    &profile_path,
                                    crate::VirtualGamepadBackend::XInput,
                                );
                            }
                        }
                    }
                },
            )
            .unwrap();
        assert_eq!(code, 0);
        let reloads = reloads.load(Ordering::SeqCst);
        assert!(
            reloads >= 2,
            "controller-kind changes must hot-reload, not refuse (saw {reloads})"
        );

        shutdown_and_join(&mut client, server);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_daemon_routes_layout_switch_to_the_tagged_session() {
        let dir = temp_test_dir("switch");
        let path = dir.join("test.sock");
        let first_profile = dir.join("first.json");
        write_profile(&first_profile, crate::VirtualGamepadBackend::XInput);
        let second_profile = dir.join("second.json");
        write_profile(&second_profile, crate::VirtualGamepadBackend::DirectInput);
        let server = std::thread::spawn({
            let path = path.clone();
            move || run_daemon_on(&path)
        });
        let mut client = wait_for_server(&path);

        let mut request = session_request(
            vec!["sleep".into(), "1".into()],
            Some(first_profile.display().to_string()),
        );
        request.tag = Some(77);
        let watcher = std::thread::spawn(move || {
            let mut reloads = 0;
            let code = client
                .launch_and_wait(request, |event| {
                    if matches!(event, Event::ProfileReloaded { .. }) {
                        reloads += 1;
                    }
                })
                .expect("session must end cleanly");
            (code, reloads)
        });

        // The switch must land on a live session, so poll the status until
        // the session is active instead of guessing a fixed delay; an
        // unknown tag must be refused either way.
        let mut switcher = wait_for_server(&path);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !switcher
            .status()
            .map(|status| status.session_active)
            .unwrap_or(false)
        {
            assert!(
                Instant::now() < deadline,
                "the session never became active"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let unknown = switcher.request(Request::ReloadProfile {
            tag: 41,
            profile: second_profile.display().to_string(),
        });
        assert!(
            matches!(unknown, Ok(Response::Error { .. })),
            "an unknown tag must be refused"
        );
        match switcher
            .request(Request::ReloadProfile {
                tag: 77,
                profile: second_profile.display().to_string(),
            })
            .unwrap()
        {
            Response::Reloaded => {}
            other => panic!("expected reloaded, got {other:?}"),
        }
        drop(switcher);
        let (code, reloads) = watcher.join().unwrap();
        assert_eq!(code, 0);
        // Exactly one switch was requested, and the session applied it.
        assert!(reloads >= 1, "the layout switch must reach the session");

        let mut final_client = wait_for_server(&path);
        shutdown_and_join(&mut final_client, server);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_daemon_runs_two_sessions_concurrently() {
        let dir = temp_test_dir("multi");
        let path = dir.join("test.sock");
        let server = std::thread::spawn({
            let path = path.clone();
            move || run_daemon_on(&path)
        });
        let mut first = wait_for_server(&path);
        let mut second = DaemonClient::connect(&path).unwrap();

        // Two games at once: both sessions must be accepted and both must
        // run to completion — the old daemon refused the second launch.
        let session_a = first
            .begin_launch(session_request(vec!["sleep".into(), "0.4".into()], None))
            .unwrap();
        let session_b = second
            .begin_launch(session_request(vec!["sleep".into(), "0.2".into()], None))
            .unwrap();
        assert_ne!(session_a, session_b, "sessions need distinct ids");

        let (code_a, code_b) = std::thread::scope(|scope| {
            let a = scope.spawn({
                let first = &mut first;
                move || {
                    first
                        .wait_session(|_| {})
                        .expect("first session must end cleanly")
                }
            });
            let b = scope.spawn({
                let second = &mut second;
                move || {
                    second
                        .wait_session(|_| {})
                        .expect("second session must end cleanly")
                }
            });
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_eq!(code_a, 0);
        assert_eq!(code_b, 0);

        let status = first.status().unwrap();
        assert!(!status.session_active, "both sessions must be over");

        shutdown_and_join(&mut first, server);
        std::fs::remove_dir_all(&dir).ok();
    }
}
