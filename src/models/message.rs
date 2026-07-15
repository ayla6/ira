use crate::models::Game;
use std::sync::mpsc;

pub enum AppMessage {
    EnrichedGame(Game),
    NewGame(Game),
    WatcherGameUpdated(Game),
    AddGameError(String),
    GameStopped(i64),
    GameStarted(i64),
    /// Fired by the LutrisWatcher when pga.db changes (debounced).
    /// Carries (lutris_id, playtime, lastplayed) for every Lutris game.
    LutrisDataChanged(Vec<(i64, f64, i64)>),
    /// Fired by the ShadPS4Watcher when play_time.txt changes.
    ShadPS4PlaytimeChanged,
    /// Initial game list loaded in the background.
    GamesLoaded(Vec<Game>),
    /// SGDB assets downloaded for a game.
    SessionRecorded {
        game_id: i64,
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
}

pub struct AppSender {
    tx: mpsc::Sender<AppMessage>,
    fd: std::os::unix::io::RawFd,
}

impl Clone for AppSender {
    fn clone(&self) -> Self {
        let new_fd = unsafe { libc::dup(self.fd) };
        Self { tx: self.tx.clone(), fd: new_fd }
    }
}

impl AppSender {
    pub fn new(tx: mpsc::Sender<AppMessage>, fd: std::os::unix::io::RawFd) -> Self {
        Self { tx, fd }
    }

    pub fn send(&self, msg: AppMessage) -> Result<(), mpsc::SendError<AppMessage>> {
        let result = self.tx.send(msg);
        if result.is_ok() {
            let byte = [1u8; 1];
            unsafe { libc::write(self.fd, byte.as_ptr() as *const _, 1); }
        }
        result
    }
}

impl Drop for AppSender {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}
