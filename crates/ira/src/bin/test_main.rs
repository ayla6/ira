// Test binary — run non-UI tests without initializing GTK.
// Usage: cargo run --bin ira-test [psf|npbind|discover|trophies|playtime|all]

use ira_platforms::ps4;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let test = args.get(1).map(|s| s.as_str()).unwrap_or("all");

    match test {
        "psf" => test_psf(),
        "npbind" => test_npbind(),
        "discover" => test_discover(),
        "trophies" => test_trophies(),
        "playtime" => test_playtime(),
        "all" => {
            test_psf();
            test_npbind();
            test_discover();
            test_trophies();
            test_playtime();
        }
        _ => {
            eprintln!("Usage: ira-test [psf|npbind|discover|trophies|playtime|all]");
            std::process::exit(1);
        }
    }
}

fn test_psf() {
    println!("=== PSF Parsing ===");
    let install_dirs = ps4::read_install_dirs();
    for dir in &install_dirs {
        let mut game_dirs = Vec::new();
        ps4::scan_dir_for_test(dir, &mut game_dirs);
        for game_dir in &game_dirs {
            let param_sfo = game_dir.join("sce_sys").join("param.sfo");
            match ps4::parse_psf(&param_sfo) {
                Ok(psf) => {
                    let title = ps4::psf_get_title(&psf);
                    let serial = ps4::psf_get_title_id(&psf);
                    println!(
                        "  {} -> title='{}' serial='{}'",
                        game_dir.display(),
                        title,
                        serial
                    );
                }
                Err(e) => println!("  {} -> ERROR: {}", game_dir.display(), e),
            }
        }
    }
}

fn test_npbind() {
    println!("=== npbind.dat Parsing ===");
    let install_dirs = ps4::read_install_dirs();
    for dir in &install_dirs {
        let mut game_dirs = Vec::new();
        ps4::scan_dir_for_test(dir, &mut game_dirs);
        for game_dir in &game_dirs {
            let npbind = game_dir.join("sce_sys").join("npbind.dat");
            match ps4::parse_npbind(&npbind) {
                Ok(ids) => println!("  {} -> {:?}", game_dir.display(), ids),
                Err(e) => println!("  {} -> ERROR: {}", game_dir.display(), e),
            }
        }
    }
}

fn test_discover() {
    println!("=== Game Discovery ===");
    let games = ps4::discover_games();
    for g in &games {
        println!("  {} ({}) -> NPWR={}", g.title, g.serial, g.npwr_id);
    }
    println!("  Total: {} games", games.len());
}

fn test_trophies() {
    println!("=== Trophy Parsing ===");
    let games = ps4::discover_games();
    for g in &games {
        let trop_xml = ps4::trophy_dir(&g.npwr_id).join("Xml").join("TROP.XML");
        let user_xml = ps4::user_trophy_path(&g.npwr_id);

        let defs = ps4::parse_trop_xml(&trop_xml);
        let unlocks = ps4::parse_user_trophies(&user_xml);

        let earned = unlocks.values().filter(|(e, _)| *e).count();
        println!(
            "  {} ({}): {}/{} earned",
            g.title,
            g.npwr_id,
            earned,
            defs.len()
        );

        for def in defs.iter().take(3) {
            let (earned, _ts) = unlocks.get(&def.id).cloned().unwrap_or((false, 0));
            let status = if earned { "UNLOCKED" } else { "locked" };
            println!("    [{}] {} ({}) — {}", def.ttype, def.name, def.id, status);
        }
        if defs.len() > 3 {
            println!("    ... and {} more", defs.len() - 3);
        }
    }
}

fn test_playtime() {
    println!("=== Playtime ===");
    let times = ps4::read_play_times();
    for (serial, time) in &times {
        let hours = ps4::parse_playtime(time);
        println!("  {} -> {} ({:.2}h)", serial, time, hours);
    }
}
