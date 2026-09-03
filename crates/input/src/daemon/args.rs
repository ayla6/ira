use std::path::PathBuf;

use super::SessionEvent;

pub struct Arguments {
    pub device: Option<PathBuf>,
    pub profile: Option<PathBuf>,
    pub calibration: Option<PathBuf>,
    pub pause_unfocused: bool,
    pub motion_port: Option<u16>,
    pub vdf_import: Option<(PathBuf, PathBuf)>,
    pub list: bool,
    pub probe_sensors: bool,
    pub steam_app_id: Option<String>,
    pub trace: bool,
    pub command: Vec<String>,
    /// Daemon-only fields: they never come from the command line, only from
    /// a server assembling a session out of a Launch request.
    pub daemon: bool,
    pub no_daemon: bool,
    /// Identity used for hub routing; the server assigns one per session.
    pub(crate) session_id: u64,
    /// The shared pad hub. A daemon hands down its hub; a standalone
    /// (`--no-daemon`) session spawns a private one in setup.
    pub(crate) hub: Option<super::hub::HubHandle>,
    pub(crate) env: Option<Vec<(String, String)>>,
    pub(crate) working_dir: Option<String>,
    pub(crate) events: Option<std::sync::mpsc::Sender<SessionEvent>>,
}

pub fn parse_arguments() -> Result<Arguments, String> {
    let mut arguments = Arguments {
        device: None,
        profile: None,
        calibration: None,
        pause_unfocused: false,
        motion_port: None,
        vdf_import: None,
        list: false,
        probe_sensors: false,
        steam_app_id: None,
        trace: false,
        command: Vec::new(),
        daemon: false,
        no_daemon: false,
        session_id: 0,
        hub: None,
        env: None,
        working_dir: None,
        events: None,
    };
    let mut values = std::env::args().skip(1);
    while let Some(argument) = values.next() {
        if argument == "--" {
            arguments.command.extend(values);
            break;
        }
        match argument.as_str() {
            "--device" => {
                arguments.device = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--device requires a path".to_string())?,
                ));
            }
            "--profile" => {
                arguments.profile = Some(PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--profile requires a path".to_string())?,
                ));
            }
            "--calibration" => {
                arguments.calibration =
                    Some(PathBuf::from(values.next().ok_or_else(|| {
                        "--calibration requires a path".to_string()
                    })?));
            }
            "--steam-app-id" => {
                arguments.steam_app_id = Some(
                    values
                        .next()
                        .ok_or_else(|| "--steam-app-id requires an ID".to_string())?,
                );
            }
            "--pause-unfocused" => arguments.pause_unfocused = true,
            "--motion-port" => {
                let raw = values
                    .next()
                    .ok_or_else(|| "--motion-port requires a port number".to_string())?;
                arguments.motion_port = Some(
                    raw.parse()
                        .map_err(|_| format!("--motion-port expects a number, got {raw}"))?,
                );
            }
            "--vdf-import" => {
                let input = PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--vdf-import requires an input .vdf path".to_string())?,
                );
                let output = PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--vdf-import requires an output .json path".to_string())?,
                );
                arguments.vdf_import = Some((input, output));
            }
            "--list" => arguments.list = true,
            "--probe-sensors" => arguments.probe_sensors = true,
            "--trace" => arguments.trace = true,
            "--daemon" => arguments.daemon = true,
            "--no-daemon" => arguments.no_daemon = true,
            "--help" | "-h" => {
                println!(
                    "usage: ira-input [--vdf-import IN.vdf OUT.json] | --list | --daemon | --no-daemon | \
                     [--device PATH] [--profile PATH] [--steam-app-id ID] [--trace] -- COMMAND"
                );
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument {unknown}")),
        }
    }
    Ok(arguments)
}
