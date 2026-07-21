use std::time::Duration;

use ira_models::{AppMessage, AppSender};
use crate::watcher_util::DebouncedFileWatcher;

use super::paths::persistent_settings_path;

/// Watches persistent_settings.dat for changes (playtime/last-played updates).
/// Uses inotify — zero CPU when idle.
pub struct Rpcs3Watcher {
    _watcher: DebouncedFileWatcher,
}

impl Rpcs3Watcher {
    pub fn new(sender: AppSender) -> Result<Self, String> {
        let path = persistent_settings_path();
        let watcher = DebouncedFileWatcher::new(
            &path,
            "persistent_settings.dat",
            Duration::from_secs(2),
            move || {
                let _ = sender.send(AppMessage::Rpcs3PlaytimeChanged);
            },
        )?;
        Ok(Rpcs3Watcher { _watcher: watcher })
    }
}
