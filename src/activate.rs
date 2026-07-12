use crate::api::SteamClient;
use crate::bench::run_bench;
use crate::config;
use crate::db;
use crate::game_list::build_game_list;
use crate::models::{AppMessage, AppSender};
use crate::platforms::lutris_watcher::LutrisWatcher;
use crate::platforms::ps4::ShadPS4Watcher;
use crate::ui::{build_ui, handle_app_message, SharedState};
use crate::watcher::AchievementWatcher;
use gtk4::glib;
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
    let _ = Box::from_raw(data as *mut MainLoopData);
}

pub fn activate(app: &adw::Application) -> SharedState {
    let cfg = config::load_config();

    let db = db::init_db(&format!("{}/gse.db", cfg.save_dir));

    crate::platforms::api_emulators::ensure_skeleton(&cfg.save_dir);

    let steam = Arc::new(SteamClient::new(
        cfg.steam_api_key.clone(),
        cfg.steam_griddb_api_key.clone(),
        &format!("{}/data", cfg.save_dir),
    ));

    let mut pipe_fds = [0i32; 2];
    unsafe {
        libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    let (tx, rx) = mpsc::channel::<AppMessage>();
    let sender = AppSender::new(tx, write_fd);

    let cfg_for_watcher = Arc::new(cfg.clone());
    let watcher = match AchievementWatcher::new(cfg_for_watcher, sender.clone(), cfg.save_dir.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("Live achievement watching unavailable: {}", e);
            None
        }
    };

    let game_names = watcher.as_ref().map(|w| w.game_names()).unwrap_or_else(|| {
        Arc::new(Mutex::new(HashMap::new()))
    });

    let lutris_watcher = match LutrisWatcher::new(sender.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("Lutris DB watching unavailable: {}", e);
            None
        }
    };

    let shadps4_watcher = match ShadPS4Watcher::new(sender.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("shadPS4 playtime watching unavailable: {}", e);
            None
        }
    };

    let shadps4_enabled = cfg.shadps4_enabled;
    let save_dir = cfg.save_dir.clone();
    let state = build_ui(
        app,
        Vec::new(),
        cfg,
        steam.clone(),
        watcher.clone(),
        db.clone(),
        sender.clone(),
        game_names,
    );

    state.borrow_mut().lutris_watcher = lutris_watcher;
    state.borrow_mut().shadps4_watcher = shadps4_watcher;

    {
        let data = Box::new(MainLoopData {
            read_fd,
            receiver: RefCell::new(rx),
            state: state.clone(),
        });
        let data_ptr = Box::into_raw(data) as *mut std::ffi::c_void;
        unsafe {
        let source = g_unix_fd_source_new(read_fd, glib::ffi::G_IO_IN);
        let func_ptr: unsafe extern "C" fn(i32, u32, glib::ffi::gpointer) -> glib::ffi::gboolean = source_trampoline;
        glib::ffi::g_source_set_callback(
            source as *mut glib::ffi::GSource,
            std::mem::transmute(func_ptr),
            data_ptr,
            Some(source_destroy),
        );
            glib::ffi::g_source_attach(source as *mut glib::ffi::GSource, std::ptr::null_mut());
            glib::ffi::g_source_unref(source as *mut glib::ffi::GSource);
        }
    }

    {
        let db = db.clone();
        let sender = sender.clone();
        let save_dir = save_dir.clone();
        std::thread::spawn(move || {
            let games = build_game_list(&db, &save_dir, shadps4_enabled);
            let _ = sender.send(AppMessage::GamesLoaded(games));
        });
    }

    if std::env::var("AV_BENCH").is_ok() {
        run_bench(state.clone());
    }

    state
}
