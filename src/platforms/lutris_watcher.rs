use std::time::{Duration, Instant};

use crate::AppSender;
use crate::models::AppMessage;
use crate::platforms::lutris::{lutris_db_path, load_lutris_playtime};
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Watches `pga.db` for changes and sends `LutrisDataChanged` messages with
/// fresh `(lutris_id, playtime, lastplayed)` tuples. Uses inotify — zero CPU
/// when the file is not being written to.
pub struct LutrisWatcher {
    _watcher: std::sync::Arc<std::sync::Mutex<RecommendedWatcher>>,
}

impl LutrisWatcher {
    pub fn new(sender: AppSender) -> Result<Self, String> {
        let db_path = lutris_db_path();

        let (tx, rx) = std::sync::mpsc::channel();
        let nw = RecommendedWatcher::new(tx, NotifyConfig::default())
            .map_err(|e| e.to_string())?;

        let watcher = std::sync::Arc::new(std::sync::Mutex::new(nw));

        // Watch the directory containing pga.db (non-recursive). SQLite with
        // `delete` journal mode also creates/deletes a `-journal` file, so we
        // watch the directory and filter for `pga.db` writes.
        let watch_dir = db_path.parent().ok_or("no parent for pga.db")?;
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
                        let is_pga_write = event.paths.iter().any(|p| {
                            p.file_name().and_then(|n| n.to_str()) == Some("pga.db")
                        }) && matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );
                        if is_pga_write {
                            last_event = Instant::now();
                            pending = true;
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("Lutris watcher error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if pending && last_event.elapsed() >= DEBOUNCE {
                            pending = false;
                            match load_lutris_playtime() {
                                Ok(data) => {
                                    let _ = thread_sender
                                        .send(AppMessage::LutrisDataChanged(data));
                                }
                                Err(e) => {
                                    eprintln!("Lutris watcher re-read failed: {}", e);
                                }
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(LutrisWatcher { _watcher: watcher })
    }
}
