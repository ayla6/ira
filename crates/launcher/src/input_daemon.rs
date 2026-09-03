//! Launching games through the resident input daemon. The daemon owns the
//! physical controllers and virtual devices across sessions, so exactly one
//! input process exists no matter how many games launch. When the daemon
//! cannot be reached or refuses, every caller falls back to the classic
//! wrapper spawn — the game itself must always start.

use ira_input_ipc::{DaemonClient, LaunchRequest};
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
/// the command; `Err` means fall back to the wrapper path.
pub fn launch_via_daemon(
    command: &[String],
    env: &[(String, String)],
    working_dir: Option<&str>,
    input: &InputLaunch,
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
    })?;
    Ok(client)
}
