use ira_config::Config;
use ira_models::{AppMessage, AppSender, GameEntry, MergedAchievement};
use ira_parser::unlock_status_path;
use ::notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type LoadGameFn = Arc<dyn Fn(&GameEntry, &str) -> Result<ira_models::Game, String> + Send + Sync>;

struct WatcherState {
    dir_to_game: HashMap<PathBuf, GameEntry>,
    last_earned: HashMap<String, HashMap<String, bool>>,
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
    pub fn new(cfg: Arc<Config>, sender: AppSender, save_dir: String, load_game: LoadGameFn) -> Result<Self, String> {
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

    pub fn watch(&self, entry: &GameEntry, achievements: &[MergedAchievement]) {
        let earned: HashMap<String, bool> =
            achievements.iter().map(|a| (a.name.clone(), a.earned)).collect();

        let watch_dir = unlock_status_path(
            &self.state.lock().unwrap().save_dir,
            &entry.trophy_source,
            &entry.steam_id,
            &entry.platform_id,
        )
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

        let already_watching = {
            let mut st = self.state.lock().unwrap();
            st.last_earned.insert(entry.steam_id.clone(), earned);
            st.dir_to_game.values().any(|g| g.steam_id == entry.steam_id)
        };

        if !already_watching {
            let mut st = self.state.lock().unwrap();
            st.dir_to_game.insert(watch_dir.clone(), entry.clone());
            drop(st);
            let mut w = self.watcher.lock().unwrap();
            if let Err(e) = w.watch(&watch_dir, RecursiveMode::NonRecursive) {
                eprintln!("Could not watch {:?} for live updates: {}", watch_dir, e);
            }
        }
    }
}

fn event_loop(
    rx: std::sync::mpsc::Receiver<::notify::Result<::notify::Event>>,
    state: Arc<Mutex<WatcherState>>,
    sender: AppSender,
    cfg: Arc<Config>,
) {
    let mut pending: HashMap<String, PathBuf> = HashMap::new();
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
                    let ready: Vec<(String, PathBuf)> = pending.drain().collect();
                    for (app_id, game_dir) in ready {
                        process_reload(&app_id, &game_dir, &state, &sender, &cfg);
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
    pending: &mut HashMap<String, PathBuf>,
) {
    let is_create = matches!(event.kind, EventKind::Create(_));
    let is_modify = matches!(event.kind, EventKind::Modify(_));

    for path in &event.paths {
        if path.file_name().and_then(|n| n.to_str()) != Some("achievements.json") {
            continue;
        }
        if !is_create && !is_modify {
            continue;
        }

        let game_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let st = state.lock().unwrap();
        if let Some(entry) = st.dir_to_game.get(&game_dir) {
            pending.insert(entry.steam_id.clone(), game_dir.clone());
        }
    }
}

fn process_reload(
    app_id: &str,
    game_dir: &Path,
    state: &Mutex<WatcherState>,
    sender: &AppSender,
    cfg: &Config,
) {
    let (entry, save_dir, load_game) = {
        let st = state.lock().unwrap();
        let entry = st.dir_to_game.get(game_dir).cloned();
        let save_dir = st.save_dir.clone();
        let load_game = st.load_game.clone();
        (entry, save_dir, load_game)
    };

    let Some(entry) = entry else { return };

    let game = match load_game(&entry, &save_dir) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Live-reload of {} failed: {}", app_id, e);
            return;
        }
    };

    let mut newly_earned: Vec<MergedAchievement> = Vec::new();

    {
        let mut st = state.lock().unwrap();
        let previous = st.last_earned.get(app_id).cloned().unwrap_or_default();
        let mut current = HashMap::new();
        for a in &game.achievements {
            current.insert(a.name.clone(), a.earned);
            if a.earned && !previous.get(&a.name).copied().unwrap_or(false) {
                newly_earned.push(a.clone());
            }
        }
        st.last_earned.insert(app_id.to_string(), current);
    }

    let game_name = {
        let st = state.lock().unwrap();
        let names = st.game_names.lock().unwrap();
        names.get(app_id).cloned().unwrap_or_else(|| game.name.clone())
    };

    let _ = sender.send(AppMessage::WatcherGameUpdated(game));

    if cfg.notifications_enabled {
        for a in newly_earned {
            crate::notify::notify_unlock(&game_name, &a);
        }
    }
}
