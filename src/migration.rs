use crate::db;
use crate::parser;

pub fn populate_db_from_dirs(db: &db::DbConn, save_dir: &str) {
    let steam_dir = format!("{}/steam", save_dir);
    if let Ok(entries) = std::fs::read_dir(&steam_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            let title = parser::read_app_name(save_dir, &app_id).unwrap_or_default();
            let _ = db::add_game(db, "steam", &app_id, &app_id, &title);
        }
    }

    let gog_dir = format!("{}/gog", save_dir);
    if let Ok(galaxy_entries) = std::fs::read_dir(&gog_dir) {
        for galaxy_entry in galaxy_entries.flatten() {
            if !galaxy_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let galaxy_path = galaxy_entry.path();
            if let Ok(product_entries) = std::fs::read_dir(&galaxy_path) {
                for product_entry in product_entries.flatten() {
                    if !product_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let product_dir = product_entry.path();
                    let product_id = match product_entry.file_name().to_str() {
                        Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                        _ => continue,
                    };
                    let app_id = match std::fs::read_to_string(product_dir.join("steam_appid.txt")) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue,
                    };
                    if app_id.parse::<i64>().is_err() {
                        continue;
                    }
                    let title = parser::read_app_name(save_dir, &app_id).unwrap_or_default();
                    let _ = db::add_game(db, "gog", &app_id, &product_id, &title);
                }
            }
        }
    }
}

pub fn migrate_data_dir(save_dir: &str) {
    let data_dir = std::path::Path::new(save_dir).join("data");
    let steam_dir = data_dir.join("steam");

    let entries = match std::fs::read_dir(&data_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let _ = std::fs::create_dir_all(&steam_dir);

    for entry in entries.flatten() {
        let name = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if name == "steam" || name == "steamgriddb" {
            continue;
        }
        if name.parse::<i64>().is_err() {
            continue;
        }
        let src = entry.path();
        let dest = steam_dir.join(&name);
        if dest.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            eprintln!("Migration: could not move {} → {}: {}", src.display(), dest.display(), e);
        }
    }
}
