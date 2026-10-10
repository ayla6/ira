use crate::bench::run_bench;
use crate::game_list::start_saved_game_load;
use crate::ui::{build_ui, handle_app_message, SharedState};
use gtk4::glib;
use ira_api::SteamDataClient;
use ira_config as config;
use ira_db as db;
use ira_models::{AppMessage, AppSender};
use ira_platforms::ps3::Rpcs3Watcher;
use ira_platforms::ps4::ShadPS4Watcher;
use ira_watcher::AchievementWatcher;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

struct MainLoopData {
    read_fd: i32,
    receiver: RefCell<mpsc::Receiver<AppMessage>>,
    state: SharedState,
}

impl MainLoopData {
    fn drain(&self) {
        let mut buf = [0u8; 256];
        while unsafe { libc::read(self.read_fd, buf.as_mut_ptr() as *mut _, 256) } > 0 {}
        while let Ok(msg) = self.receiver.borrow_mut().try_recv() {
            handle_app_message(&self.state, msg);
        }
    }
}

impl Drop for MainLoopData {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read_fd);
        }
    }
}

pub fn remove_source(state: &SharedState) {
    let source_id = state.borrow().source_id.take();
    let Some(source_id) = source_id else {
        return;
    };

    let removed = unsafe { glib::ffi::g_source_remove(source_id) };
    if removed == glib::ffi::GFALSE {
        eprintln!("Failed to remove GLib application message source {source_id}");
    }
}

fn attach_message_source(state: &SharedState, read_fd: i32, receiver: mpsc::Receiver<AppMessage>) {
    let data = Rc::new(MainLoopData {
        read_fd,
        receiver: RefCell::new(receiver),
        state: state.clone(),
    });
    let source_data = data.clone();
    let source_id = glib_unix::unix_fd_add_local(read_fd, glib::IOCondition::IN, move |_, _| {
        source_data.drain();
        glib::ControlFlow::Continue
    });
    state.borrow().source_id.set(Some(source_id.as_raw()));
}

pub fn activate(app: &adw::Application) -> SharedState {
    let _span = tracing::info_span!("activate").entered();

    let cfg = {
        let _s = tracing::info_span!("load_config").entered();
        config::load_config()
    };

    if let Err(e) = cfg.ensure_rom_folders() {
        eprintln!("Failed to create ROM library folders: {e}");
    }

    let db = {
        let _s = tracing::info_span!("init_db").entered();
        db::init_db(&format!("{}/ira.db", cfg.save_dir))
    };

    {
        let _s = tracing::info_span!("ensure_skeleton").entered();
        ira_platforms::api_emulators::ensure_skeleton(&cfg.save_dir);
    }

    let mut pipe_fds = [0i32; 2];
    unsafe {
        libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    let (tx, rx) = mpsc::channel::<AppMessage>();
    let sender = AppSender::new(tx, write_fd);
    let controller_registry = match ira_input::ControllerRegistry::start() {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!("Controller registry unavailable: {e}");
            ira_input::ControllerRegistry::snapshot_only()
        }
    };

    let save_dir = cfg.save_dir.clone();

    // While Ira is open idle controllers run their default layout;
    // Disabled keeps native desktop input.
    crate::input_desktop::start(save_dir.clone());

    start_saved_game_load(db.clone(), save_dir.clone(), cfg.clone(), sender.clone());

    let steam_api_key = cfg.steam_api_key.clone();
    let steam_griddb_api_key = cfg.steam_griddb_api_key.clone();
    let cfg_for_watcher = Arc::new(cfg.clone());

    let steam = Arc::new(SteamDataClient::new(
        steam_api_key,
        steam_griddb_api_key,
        &format!("{}/data", save_dir),
    ));
    steam.set_sgdb_disabled_assets(&cfg.sgdb_disabled_assets);
    steam.set_sgdb_filtered_users(
        &cfg.sgdb_filtered_users
            .iter()
            .map(|u| (u.name.clone(), u.steam64.clone()))
            .collect::<Vec<_>>(),
    );
    steam.set_sgdb_filtered_styles(&cfg.sgdb_filtered_styles);
    steam.set_sgdb_show_filtered(cfg.sgdb_show_filtered);

    let state = {
        let _s = tracing::info_span!("build_ui_wrap").entered();
        build_ui(
            app,
            Vec::new(),
            cfg,
            crate::ui::AppContext {
                steam: steam.clone(),
                watcher: None,
                db: db.clone(),
                sender: sender.clone(),
                game_names: Arc::new(Mutex::new(HashMap::new())),
                controller_registry,
            },
        )
    };

    {
        let _s = tracing::info_span!("post_ui_setup").entered();

        let (watcher, game_names) = match AchievementWatcher::new(
            cfg_for_watcher,
            sender.clone(),
            save_dir.clone(),
            Arc::new(crate::game_loader::load_game),
        ) {
            Ok(watcher) => {
                let game_names = watcher.game_names();
                (Some(watcher), Some(game_names))
            }
            Err(e) => {
                eprintln!("Live achievement watching unavailable: {}", e);
                (None, None)
            }
        };
        {
            let mut state = state.borrow_mut();
            state.steam = steam.clone();
            if let Some(game_names) = game_names {
                state.game_names = game_names;
            }
            state.watcher = watcher;
        }

        refresh_playtime_watchers(&state);
    }

    {
        let steam = state.borrow().steam.clone();
        let db = db.clone();
        std::thread::spawn(move || {
            let entries = match db::load_all_games(&db) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in &entries {
                if entry.steam_id.is_empty() {
                    continue;
                }
                if !entry.trophy_source.has_steam_enrichment() {
                    continue;
                }
                steam.ensure_steamcmd_cache(&entry.steam_id);
            }
        });
    }

    {
        attach_message_source(&state, read_fd, rx);
    }

    if std::env::var("AV_BENCH").is_ok() {
        run_bench(state.clone());
    }

    state
}

pub(crate) fn refresh_playtime_watchers(state: &SharedState) {
    let (sender, shadps4_executable, rpcs3_executable) = {
        let state = state.borrow();
        (
            state.sender.clone(),
            state.cfg.shadps4_executable.clone(),
            state.cfg.rpcs3_executable.clone(),
        )
    };
    let shadps4_watcher = ShadPS4Watcher::new(sender.clone(), &shadps4_executable)
        .map(Some)
        .unwrap_or_else(|error| {
            eprintln!("shadPS4 playtime watching unavailable: {error}");
            None
        });
    let rpcs3_watcher = Rpcs3Watcher::new(sender, &rpcs3_executable)
        .map(Some)
        .unwrap_or_else(|error| {
            eprintln!("RPCS3 playtime watching unavailable: {error}");
            None
        });
    let mut state = state.borrow_mut();
    state.shadps4_watcher = shadps4_watcher;
    state.rpcs3_watcher = rpcs3_watcher;
}
