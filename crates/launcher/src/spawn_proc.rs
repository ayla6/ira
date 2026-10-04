//! Process spawning for games: subreaper setup, file-backed stdio, rich
//! spawn diagnostics, and untracked detached spawns.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::logs::{clear_game_log, get_game_log, tail_file_to_log};

const PR_SET_CHILD_SUBREAPER: i32 = 36;

fn set_subreaper() {
    let ret = unsafe { libc::prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if ret != 0 {
        eprintln!(
            "Warning: failed to set child subreaper (prctl returned {})",
            ret
        );
    }
}

/// Spawns the game process with piped stdout/stderr for in-memory logging.
pub fn spawn_game(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_path: Option<&str>,
) -> Result<Child, String> {
    spawn_game_with(command, env, cwd, log_path)
}

/// The spawn behind both `spawn_game` and `spawn_detached`.
///
/// Games must survive Ira: no PR_SET_PDEATHSIG (it fires when the spawning
/// thread exits, killing the game when Ira quits), own process group (so
/// terminal Ctrl-C never reaches them), and file-backed stdio — piped
/// output SIGPIPEs the game the moment our reader threads die with us.
/// `log_path` truncates and receives stdout+stderr (O_APPEND interleaved);
/// `None` sends both to null. Callers tail the file for the live log view.
fn spawn_game_with(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_path: Option<&str>,
) -> Result<Child, String> {
    if command.is_empty() {
        return Err("Failed to spawn game process: empty command".to_string());
    }
    set_subreaper();

    let mut cmd = Command::new(&command[0]);
    for arg in &command[1..] {
        cmd.arg(arg);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // Clear the parent env entirely so filtered vars (CARGO_*, RUSTUP_*, etc.)
    // don't leak into the child. Our env list is the complete env for the child.
    cmd.env_clear();
    for (key, val) in env {
        cmd.env(key, val);
    }

    cmd.stdout(game_stdio(log_path));
    cmd.stderr(game_stdio(log_path));
    cmd.process_group(0);

    match cmd.spawn() {
        Ok(child) => Ok(child),
        Err(error) => {
            let diagnostic = format_spawn_error(command, env, cwd, &error);
            eprintln!("launch: {diagnostic}");
            Err(diagnostic)
        }
    }
}

/// Stdio for spawned games: append to `log_path`, or null when absent.
/// Never pipes: our reader threads die with Ira, and a game writing to a
/// dead pipe eats SIGPIPE the moment we quit (or freezes on a full one).
fn game_stdio(log_path: Option<&str>) -> Stdio {
    let Some(path) = log_path else {
        return Stdio::null();
    };
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, "");
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

fn format_spawn_error(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    error: &std::io::Error,
) -> String {
    let program = command.first().map(String::as_str).unwrap_or("");
    let resolved_program = resolve_program(program, env);
    let cwd_info = cwd
        .map(|path| {
            format!(
                "{} (exists={}, directory={})",
                path,
                Path::new(path).exists(),
                Path::new(path).is_dir()
            )
        })
        .unwrap_or_else(|| "<inherit>".to_string());
    let path = env_value(env, "PATH").unwrap_or_else(|| "<unset>".to_string());
    let prefix = env_value(env, "WINEPREFIX").unwrap_or_else(|| "<unset>".to_string());
    let wine = env_value(env, "WINE").unwrap_or_else(|| "<unset>".to_string());

    format!(
        "Failed to spawn game process: {error} (kind={:?}, raw_os_error={:?}); command={command:?}; program={program:?}; resolved_program={:?}; program_exists={}; cwd={cwd_info}; PATH={path:?}; WINE={wine:?}; WINEPREFIX={prefix:?}",
        error.kind(),
        error.raw_os_error(),
        resolved_program.as_ref().map(|p| p.display().to_string()),
        resolved_program.as_ref().is_some_and(|p| p.is_file()),
    )
}

fn env_value(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

fn resolve_program(program: &str, env: &[(String, String)]) -> Option<std::path::PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') {
        return Some(path.to_path_buf());
    }
    let search_path = env_value(env, "PATH").or_else(|| std::env::var("PATH").ok())?;
    std::env::split_paths(&search_path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Spawns a process that is not tracked as a play session: nothing is
/// recorded, no messages are sent, and the child is left running after Ira
/// exits (aside from the subreaper). Its output is streamed into the log
/// buffer for `log_key` so the in-app log viewer still shows it.
pub fn spawn_detached(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    log_key: i64,
    header: String,
    desktop_hold: Option<ira_input_ipc::DaemonClient>,
) -> Result<(), String> {
    // Detached output lives in temp (no save dir is threaded here) and is
    // tailed into the buffer; the game must survive Ira either way.
    let log_path = std::env::temp_dir()
        .join(format!("ira-detached-{log_key}.log"))
        .to_string_lossy()
        .into_owned();
    let mut child = spawn_game_with(command, env, cwd, Some(&log_path))?;
    clear_game_log(log_key);
    let log = get_game_log(log_key);
    log.lock().unwrap().push(header);

    let done = Arc::new(AtomicBool::new(false));
    tail_file_to_log(log_path, log.clone(), done.clone());

    let exit_log = log.clone();
    let tail_done = done.clone();
    std::thread::spawn(move || {
        // Held until the process exits: a detached spawn opened with
        // input remapping disabled must not be remapped underneath by
        // the daemon's idle desktop session. Dropping the client when
        // this thread ends releases the hold, however the process ends.
        let _desktop_hold = desktop_hold;
        match child.wait() {
            Ok(status) => {
                exit_log
                    .lock()
                    .unwrap()
                    .push(format!("Process exited with status {status}"));
            }
            Err(error) => {
                eprintln!("launch: failed to read detached process status: {error}");
            }
        }
        tail_done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_spawn_error;

    #[test]
    fn test_format_spawn_error_reports_missing_program_and_cwd() {
        let error = std::io::Error::from_raw_os_error(2);
        let message = format_spawn_error(
            &["/missing/game".to_string()],
            &[("PATH".to_string(), "/usr/bin".to_string())],
            Some("/missing/cwd"),
            &error,
        );

        assert!(message.contains("raw_os_error=Some(2)"));
        assert!(message.contains("program_exists=false"));
        assert!(message.contains("cwd=/missing/cwd (exists=false, directory=false)"));
        assert!(message.contains("WINEPREFIX=\"<unset>\""));
    }
}
