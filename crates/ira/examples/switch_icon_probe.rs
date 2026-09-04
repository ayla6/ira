//! Diagnostic tool: runs the exact Switch icon-extraction chain
//! (keys -> control-NCA decrypt -> emulator caches) against every Switch
//! entry in the library database, reporting where and how long it fails.
//!
//! Usage: switch_icon_probe

use ira_config::load_config;
use ira_platforms::switch::{native_icon, SwitchCaches, SwitchIcon};
use std::path::Path;

fn describe(icon: &SwitchIcon) -> String {
    match icon {
        SwitchIcon::Bytes(b) => format!("Bytes({} bytes)", b.len()),
        SwitchIcon::File(f) => format!("File({} exists={})", f.display(), f.is_file()),
        SwitchIcon::None => "None".to_string(),
    }
}

fn main() {
    let cfg = load_config();
    let exe = cfg.console("switch").executable.clone();
    println!(
        "configured switch executable: {exe:?} (exists: {})",
        Path::new(&exe).is_file()
    );
    println!("rom roots: {:?}", cfg.all_rom_roots());

    println!("\n== detected switch emulators:");
    for choice in ira_platforms::emulator_detect::detect_emulators("switch") {
        let n = ira_platforms::switch::discover_installed_games(&choice.launch_command).len();
        println!("  {:?} -> {n} installed titles", choice.launch_command);
    }
    let n = ira_platforms::switch::discover_installed_games(&exe).len();
    println!("  configured exe {exe:?} -> {n} installed titles");

    let db = ira_db::init_db(&format!("{}/ira.db", cfg.save_dir));
    let entries = ira_db::load_all_games(&db).unwrap_or_default();
    let switch_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == ira_models::GameKind::Switch)
        .collect();
    println!(
        "\n== {} Switch DB entries (automode path, with timing):",
        switch_entries.len()
    );
    let caches = SwitchCaches::load(&exe);
    for e in &switch_entries {
        // Same game_root resolution as ui::image_manager_helpers::native_icon_bytes.
        let resolved;
        let root: &Path = if Path::new(&e.rom_path).is_absolute() {
            Path::new(&e.rom_path)
        } else {
            match cfg.resolve_rom_path(&e.platform_id, &e.rom_path) {
                Some(r) => {
                    resolved = r;
                    &resolved
                }
                None => {
                    println!(
                        "{:<40} rom={:?} -> unresolvable, extraction skipped",
                        e.title, e.rom_path
                    );
                    continue;
                }
            }
        };
        let t0 = std::time::Instant::now();
        let icon = native_icon(root, &caches, &exe);
        println!(
            "{:<40} rom={:?} exists={} -> {} in {:?}",
            e.title,
            root.display(),
            root.exists(),
            describe(&icon),
            t0.elapsed(),
        );
    }
}
