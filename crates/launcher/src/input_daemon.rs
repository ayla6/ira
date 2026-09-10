//! Launching games through the resident input daemon. The daemon owns the
//! physical controllers and virtual devices across sessions, so exactly one
//! input process exists no matter how many games launch. When the daemon
//! cannot be reached or refuses, every caller falls back to the classic
//! wrapper spawn — the game itself must always start.

use ira_input_ipc::{DaemonClient, LaunchRequest, Request, Response};
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
