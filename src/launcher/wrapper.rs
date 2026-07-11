use crate::AppSender;
use crate::AppMessage;
use crate::db::{DbConn, record_session};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const PR_SET_CHILD_SUBREAPER: i32 = 36;

fn set_subreaper() {
    unsafe {
        libc::prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    }
}

pub fn spawn_game(
    command: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
) -> Result<Child, String> {
    set_subreaper();

    let mut cmd = Command::new(&command[0]);
    for arg in &command[1..] {
        cmd.arg(arg);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (key, val) in env {
        cmd.env(key, val);
    }
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    cmd.spawn().map_err(|e| format!("Failed to spawn game process: {}", e))
}

pub fn monitor_process(
    mut child: Child,
    _child_pid: i32,
    sender: &AppSender,
    lutris_id: i64,
    started_at: i64,
    db: DbConn,
    game_id: i64,
    running_games: Arc<Mutex<HashMap<i64, i32>>>,
) {
    let mut exited = false;
    while !exited {
        std::thread::sleep(Duration::from_secs(2));

        reap_zombies();

        match child.try_wait() {
            Ok(Some(_status)) => {
                exited = true;
            }
            Ok(None) => {}
            Err(_) => {
                exited = true;
            }
        }
    }

    let _ = reap_zombies_once();

    let ended_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let duration = ended_at - started_at;

    if duration < 5 {
        eprintln!("Game {} exited after {}s — possible crash", lutris_id, duration);
    }

    if let Err(e) = record_session(&db, game_id, started_at, ended_at) {
        eprintln!("Failed to record play session: {}", e);
    }

    {
        let mut map = running_games.lock().unwrap();
        map.remove(&lutris_id);
    }

    let _ = sender.send(AppMessage::SessionRecorded {
        game_id,
        duration_seconds: duration,
        started_at,
        ended_at,
    });
    let _ = sender.send(AppMessage::GameStopped(lutris_id));
}

fn reap_zombies() {
    loop {
        let ret = unsafe {
            let mut status: i32 = 0;
            libc::waitpid(-1, &mut status as *mut i32, libc::WNOHANG)
        };
        if ret <= 0 {
            break;
        }
    }
}

fn reap_zombies_once() -> Option<i32> {
    unsafe {
        let mut status: i32 = 0;
        let ret = libc::waitpid(-1, &mut status as *mut i32, libc::WNOHANG);
        if ret > 0 {
            Some(status)
        } else {
            None
        }
    }
}
