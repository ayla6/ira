use std::time::Duration;

use crate::AppSender;
use crate::models::AppMessage;
use crate::platforms::lutris::{lutris_db_path, load_lutris_playtime};
use crate::platforms::watcher_util::DebouncedFileWatcher;

/// Watches `pga.db` for changes and sends `LutrisDataChanged` messages with
/// fresh `(lutris_id, playtime, lastplayed)` tuples. Uses inotify — zero CPU
/// when the file is not being written to.
pub struct LutrisWatcher {
    _watcher: DebouncedFileWatcher,
}

impl LutrisWatcher {
    pub fn new(sender: AppSender) -> Result<Self, String> {
        let db_path = lutris_db_path();
        let watcher = DebouncedFileWatcher::new(
            &db_path,
            "pga.db",
            Duration::from_secs(2),
            move || {
                match load_lutris_playtime() {
                    Ok(data) => {
                        let _ = sender.send(AppMessage::LutrisDataChanged(data));
                    }
                    Err(e) => {
                        eprintln!("Lutris watcher re-read failed: {}", e);
                    }
                }
            },
        )?;
        Ok(LutrisWatcher { _watcher: watcher })
    }
}
