//! Launching games through the resident input daemon. The daemon owns the
//! physical controllers and virtual devices across sessions, so exactly one
//! input process exists no matter how many games launch. When the daemon
//! cannot be reached or refuses, every caller falls back to the classic
//! wrapper spawn — the game itself must always start.

use std::time::{Duration, Instant};

use ira_input_ipc::{
    socket_path, DaemonStatus, LaunchRequest, Request, Response, PROTOCOL_VERSION,
};
pub use ira_input_ipc::DaemonClient;
use ira_models::ControllerInputMode;

/// The input-related slice of a launch config, resolved for the daemon.
pub struct InputLaunch {
    pub mode: Option<ControllerInputMode>,
    pub profile: Option<String>,
    pub calibration: Option<String>,
    pub pause_unfocused: bool,
}

/// Tries to hand the game to the input daemon. `Ok(client)` means the daemon
/// accepted the session and the caller should monitor it instead of spawning
/// the command; `Err` means fall back to the wrapper path. `tag` (Ira's game
/// id) lets later requests — a mid-game layout switch — address the session.
pub fn launch_via_daemon(
    command: &[String],
    env: &[(String, String)],
    working_dir: Option<&str>,
    input: &InputLaunch,
    tag: i64,
) -> Result<DaemonClient, String> {
    if !matches!(input.mode, Some(ControllerInputMode::Enabled)) {
        return Err("input remapping is disabled".to_string());
    }
    let binary = super::env_builder::input_binary_path()
        .ok_or_else(|| "ira-input binary was not found".to_string())?;
    // A daemon from an older build would misunderstand the protocol;
    // connect_daemon refuses it so the caller falls back instead of
    // launching something broken.
    let mut client = DaemonClient::connect_daemon(&binary)?;
    client.begin_launch(LaunchRequest {
        device: None,
        command: command.to_vec(),
        env: env.to_vec(),
        working_dir: working_dir.map(str::to_string),
        profile: input.profile.clone(),
        calibration: input.calibration.clone(),
        pause_unfocused: input.pause_unfocused,
        trace: false,
        motion_port: None,
        steam_app_id: None,
        tag: Some(tag),
    })?;
    Ok(client)
}

/// Connects to the input daemon for the app's long-lived desktop presence,
/// spawning one on demand. `Err` means no daemon is reachable and the idle
/// controller keeps its native behaviour.
pub fn desktop_client() -> Result<DaemonClient, String> {
    let binary = super::env_builder::input_binary_path()
        .ok_or_else(|| "ira-input binary was not found".to_string())?;
    DaemonClient::connect_daemon(&binary)
}

/// Updates the daemon's standing desktop-default wish: while the app is
/// open, an idle controller runs its default layout through the daemon
/// (`enabled`) instead of going silent under the hub's grab. The daemon
/// supersedes the wish for game launches and resumes it afterwards.
pub fn send_desktop_default(
    client: &mut DaemonClient,
    enabled: bool,
    profile: Option<&str>,
    calibration: Option<&str>,
) -> Result<(), String> {
    match client.request(Request::DesktopDefault {
        enabled,
        profile: profile.map(str::to_string),
        calibration: calibration.map(str::to_string),
        motion_port: None,
    })? {
        Response::Applied => Ok(()),
        Response::Error { message } => Err(message),
        _ => Err("unexpected response to the desktop-default wish".to_string()),
    }
}

/// Suspends the daemon's desktop-default controller behaviour for the
/// lifetime of the returned client: a game that bypasses the daemon (input
/// remapping disabled or inherited-off) must not be remapped underneath by
/// the idle desktop session. Dropping the client releases the hold, so a
/// crash can never leave the controller suspended. `None` means no daemon
/// is running (the pad is already native) or one from an older build is;
/// either way the game launches untouched.
pub fn hold_desktop_for_game() -> Option<DaemonClient> {
    let mut client = DaemonClient::connect(&socket_path()).ok()?;
    let status = match client.status() {
        Ok(status) => status,
        Err(_) => return None,
    };
    if status.protocol_version != PROTOCOL_VERSION {
        eprintln!(
            "launch: daemon speaks protocol {}, this build speaks {PROTOCOL_VERSION}; \
             its desktop remap stays up for this game",
            status.protocol_version
        );
        return None;
    }
    match client.request(Request::HoldDesktop { hold: true }) {
        Ok(Response::Applied) => {}
        Ok(Response::Error { message }) => {
            eprintln!("launch: desktop hold refused: {message}");
            return None;
        }
        _ => return None,
    }
    // The desktop session winds down asynchronously; give it a moment so
    // the game never sees its virtual pad, but spawn regardless — a stuck
    // daemon must not block the game.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match client.status() {
            Ok(DaemonStatus { desktop_active, .. }) if !desktop_active => break,
            Ok(_) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    client.drain_in_background();
    Some(client)
}

/// Tells the daemon running `tag`'s game session to switch to the layout at
/// `profile`: the session reloads it as if the file had been edited in
/// place. Unlike a launch this never starts a daemon — no running session
/// (or a stale one) simply surfaces as `Err` for the caller to log, and the
/// newly picked layout applies at the next launch instead.
pub fn reload_session_profile(tag: i64, profile: &str) -> Result<(), String> {
    let mut client = DaemonClient::connect(&ira_input_ipc::socket_path())?;
    let status = client.status()?;
    if status.protocol_version != ira_input_ipc::PROTOCOL_VERSION {
        return Err(format!(
            "daemon speaks protocol {}, this build speaks {}",
            status.protocol_version,
            ira_input_ipc::PROTOCOL_VERSION
        ));
    }
    match client.request(Request::ReloadProfile {
        tag,
        profile: profile.to_string(),
    })? {
        Response::Reloaded => Ok(()),
        Response::Error { message } => Err(message),
        _ => Err("unexpected response to the layout switch".to_string()),
    }
}

/// Asks a running input daemon to retire. Best-effort: connecting never
/// spawns one, and a dead socket is simply "nothing to shut down". Without
/// this the daemon only leaves on its own idle timer, and a session that
/// never fully winds down would keep it alive indefinitely after Ira is
/// gone. An active game session makes the daemon refuse, which is correct:
/// closing Ira must not kill a game that is still playing.
pub fn shutdown_daemon() {
    let Ok(mut client) = DaemonClient::connect(&ira_input_ipc::socket_path()) else {
        return;
    };
    match client.request(Request::Shutdown { stop_running: false }) {
        Ok(Response::Bye) => {}
        Ok(Response::Error { message }) => {
            eprintln!("ira-input: daemon stays up: {message}");
        }
        Ok(_) => {}
        Err(error) => eprintln!("ira-input: shutdown request failed: {error}"),
    }
}
