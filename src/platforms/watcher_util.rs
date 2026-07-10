use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A debounced file watcher that fires `on_change` when a specific file changes.
/// Uses inotify — zero CPU when idle.
pub struct DebouncedFileWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
}

impl DebouncedFileWatcher {
    /// Watch `file_name` in the directory of `file_path`. When the file changes
    /// (create/modify/remove), wait `debounce` since the last event, then call `on_change`.
    pub fn new(
        file_path: &Path,
        file_name: &str,
        debounce: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let watch_dir = file_path.parent().ok_or("no parent directory")?;

        let (tx, rx) = std::sync::mpsc::channel();
        let nw = RecommendedWatcher::new(tx, NotifyConfig::default())
            .map_err(|e| e.to_string())?;
        let watcher = Arc::new(Mutex::new(nw));

        {
            let mut w = watcher.lock().unwrap();
            w.watch(watch_dir, RecursiveMode::NonRecursive)
                .map_err(|e| format!("watch {}: {}", watch_dir.display(), e))?;
        }

        let file_name = file_name.to_string();
        std::thread::spawn(move || {
            let mut last_event = std::time::Instant::now();
            let mut pending = false;

            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Ok(event)) => {
                        let is_target = event.paths.iter().any(|p| {
                            p.file_name().and_then(|n| n.to_str()) == Some(&file_name)
                        }) && matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );
                        if is_target {
                            last_event = std::time::Instant::now();
                            pending = true;
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("File watcher error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if pending && last_event.elapsed() >= debounce {
                            pending = false;
                            on_change();
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(DebouncedFileWatcher { _watcher: watcher })
    }
}
