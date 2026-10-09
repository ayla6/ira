//! In-memory game log buffers plus the file-tail plumbing that feeds them.
//! Games write stdout/stderr to a file (never a pipe — a dead pipe SIGPIPEs
//! the game when Ira exits); tails stream the file into the shared buffer
//! for the in-app log viewer.

use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub type GameLog = Arc<Mutex<Vec<String>>>;

static GAME_LOGS: OnceLock<Mutex<std::collections::HashMap<i64, GameLog>>> = OnceLock::new();

fn game_logs() -> &'static Mutex<std::collections::HashMap<i64, GameLog>> {
    GAME_LOGS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Creates a new empty buffer if none exists.
pub fn get_game_log(game_id: i64) -> GameLog {
    let mut logs = game_logs().lock().unwrap();
    logs.entry(game_id)
        .or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Clears the log buffer for a game (called at launch start).
pub fn clear_game_log(game_id: i64) {
    let logs = game_logs().lock().unwrap();
    if let Some(log) = logs.get(&game_id) {
        log.lock().unwrap().clear();
    }
}

pub fn game_log_path(save_dir: &str, game_id: i64) -> String {
    std::path::Path::new(save_dir)
        .join("logs")
        .join(format!("{}.log", game_id))
        .to_string_lossy()
        .into_owned()
}

/// Renders a command for the log the way a shell would need it typed:
/// arguments a shell would misread (spaces, quotes, globs, …) are
/// single-quoted, so a path with spaces reads as one argument and the line
/// can be pasted into a terminal. The spawn itself is argv-based and never
/// needs this; it's display-only.
fn display_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            let plain = !arg.is_empty()
                && arg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_/.:=,+@%~".contains(&b));
            if plain {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first lines of every game log: the command being run and the launch
/// environment, minus cargo's own noise.
pub(crate) fn log_launch_header(
    command: &[String],
    env: &[(String, String)],
    log_buf: &GameLog,
    started_message: &str,
) {
    let mut log = log_buf.lock().unwrap();
    log.push(format!(
        "{} from {}",
        started_message,
        display_command(command)
    ));
    let mut sorted_env = env.to_vec();
    sorted_env.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in &sorted_env {
        if k.starts_with("CARGO_") || k.starts_with("RUSTUP_") || k.starts_with("RUST_") {
            continue;
        }
        log.push(format!("DEBUG: {}={}", k, v));
    }
}

fn push_lines(log: &GameLog, pending: &mut String, chunk: &[u8]) {
    pending.push_str(&String::from_utf8_lossy(chunk));
    while let Some(pos) = pending.find('\n') {
        let line = pending[..pos].trim_end_matches('\r').to_string();
        log.lock().unwrap().push(line);
        *pending = pending[pos + 1..].to_string();
    }
}

/// Follows a game log FILE into the shared buffer (replaces pipe capture:
/// pipes SIGPIPE the game when Ira exits, files don't). Stops after `done`
/// is set and the file goes quiet for ~1s so exit-flushes still land.
/// Reopens per poll, so truncation/rotation just restarts the offset.
pub(crate) fn tail_file_to_log(path: String, log: GameLog, done: Arc<std::sync::atomic::AtomicBool>) {
    use std::io::{Seek, SeekFrom};
    std::thread::spawn(move || {
        let mut offset = 0u64;
        let mut pending = String::new();
        let mut quiet_polls = 0u32;
        loop {
            let mut grown = false;
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() < offset {
                    offset = 0;
                }
                if let Ok(mut f) = std::fs::File::open(&path) {
                    if f.seek(SeekFrom::Start(offset)).is_ok() {
                        let mut buf = [0u8; 8192];
                        loop {
                            match f.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    grown = true;
                                    push_lines(&log, &mut pending, &buf[..n]);
                                }
                                Err(_) => break,
                            }
                        }
                        if let Ok(pos) = f.stream_position() {
                            offset = pos;
                        }
                    }
                }
            }
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                if !grown {
                    quiet_polls += 1;
                    if quiet_polls >= 4 {
                        if !pending.is_empty() {
                            log.lock().unwrap().push(std::mem::take(&mut pending));
                        }
                        break;
                    }
                } else {
                    quiet_polls = 0;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

/// Drains one piped output stream of a spawned process into the shared game
/// log buffer, one line at a time. Runs on its own thread so both pipes can
/// be read live without deadlocking on a full pipe buffer.
pub(crate) fn pipe_lines_to_log<R: Read + std::marker::Send + 'static>(pipe: Option<R>, log: GameLog) {
    std::thread::spawn(move || {
        let Some(mut reader) = pipe else { return };
        let mut buf = [0u8; 4096];
        let mut pending = String::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if !pending.is_empty() {
                        log.lock().unwrap().push(pending);
                    }
                    break;
                }
                Ok(n) => {
                    push_lines(&log, &mut pending, &buf[..n]);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::display_command;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn test_display_command_quotes_arguments_with_spaces() {
        assert_eq!(
            display_command(&[
                "azahar".to_string(),
                "/games/Zelda Tears of the Kingdom.3ds".to_string(),
            ]),
            "azahar '/games/Zelda Tears of the Kingdom.3ds'"
        );
    }

    #[test]
    fn test_display_command_leaves_plain_arguments_bare() {
        assert_eq!(
            display_command(&[
                "flatpak".to_string(),
                "run".to_string(),
                "--filesystem=/games:ro".to_string(),
                "org.azahar_emu.Azahar".to_string(),
            ]),
            "flatpak run --filesystem=/games:ro org.azahar_emu.Azahar"
        );
    }

    #[test]
    fn test_display_command_quotes_empty_and_embedded_quotes() {
        assert_eq!(display_command(&[String::new()]), "''");
        assert_eq!(
            display_command(&["/games/it's.iso".to_string()]),
            "'/games/it'\\''s.iso'"
        );
    }

    #[test]
    fn test_tail_file_to_log_collects_lines_and_stops() {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("ira-tail-test-{}.log", std::process::id()));
        std::fs::write(&path, "first\n").unwrap();
        let log = super::get_game_log(-97531);
        log.lock().unwrap().clear();
        let done = Arc::new(AtomicBool::new(false));
        super::tail_file_to_log(
            path.to_string_lossy().into_owned(),
            log.clone(),
            done.clone(),
        );
        // Append after the tail starts: it must pick up growth.
        std::thread::sleep(std::time::Duration::from_millis(400));
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, "second").unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        // Give the tail its ~1s quiet window to flush and stop.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let lines = log.lock().unwrap().clone();
        assert!(lines.contains(&"first".to_string()), "{lines:?}");
        assert!(lines.contains(&"second".to_string()), "{lines:?}");
        std::fs::remove_file(&path).ok();
    }
}
