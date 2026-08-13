use crate::bench::run_bench;
use crate::game_list::start_game_list_load;
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
use std::sync::{mpsc, Arc, Mutex};

extern "C" {
    fn g_unix_fd_source_new(fd: i32, condition: u32) -> *mut std::ffi::c_void;
}

struct MainLoopData {
    read_fd: i32,
    receiver: RefCell<mpsc::Receiver<AppMessage>>,
    state: SharedState,
}

unsafe extern "C" fn source_trampoline(
    _fd: i32,
    _condition: u32,
    data: glib::ffi::gpointer,
) -> glib::ffi::gboolean {
    let data: &MainLoopData = &*(data as *const MainLoopData);
    let mut buf = [0u8; 256];
    while libc::read(data.read_fd, buf.as_mut_ptr() as *mut _, 256) > 0 {}
    while let Ok(msg) = data.receiver.borrow_mut().try_recv() {
        handle_app_message(&data.state, msg);
    }
    glib::ffi::G_SOURCE_CONTINUE
}

unsafe extern "C" fn source_destroy(data: glib::ffi::gpointer) {
    let data = Box::from_raw(data as *mut MainLoopData);
    libc::close(data.read_fd);
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

    start_game_list_load(db.clone(), save_dir.clone(), cfg.clone(), sender.clone());

    let steam_api_key = cfg.steam_api_key.clone();
    let steam_griddb_api_key = cfg.steam_griddb_api_key.clone();
    let cfg_for_watcher = Arc::new(cfg.clone());

    let steam = Arc::new(SteamDataClient::new(
        steam_api_key,
        steam_griddb_api_key,
        &format!("{}/data", save_dir),
    ));

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

        state.borrow_mut().steam = steam.clone();

        let watcher = match AchievementWatcher::new(
            cfg_for_watcher,
            sender.clone(),
            save_dir.clone(),
            Arc::new(crate::game_loader::load_game),
        ) {
            Ok(w) => {
                let game_names = w.game_names();
                state.borrow_mut().game_names = game_names;
                Some(w)
            }
            Err(e) => {
                eprintln!("Live achievement watching unavailable: {}", e);
                None
            }
        };
        state.borrow_mut().watcher = watcher;

        refresh_playtime_watchers(&state);
    }

    // Warm steamcmd.net cache for any game with a steam_id in the background
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
        let data = Box::new(MainLoopData {
            read_fd,
            receiver: RefCell::new(rx),
            state: state.clone(),
        });
        let data_ptr = Box::into_raw(data) as *mut std::ffi::c_void;
        unsafe {
            let source = g_unix_fd_source_new(read_fd, glib::ffi::G_IO_IN);
            let func_ptr: unsafe extern "C" fn(
                i32,
                u32,
                glib::ffi::gpointer,
            ) -> glib::ffi::gboolean = source_trampoline;
            glib::ffi::g_source_set_callback(
                source as *mut glib::ffi::GSource,
                std::mem::transmute::<
                    unsafe extern "C" fn(i32, u32, glib::ffi::gpointer) -> glib::ffi::gboolean,
                    glib::ffi::GSourceFunc,
                >(func_ptr),
                data_ptr,
                Some(source_destroy),
            );
            let source_id =
                glib::ffi::g_source_attach(source as *mut glib::ffi::GSource, std::ptr::null_mut());
            state.borrow().source_id.set(Some(source_id));
            glib::ffi::g_source_unref(source as *mut glib::ffi::GSource);
        }
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
