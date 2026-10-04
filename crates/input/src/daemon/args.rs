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
                arguments.device = Some(PathBuf::from(take_arg(
                    &mut values,
                    "--device requires a path",
                )?));
            }
            "--profile" => {
                arguments.profile = Some(PathBuf::from(take_arg(
                    &mut values,
                    "--profile requires a path",
                )?));
            }
            "--calibration" => {
                arguments.calibration = Some(PathBuf::from(take_arg(
                    &mut values,
                    "--calibration requires a path",
                )?));
            }
            "--steam-app-id" => {
                arguments.steam_app_id = Some(take_arg(
                    &mut values,
                    "--steam-app-id requires an ID",
                )?);
            }
            "--pause-unfocused" => arguments.pause_unfocused = true,
            "--motion-port" => {
                let raw = take_arg(&mut values, "--motion-port requires a port number")?;
                arguments.motion_port = Some(
                    raw.parse()
                        .map_err(|_| format!("--motion-port expects a number, got {raw}"))?,
                );
            }
            "--vdf-import" => {
                let input = PathBuf::from(take_arg(
                    &mut values,
                    "--vdf-import requires an input .vdf path",
                )?);
                let output = PathBuf::from(take_arg(
                    &mut values,
                    "--vdf-import requires an output .json path",
                )?);
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

/// The value following a flag that needs one, with the flag's own error
/// message when nothing follows.
fn take_arg(values: &mut impl Iterator<Item = String>, message: &str) -> Result<String, String> {
    values.next().ok_or_else(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::take_arg;

    #[test]
    fn test_take_arg_reads_the_next_value() {
        let mut values = vec!["path".to_string()].into_iter();
        assert_eq!(take_arg(&mut values, "missing").unwrap(), "path");
        assert_eq!(take_arg(&mut values, "needs a path"), Err("needs a path".to_string()));
    }
}
