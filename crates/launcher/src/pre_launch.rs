//! Pre-launch scripts: run synchronously via `sh -c` in the game's working
//! directory. Aborts the launch on non-zero exit (unless fire-and-forget).

/// Run a pre-launch command synchronously via `sh -c`.
/// Uses the game's working directory. Aborts launch on non-zero exit.
pub(crate) fn run_pre_launch(
    cmd: &str,
    cwd: Option<&str>,
    save_dir: &str,
    game_id: i64,
    wait: bool,
) -> Result<(), String> {
    let log_path = super::wrapper::game_log_path(save_dir, game_id);
    let mut child = std::process::Command::new("sh");
    child.arg("-c").arg(cmd);
    if let Some(dir) = cwd {
        child.current_dir(dir);
    }
    if let Some(parent) = std::path::Path::new(&log_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create log directory {}: {}", parent.display(), e);
        }
    }
    use std::process::Stdio;
    child.stdout(Stdio::piped()).stderr(Stdio::piped());
    let spawned = child
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to run pre-launch command {cmd:?} with cwd {:?}: {} (kind={:?}, raw_os_error={:?})",
                cwd,
                e,
                e.kind(),
                e.raw_os_error()
            )
        })?;
    if !wait {
        // Fire-and-forget: a waiter thread reaps the child and records its
        // output, so a long-running script doesn't delay the game launch.
        std::thread::spawn(move || match spawned.wait_with_output() {
            Ok(out) => {
                write_pre_launch_log(&log_path, &out);
                if !out.status.success() {
                    eprintln!(
                        "Pre-launch command failed (exit code {:?})",
                        out.status.code()
                    );
                }
            }
            Err(e) => eprintln!("Failed to read pre-launch command status: {e}"),
        });
        return Ok(());
    }
    match spawned.wait_with_output() {
        Ok(out) => {
            write_pre_launch_log(&log_path, &out);
            if out.status.success() {
                return Ok(());
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{}\n{}", stdout.trim(), stderr.trim());
            let snippet = if combined.trim().is_empty() {
                "no output".to_string()
            } else {
                combined
                    .trim()
                    .lines()
                    .take(10)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Err(format!(
                "Pre-launch command failed (exit code {:?}):\n{}",
                out.status.code(),
                snippet
            ))
        }
        Err(e) => Err(format!(
            "Failed to run pre-launch command {cmd:?} with cwd {:?}: {} (kind={:?}, raw_os_error={:?})",
            cwd,
            e,
            e.kind(),
            e.raw_os_error()
        )),
    }
}

fn write_pre_launch_log(log_path: &str, out: &std::process::Output) {
    if let Ok(mut f) = std::fs::File::create(log_path) {
        use std::io::Write;
        let _ = f.write_all(&out.stdout);
        let _ = f.write_all(&out.stderr);
    }
}
