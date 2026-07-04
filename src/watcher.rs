use crate::config::Config;
use crate::parser::{load_game, MergedAchievement};
use crate::AppMessage;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct WatcherState {
    dir_to_app: HashMap<PathBuf, String>,
    last_earned: HashMap<String, HashMap<String, bool>>,
    game_names: Arc<Mutex<HashMap<String, String>>>,
    root_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AchievementWatcher {
    watcher: Arc<Mutex<RecommendedWatcher>>,
    state: Arc<Mutex<WatcherState>>,
}

impl AchievementWatcher {
    pub fn new(cfg: Arc<Config>, sender: Sender<AppMessage>) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(WatcherState {
            dir_to_app: HashMap::new(),
            last_earned: HashMap::new(),
            game_names: Arc::new(Mutex::new(HashMap::new())),
            root_dir: None,
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

    pub fn watch(&self, app_id: &str, game_dir: &str, achievements: &[MergedAchievement]) {
        let earned: HashMap<String, bool> =
            achievements.iter().map(|a| (a.name.clone(), a.earned)).collect();
        let game_dir_path = PathBuf::from(game_dir);

        let already_watching = {
            let mut st = self.state.lock().unwrap();
            st.last_earned.insert(app_id.to_string(), earned);
            st.dir_to_app.values().any(|id| id == app_id)
        };

        if !already_watching {
            self.state.lock().unwrap().dir_to_app.insert(game_dir_path.clone(), app_id.to_string());
            let mut w = self.watcher.lock().unwrap();
            if let Err(e) = w.watch(&game_dir_path, RecursiveMode::NonRecursive) {
                eprintln!("Could not watch {} for live updates: {}", game_dir, e);
            }
        }
    }

    pub fn watch_root(&self, dir: &str) -> Result<(), String> {
        let path = PathBuf::from(dir);
        self.state.lock().unwrap().root_dir = Some(path.clone());
        let mut w = self.watcher.lock().unwrap();
        w.watch(&path, RecursiveMode::NonRecursive).map_err(|e| e.to_string())
    }
}

fn event_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    state: Arc<Mutex<WatcherState>>,
    sender: Sender<AppMessage>,
    cfg: Arc<Config>,
) {
    let mut pending: HashMap<String, PathBuf> = HashMap::new();
    let mut last_event = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) => {
                last_event = Instant::now();
                handle_notify_event(&event, &state, &sender, &mut pending);
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
    event: &notify::Event,
    state: &Mutex<WatcherState>,
    sender: &Sender<AppMessage>,
    pending: &mut HashMap<String, PathBuf>,
) {
    let is_create = matches!(event.kind, EventKind::Create(_));
    let is_modify = matches!(event.kind, EventKind::Modify(_));

    for path in &event.paths {
        let st = state.lock().unwrap();

        if let Some(root) = &st.root_dir {
            if let Some(parent) = path.parent() {
                if parent == root.as_path() && is_create {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.parse::<i64>().is_ok() && path.is_dir() {
                        let _ = sender.send(AppMessage::WatcherNewGameDir {
                            app_id: name.to_string(),
                            game_dir: path.to_string_lossy().into_owned(),
                        });
                    }
                    return;
                }
            }
        }

        if path.file_name().and_then(|n| n.to_str()) != Some("achievements.json") {
            continue;
        }
        if !is_create && !is_modify {
            continue;
        }

        let game_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if let Some(app_id) = st.dir_to_app.get(&game_dir) {
            pending.insert(app_id.clone(), game_dir.clone());
        }
    }
}

fn process_reload(
    app_id: &str,
    game_dir: &Path,
    state: &Mutex<WatcherState>,
    sender: &Sender<AppMessage>,
    cfg: &Config,
) {
    let game = match load_game(app_id, game_dir) {
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
            notify_unlock(&game_name, &a);
        }
    }
}

fn notify_unlock(game_name: &str, ach: &MergedAchievement) {
    let title = format!("{} — Achievement Unlocked", game_name);
    let body = if ach.description.is_empty() {
        ach.display_name.clone()
    } else {
        format!("{}\n{}", ach.display_name, ach.description)
    };
    let icon = if ach.icon_path.is_empty() {
        "starred-symbolic".to_string()
    } else {
        ach.icon_path.clone()
    };

    std::thread::spawn(move || {
        let _ = Command::new("notify-send")
            .args([
                "--app-name=Achievement Viewer",
                &format!("--icon={}", icon),
                &title,
                &body,
            ])
            .spawn()
            .and_then(|mut c| c.wait());
    });
}
