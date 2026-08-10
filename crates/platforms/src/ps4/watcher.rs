use std::time::Duration;

use crate::watcher_util::DebouncedFileWatcher;
use ira_models::AppMessage;
use ira_models::AppSender;

use super::paths::play_time_path_for;

/// Watches play_time.txt for changes and sends a message.
/// Uses inotify — zero CPU when idle.
pub struct ShadPS4Watcher {
    _watcher: DebouncedFileWatcher,
}

impl ShadPS4Watcher {
    pub fn new(sender: AppSender, executable: &str) -> Result<Self, String> {
        let path = play_time_path_for(executable);
        let watcher =
            DebouncedFileWatcher::new(&path, "play_time.txt", Duration::from_secs(2), move || {
                let _ = sender.send(AppMessage::ShadPS4PlaytimeChanged);
            })?;
        Ok(ShadPS4Watcher { _watcher: watcher })
    }
}
