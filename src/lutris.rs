use rusqlite::Connection;
use std::path::PathBuf;
use crate::AppSender;
use std::time::{Duration, Instant};

use crate::AppMessage;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// A game as known to Lutris (read from `~/.local/share/lutris/pga.db`).
/// Lutris is the source of truth for the game list; our DB only stores the
/// matching to an achievement source (Steam/GOG) plus user preferences.
pub struct LutrisGame {
    /// Lutris internal numeric id — the stable link we store in our DB.
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub runner: String,
    /// "steam", "gog", or "" for games with no store service.
    pub service: String,
    /// Steam app id (service=steam) or GOG product id (service=gog).
    pub service_id: String,
    pub installed: bool,
    /// Playtime in hours.
    pub playtime: f64,
    /// Unix timestamp of last play.
    pub lastplayed: i64,
    pub platform: String,
    /// Install directory (where the game + its save files live).
    pub directory: String,
}

pub fn lutris_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local").join("share").join("lutris").join("pga.db")
}

/// Read every game from the Lutris database, ordered by name.
pub fn load_lutris_games() -> Result<Vec<LutrisGame>, String> {
    let path = lutris_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, slug, runner, service, service_id, installed, playtime, lastplayed, platform, directory
             FROM games ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let games = stmt
        .query_map([], |row| {
            Ok(LutrisGame {
                id: row.get(0)?,
                name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                slug: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                runner: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                service: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                service_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                installed: row.get::<_, Option<i64>>(6)?.map(|i| i != 0).unwrap_or(false),
                playtime: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                lastplayed: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                platform: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                directory: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for g in games {
        result.push(g.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

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

/// Read just `(id, playtime, lastplayed)` for every game in pga.db.
pub fn load_lutris_playtime() -> Result<Vec<(i64, f64, i64)>, String> {
    let path = lutris_db_path();
    let conn = Connection::open(&path)
        .map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut stmt = conn
        .prepare("SELECT id, playtime, lastplayed FROM games")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for r in rows {
        result.push(r.map_err(|e| e.to_string())?);
    }
    Ok(result)
}
