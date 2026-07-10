use std::time::{Duration, Instant};

use crate::AppSender;
use crate::models::AppMessage;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::paths::play_time_path;

/// Watches play_time.txt for changes and sends a message.
/// Uses inotify — zero CPU when idle.
pub struct ShadPS4Watcher {
    _watcher: std::sync::Arc<std::sync::Mutex<RecommendedWatcher>>,
}

impl ShadPS4Watcher {
    pub fn new(sender: AppSender) -> Result<Self, String> {
        let path = play_time_path();
        let watch_dir = path.parent().ok_or("no parent for play_time.txt")?;

        let (tx, rx) = std::sync::mpsc::channel();
        let nw = RecommendedWatcher::new(tx, NotifyConfig::default())
            .map_err(|e| e.to_string())?;
        let watcher = std::sync::Arc::new(std::sync::Mutex::new(nw));

        {
            let mut w = watcher.lock().unwrap();
            w.watch(watch_dir, RecursiveMode::NonRecursive)
                .map_err(|e| format!("watch {}: {}", watch_dir.display(), e))?;
        }

        let thread_sender = sender;
        std::thread::spawn(move || {
            let mut last_event = Instant::now();
            let mut pending = false;
            const DEBOUNCE: Duration = Duration::from_secs(2);

            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Ok(event)) => {
                        let is_play_time = event.paths.iter().any(|p| {
                            p.file_name().and_then(|n| n.to_str()) == Some("play_time.txt")
                        }) && matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );
                        if is_play_time {
                            last_event = Instant::now();
                            pending = true;
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("shadPS4 watcher error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if pending && last_event.elapsed() >= DEBOUNCE {
                            pending = false;
                            let _ = thread_sender.send(AppMessage::ShadPS4PlaytimeChanged);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(ShadPS4Watcher { _watcher: watcher })
    }
}
