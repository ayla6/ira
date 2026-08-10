use ::notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ira_config::Config;
use ira_models::{AppMessage, AppSender, GameEntry, MergedAchievement};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type LoadGameFn = Arc<dyn Fn(&GameEntry, &str) -> Result<ira_models::Game, String> + Send + Sync>;

struct WatchedGame {
    entry: GameEntry,
    watch_filename: String,
}

struct WatcherState {
    dir_to_game: HashMap<PathBuf, WatchedGame>,
    last_earned: HashMap<i64, HashMap<String, bool>>,
    game_names: Arc<Mutex<HashMap<String, String>>>,
    load_game: LoadGameFn,
    save_dir: String,
}

#[derive(Clone)]
pub struct AchievementWatcher {
    watcher: Arc<Mutex<RecommendedWatcher>>,
    state: Arc<Mutex<WatcherState>>,
}

impl AchievementWatcher {
    pub fn new(
        cfg: Arc<Config>,
        sender: AppSender,
        save_dir: String,
        load_game: LoadGameFn,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(WatcherState {
            dir_to_game: HashMap::new(),
            last_earned: HashMap::new(),
            game_names: Arc::new(Mutex::new(HashMap::new())),
            load_game,
            save_dir,
        }));

        let (tx, rx) = std::sync::mpsc::channel();
        let nw = RecommendedWatcher::new(tx, NotifyConfig::default()).map_err(|e| e.to_string())?;

        let thread_state = state.clone();
        let thread_sender = sender.clone();
        let thread_cfg = cfg.clone();
        std::thread::spawn(move || {
            event_loop(rx, thread_state, thread_sender, thread_cfg);
        });

        Ok(Self {
            watcher: Arc::new(Mutex::new(nw)),
            state,
        })
    }

    pub fn game_names(&self) -> Arc<Mutex<HashMap<String, String>>> {
        self.state.lock().unwrap().game_names.clone()
    }

    /// Register a game for live achievement watching.
    /// `watch_file` is the specific file whose modification triggers a reload
    /// (e.g. achievements.json for GSE, TROPUSR.DAT for PS3).
    /// The parent directory is created if it doesn't exist (e.g. when GSE
    /// hasn't generated its folder yet).
    pub fn watch(&self, entry: &GameEntry, watch_file: &Path, achievements: &[MergedAchievement]) {
        let earned: HashMap<String, bool> = achievements
            .iter()
            .map(|a| (a.name.clone(), a.earned))
            .collect();

        let watch_dir = watch_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let watch_filename = watch_file
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if !watch_dir.as_os_str().is_empty() && !watch_dir.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&watch_dir) {
                eprintln!("Could not create watch directory {:?}: {}", watch_dir, e);
            }
        }

        let already_watching = {
            let mut st = self.state.lock().unwrap();
            st.last_earned.insert(entry.id, earned);
            st.dir_to_game.values().any(|wg| wg.entry.id == entry.id)
        };

        if !already_watching {
            let mut st = self.state.lock().unwrap();
            st.dir_to_game.insert(
                watch_dir.clone(),
                WatchedGame {
                    entry: entry.clone(),
                    watch_filename,
                },
            );
            drop(st);
            let mut w = self.watcher.lock().unwrap();
            if let Err(e) = w.watch(&watch_dir, RecursiveMode::NonRecursive) {
                eprintln!("Could not watch {:?} for live updates: {}", watch_dir, e);
            }
        }
    }

    /// Remove a game from live achievement watching.
    pub fn unwatch(&self, db_id: i64) {
        let watch_dir = {
            let mut st = self.state.lock().unwrap();
            let dir = st
                .dir_to_game
                .iter()
                .find(|(_, wg)| wg.entry.id == db_id)
                .map(|(k, _)| k.clone());
            if let Some(ref dir) = dir {
                st.dir_to_game.remove(dir);
                st.last_earned.remove(&db_id);
            }
            dir
        };
        if let Some(dir) = watch_dir {
            let mut w = self.watcher.lock().unwrap();
            let _ = w.unwatch(&dir);
        }
    }
}

fn event_loop(
    rx: std::sync::mpsc::Receiver<::notify::Result<::notify::Event>>,
    state: Arc<Mutex<WatcherState>>,
    sender: AppSender,
    cfg: Arc<Config>,
) {
    let mut pending: HashMap<i64, PathBuf> = HashMap::new();
    let mut last_event = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) => {
                last_event = Instant::now();
                handle_notify_event(&event, &state, &mut pending);
            }
            Ok(Err(e)) => {
                eprintln!("Achievement watcher error: {}", e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !pending.is_empty() && last_event.elapsed() >= Duration::from_millis(300) {
                    let ready: Vec<(i64, PathBuf)> = pending.drain().collect();
                    for (db_id, game_dir) in ready {
                        process_reload(db_id, &game_dir, &state, &sender, &cfg);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_notify_event(
    event: &::notify::Event,
    state: &Mutex<WatcherState>,
    pending: &mut HashMap<i64, PathBuf>,
) {
    let is_create = matches!(event.kind, EventKind::Create(_));
    let is_modify = matches!(event.kind, EventKind::Modify(_));

    if !is_create && !is_modify {
        return;
    }

    for path in &event.paths {
        let game_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let st = state.lock().unwrap();
        if let Some(wg) = st.dir_to_game.get(&game_dir) {
            let filename = path.file_name().and_then(|n| n.to_str());
            if filename == Some(&wg.watch_filename) {
                pending.insert(wg.entry.id, game_dir.clone());
            }
        }
    }
}

fn process_reload(
    db_id: i64,
    game_dir: &Path,
    state: &Mutex<WatcherState>,
    sender: &AppSender,
    cfg: &Config,
) {
    let (entry, save_dir, load_game) = {
        let st = state.lock().unwrap();
        let entry = st.dir_to_game.get(game_dir).map(|wg| wg.entry.clone());
        let save_dir = st.save_dir.clone();
        let load_game = st.load_game.clone();
        (entry, save_dir, load_game)
    };

    let Some(entry) = entry else { return };

    let game = match load_game(&entry, &save_dir) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Live-reload of {} failed: {}", db_id, e);
            return;
        }
    };

    let mut newly_earned: Vec<MergedAchievement> = Vec::new();

    {
        let mut st = state.lock().unwrap();
        let previous = st.last_earned.get(&db_id).cloned().unwrap_or_default();
        let mut current = HashMap::new();
        for a in &game.achievements {
            current.insert(a.name.clone(), a.earned);
            if a.earned && !previous.get(&a.name).copied().unwrap_or(false) {
                newly_earned.push(a.clone());
            }
        }
        st.last_earned.insert(db_id, current);
    }

    let game_name = {
        let st = state.lock().unwrap();
        let names = st.game_names.lock().unwrap();
        names
            .get(&game.app_id)
            .cloned()
            .unwrap_or_else(|| game.name.clone())
    };

    let _ = sender.send(AppMessage::WatcherGameUpdated(game));

    if cfg.notifications_enabled {
        for a in newly_earned {
            crate::notify::notify_unlock(&game_name, &a);
        }
    }
}
