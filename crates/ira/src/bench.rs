use crate::ui::{hide_to_background, malloc_trim, switch_to_game, SharedState};
use gtk4::glib;
use std::time::Duration;

fn vm_rss_kb() -> i64 {
    let data = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in data.lines() {
        if line.starts_with("VmRSS:") {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() >= 2 {
                return f[1].parse().unwrap_or(-1);
            }
        }
    }
    -1
}

fn log(tag: &str) {
    let rss = vm_rss_kb();
    eprintln!(
        "[bench] {:<30} RSS={} KB ({:.1} MB)",
        tag,
        rss,
        rss as f64 / 1024.0
    );
}

pub fn run_bench(state: SharedState) {
    let games = state.borrow().games.clone();
    if games.len() < 2 {
        eprintln!("[bench] need >=2 games on disk");
        std::process::exit(1);
    }

    let app_a = games[0].db_id;
    let app_b = games[1].db_id;

    log("startup");

    let mut phase: u8 = 0; // 0=wait, 1=switching, 2=hide, 3=done
    let mut switch_i: i32 = 0;

    glib::timeout_add_local(Duration::from_millis(900), move || {
        match phase {
            0 => {
                log("settled, pre-switch");
                phase = 1;
            }
            1 => {
                let id = if switch_i % 2 == 0 { app_a } else { app_b };
                switch_to_game(&state, id, None);
                log(&format!("after switch {:2} ({})", switch_i + 1, id));
                switch_i += 1;
                if switch_i >= 10 {
                    phase = 2;
                }
            }
            2 => {
                hide_to_background(&state);
                log("after hide-to-background");
                phase = 3;
            }
            3 => {
                unsafe {
                    malloc_trim(0);
                }
                log("after malloc_trim(0)");
                std::process::exit(0);
            }
            _ => {}
        }
        glib::ControlFlow::Continue
    });
}
