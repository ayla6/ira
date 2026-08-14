use crate::Game;
use std::sync::mpsc;

pub enum AppMessage {
    EnrichedGame(Game),
    NewGame(Game),
    WatcherGameUpdated(Game),
    AddGameError(String),
    GameStopped(i64, Option<i64>),
    GameStarted(i64, Option<i64>),
    /// Fired by the ShadPS4Watcher when play_time.txt changes.
    ShadPS4PlaytimeChanged,
    /// Fired by the Rpcs3Watcher when persistent_settings.dat changes.
    Rpcs3PlaytimeChanged,
    /// Initial game list loaded in the background.
    GamesLoaded(Vec<Game>),
    /// Background game-list discovery progress.
    GamesLoadProgress {
        status: String,
        completed: usize,
        total: usize,
    },
    /// Rebuild the game list after a source-settings change or manual rescan.
    ReloadGames,
    /// SGDB assets downloaded for a game.
    SessionRecorded {
        game_id: i64,
        variant_id: Option<i64>,
        duration_seconds: i64,
        started_at: i64,
        ended_at: i64,
    },
    SgdbAssetsDownloaded {
        db_id: i64,
        sgdb_id: String,
        icon: String,
        hero: String,
        grid: String,
        logo: String,
        header: String,
    },
    /// User selected a different variant on the base game's play button.
    /// Reloads the game page with the variant's hero + logo.
    VariantSelected(i64, Option<i64>),
    /// Variants were added/removed/edited in the edit dialog.
    /// Rebuilds variant pseudo-game entries for this game.
    VariantsChanged(i64),
}

pub struct AppSender {
    tx: mpsc::Sender<AppMessage>,
    fd: std::os::unix::io::RawFd,
}

impl Clone for AppSender {
    fn clone(&self) -> Self {
        let new_fd = unsafe { libc::dup(self.fd) };
        Self {
            tx: self.tx.clone(),
            fd: new_fd,
        }
    }
}

impl AppSender {
    pub fn new(tx: mpsc::Sender<AppMessage>, fd: std::os::unix::io::RawFd) -> Self {
        Self { tx, fd }
    }

    pub fn send(&self, msg: AppMessage) -> Result<(), String> {
        self.tx.send(msg).map_err(|e| e.to_string())?;
        let byte = [1u8; 1];
        unsafe {
            libc::write(self.fd, byte.as_ptr() as *const _, 1);
        }
        Ok(())
    }
}

impl Drop for AppSender {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}
