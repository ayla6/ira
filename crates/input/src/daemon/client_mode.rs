//! The default wrapper mode: hand the session to the resident daemon instead
//! of running it in this process. Only reached when the caller did not pass
//! `--no-daemon`; every failure here sends the caller back to the in-process
//! session, so a missing or broken daemon never blocks a launch.

use ira_input_ipc::{DaemonClient, Event, LaunchRequest};

use super::Arguments;

/// Runs the session through the daemon: the game becomes the daemon's child
/// and this process waits it out, mirroring the game's output so a terminal
/// launch looks like the in-process session. `Err` means the daemon was
/// unavailable and the caller should run the session locally.
pub fn run_via_daemon(arguments: &Arguments) -> Result<i32, String> {
    // The wrapper used to inherit this process's environment and working
    // directory; the daemon's child gets whatever the request carries, so
    // both travel along.
    let environment: Vec<(String, String)> = std::env::vars().collect();
    let working_dir = std::env::current_dir()
        .ok()
        .map(|dir| dir.to_string_lossy().into_owned());
    let binary = std::env::current_exe()
        .map(|exe| exe.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "ira-input".to_string());
    let mut client = DaemonClient::connect_daemon(&binary)?;
    client.begin_launch(LaunchRequest {
        device: arguments
            .device
            .as_deref()
            .map(|path| path.display().to_string()),
        command: arguments.command.clone(),
        env: environment,
        working_dir,
        profile: arguments
            .profile
            .as_deref()
            .map(|path| path.display().to_string()),
        calibration: arguments
            .calibration
            .as_deref()
            .map(|path| path.display().to_string()),
        pause_unfocused: arguments.pause_unfocused,
        trace: arguments.trace,
        motion_port: arguments.motion_port,
        steam_app_id: arguments.steam_app_id.clone(),
    })?;
    client.wait_session(|event| match event {
        Event::Output { line, .. } => eprintln!("{line}"),
        Event::SessionStarted { child_pid, .. } => {
            eprintln!("ira-input: session started (pid {child_pid})")
        }
        Event::Controller {
            connected, name, ..
        } => {
            eprintln!(
                "ira-input: controller {}",
                if connected {
                    format!("connected: {name}")
                } else {
                    format!("disconnected: {name}")
                }
            );
        }
        Event::ProfileReloaded { path, .. } => {
            eprintln!("ira-input: profile reloaded: {path}")
        }
        Event::SessionEnded { .. } => {}
    })
}
