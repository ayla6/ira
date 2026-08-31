//! Reproductions for the "add a game while the library is still loading"
//! race: the auto-add wizard publishes `NewGame` while the startup load is
//! still building its snapshot, and the two publishes interleave in the UI
//! message queue. Requires a display (GTK widgets are constructed unrealized).
//!
//! GTK must be initialized and driven from a single thread, so the scenarios
//! live in one test that runs each phase against a fresh state.

use gio::prelude::*;
use ira::ui::{build_ui, handle_app_message, AppContext};
use ira::AppMessage;
use ira_models::Game;
use std::sync::mpsc;

fn test_game(db_id: i64, name: &str) -> Game {
    let mut game = Game {
        db_id,
        ..Default::default()
    };
    game.set_name(name);
    game
}

fn test_state() -> ira::ui::SharedState {
    gtk4::init().expect("gtk init (needs a display)");
    adw::init().expect("adw init");

    let tmp = tempfile::tempdir().unwrap();
    let cfg = ira_config::Config {
        save_dir: tmp.path().to_string_lossy().into_owned(),
        ..Default::default()
    };

    let app = adw::Application::new(Some("org.ira.loadtest"), Default::default());
    let db = ira_db::init_db(&format!("{}/ira.db", cfg.save_dir));
    let steam = std::sync::Arc::new(ira_api::SteamDataClient::new(
        String::new(),
        String::new(),
        &format!("{}/data", cfg.save_dir),
    ));
    // Receiver dropped on purpose: enrichment threads may write to it, the
    // test drives handle_app_message directly.
    let (tx, _rx) = mpsc::channel();
    let mut fds = [0i32; 2];
    unsafe {
        libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let sender = ira::AppSender::new(tx, fds[1]);

    let ctx = AppContext {
        steam,
        watcher: None,
        db,
        sender,
        game_names: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        controller_registry: ira_input::ControllerRegistry::snapshot_only(),
    };
    let state = build_ui(&app, Vec::new(), cfg, ctx);
    std::mem::forget(tmp); // lives for the whole test process
    state
}

/// Run pending main-loop work, including the 300ms sidebar rebuild timeout.
fn drain_main_loop(ms: u64) {
    let loop_ = glib::MainLoop::new(None, false);
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), {
        let loop_ = loop_.clone();
        move || loop_.quit()
    });
    loop_.run();
}

#[test]
fn test_adding_a_game_while_the_library_loads_keeps_every_game() {
    // Phase 1: NewGame lands mid-load, and the load's snapshot (taken
    // before the add was committed) arrives afterwards.
    {
        let state = test_state();

        // Startup load in flight: the loading screen is up (created by the
        // progress handler because no view exists yet).
        handle_app_message(
            &state,
            AppMessage::GamesLoadProgress {
                status: "Scanning Steam games…".into(),
                completed: 1,
                total: 3,
            },
        );
        assert!(state.borrow().loading.is_some(), "loading screen should be up");

        // The user adds a game while the load is still running.
        handle_app_message(&state, AppMessage::NewGame(test_game(999, "Just Added")));
        assert!(state.borrow().games.iter().any(|g| g.db_id == 999));

        drain_main_loop(400);
        {
            let s = state.borrow();
            assert!(
                s.loading.is_some(),
                "loading screen must stay up until the load finishes"
            );
            assert_eq!(s.grid_store.n_items(), 0, "no grid flash with a partial list");
            assert!(s.games.iter().any(|g| g.db_id == 999));
        }

        // Later progress from the still-running load must not fight the view.
        handle_app_message(
            &state,
            AppMessage::GamesLoadProgress {
                status: "Loading saved games…".into(),
                completed: 2,
                total: 3,
            },
        );
        assert!(state.borrow().loading.is_some());
        // The final snapshot lacks the new game — the library must keep it.
        handle_app_message(
            &state,
            AppMessage::GamesLoaded(vec![
                test_game(1, "Old Game A"),
                test_game(2, "Old Game B"),
            ]),
        );

        drain_main_loop(400);
        let s = state.borrow();
        assert!(s.loading.is_none(), "library view should be up at the end");
        assert_eq!(
            s.grid_store.n_items(),
            3,
            "grid must show both old games and the one added mid-load"
        );
        let names: Vec<String> = s.games.iter().map(|g| g.name.clone()).collect();
        assert!(names.contains(&"Just Added".to_string()), "new game survived: {names:?}");
        assert!(names.contains(&"Old Game A".to_string()));
        assert!(names.contains(&"Old Game B".to_string()));
    }

    // Phase 2: the snapshot lands first and the wizard's NewGame arrives
    // after the library is already shown.
    {
        let state = test_state();

        handle_app_message(&state, AppMessage::GamesLoaded(vec![test_game(1, "Old Game")]));
        drain_main_loop(400);

        handle_app_message(&state, AppMessage::NewGame(test_game(999, "Just Added")));
        drain_main_loop(400);

        let s = state.borrow();
        assert_eq!(s.games.len(), 2, "old and new game must both be listed");
        assert_eq!(s.grid_store.n_items(), 2);
    }
}
