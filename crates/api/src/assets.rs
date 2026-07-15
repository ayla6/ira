use std::path::Path;

use crate::SteamClient;
use ira_models::DlcInfo;

impl SteamClient {
    pub fn ensure_sgdb_assets(&self, sgdb_id: &str) -> (String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        let _ = std::fs::create_dir_all(&dir);

        let icon_path = if let Some(existing) = ira_parser::find_image_file(&dir, "icon") {
            existing.to_string_lossy().into_owned()
        } else {
            let sgdb_key = self.sgdb_api_key();
            if sgdb_key.is_empty() {
                String::new()
            } else {
                let resp = self.http
                    .get(format!("https://www.steamgriddb.com/api/v2/icons/game/{}", sgdb_id))
                    .header("Authorization", format!("Bearer {}", sgdb_key))
                    .send();
                match resp {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(raw) = r.json::<serde_json::Value>() {
                            if let Some(data) = raw.get("data").and_then(|d| d.as_array()) {
                                let mut best: Option<(&serde_json::Value, i64)> = None;
                                for item in data {
                                    let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
                                    if best.is_none() || (w <= 128 && w < best.unwrap().1) {
                                        best = Some((item, w));
                                    }
                                }
                                if let Some(chosen) = best.map(|(item, _)| item) {
                                    if let Some(url) = chosen.get("url").and_then(|u| u.as_str()) {
                                        let ext = ira_parser::url_extension(url);
                                        let dest = dir.join(format!("icon.{}", ext));
                                        if self.download_file(url, &dest).is_ok() {
                                            let converted = ira_parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                                            converted.to_string_lossy().into_owned()
                                        } else { String::new() }
                                    } else { String::new() }
                                } else { String::new() }
                            } else { String::new() }
                        } else { String::new() }
                    }
                    _ => String::new(),
                }
            }
        };

        let hero_path = if let Some(existing) = ira_parser::find_image_file(&dir, "library_hero") {
            existing.to_string_lossy().into_owned()
        } else if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "heroes") {
            let ext = ira_parser::url_extension(&url);
            self.fetch_image(&url, &dir.join(format!("library_hero.{}", ext)))
        } else { String::new() };

        let grid_path = if let Some(existing) = ira_parser::find_image_file(&dir, "library_600x900") {
            existing.to_string_lossy().into_owned()
        } else if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "grids") {
            let ext = ira_parser::url_extension(&url);
            self.fetch_image(&url, &dir.join(format!("library_600x900.{}", ext)))
        } else { String::new() };

        let logo_path = if let Some(existing) = ira_parser::find_image_file(&dir, "logo") {
            existing.to_string_lossy().into_owned()
        } else if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "logos") {
            let ext = ira_parser::url_extension(&url);
            self.fetch_image(&url, &dir.join(format!("logo.{}", ext)))
        } else { String::new() };

        let header_path = if let Some(existing) = ira_parser::find_image_file(&dir, "header") {
            existing.to_string_lossy().into_owned()
        } else {
            self.force_download_sgdb(sgdb_id, "header", false)
        };

        (icon_path, hero_path, grid_path, logo_path, header_path)
    }

    pub fn ensure_assets(
        &self,
        app_id: &str,
        has_local_icon: bool,
    ) -> (String, String) {
        let dir = self.game_dir(app_id);

        let icon_path = if has_local_icon {
            String::new()
        } else {
            let mut found = String::new();
            if let Some(cached) = self.find_cached_icon(app_id) {
                found = cached.to_string_lossy().into_owned();
            }
            if found.is_empty() {
                if let Some(url) = self.fetch_sgdb_icon_url(app_id) {
                    let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                    let dest = dir.join(format!("icon.{}", ext));
                    if self.download_file(&url, &dest).is_ok() {
                        let converted = ira_parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                        found = converted.to_string_lossy().into_owned();
                    }
                }
            }
            found
        };

        let hero_path = if let Some(cached) = self.find_cached_hero(app_id) {
            cached.to_string_lossy().into_owned()
        } else {
            self.fetch_image_fallback(
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero_2x.jpg", app_id),
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", app_id),
                &dir.join("library_hero.jpg"),
            )
        };

        (icon_path, hero_path)
    }

    pub fn ensure_grids(&self, app_id: &str) -> (String, String, String) {
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| {
            format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix)
        };

        let grid_path = self.fetch_image_fallback(
            &cdn("library_600x900_2x.jpg"),
            &cdn("library_600x900.jpg"),
            &dir.join("library_600x900.jpg"),
        );

        let header_path = self.fetch_image_fallback(&cdn("header.jpg"), "", &dir.join("header.jpg"));

        let logo_path = self.fetch_image_fallback(&cdn("logo.png"), "", &dir.join("logo.png"));

        (grid_path, header_path, logo_path)
    }

    pub fn ensure_dlc_images(&self, app_id: &str, dlcs: &mut std::collections::HashMap<String, DlcInfo>) {
        let base_dir = self.game_dir(app_id);
        let dlc_dir = base_dir.join("dlc");
        let _ = std::fs::create_dir_all(&dlc_dir);

        for (_, dlc) in dlcs.iter_mut() {
            if dlc.image_url.is_empty() {
                continue;
            }
            if dlc.image_url.starts_with("dlc/") {
                continue;
            }
            let local_rel = format!("dlc/{}.jpg", dlc.app_id);
            let dest = base_dir.join(&local_rel);
            if !dest.exists() {
                let _ = self.download_file(&dlc.image_url, &dest);
            }
            if dest.exists() {
                dlc.image_url = local_rel;
            }
        }
    }

    pub fn force_download_steam(&self, app_id: &str, asset: &str) -> String {
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix);
        match asset {
            "hero" => {
                let dest = dir.join("library_hero.jpg");
                let _ = std::fs::remove_file(&dest);
                let r = self.fetch_image(&cdn("library_hero_2x.jpg"), &dest);
                if r.is_empty() { self.fetch_image(&cdn("library_hero.jpg"), &dest) } else { r }
            }
            "grid" => {
                let dest = dir.join("library_600x900.jpg");
                let _ = std::fs::remove_file(&dest);
                let r = self.fetch_image(&cdn("library_600x900_2x.jpg"), &dest);
                if r.is_empty() { self.fetch_image(&cdn("library_600x900.jpg"), &dest) } else { r }
            }
            "header" => {
                let dest = dir.join("header.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&cdn("header.jpg"), &dest)
            }
            "logo" => {
                let dest = dir.join("logo.png");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&cdn("logo.png"), &dest)
            }
            _ => String::new(),
        }
    }

    pub fn force_download_sgdb(&self, id: &str, asset: &str, is_steam_id: bool) -> String {
        let dir = if is_steam_id { self.game_dir(id) } else { self.sgdb_dir(id) };
        let _ = std::fs::create_dir_all(&dir);
    let endpoint = match crate::sgdb::sgdb_endpoint(asset, is_steam_id, id) {
        Some(e) => e,
        None => return String::new(),
    };
        let dims: &[&str] = match asset {
            "grid" => &["600x900"],
            "header" => &["460x215", "920x430"],
            _ => &[],
        };
        let url = match self.fetch_sgdb_endpoint(&endpoint, dims) {
            Some(u) => u,
            None => return String::new(),
        };
        match asset {
            "icon" => {
                let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                let dest = dir.join(format!("icon.{}", ext));
                let _ = std::fs::remove_file(&dest);
                if self.download_file(&url, &dest).is_ok() {
                    let converted = ira_parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                    converted.to_string_lossy().into_owned()
                } else { String::new() }
            }
            "hero" => {
                let dest = dir.join("library_hero.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "grid" => {
                let dest = dir.join("library_600x900.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "header" => {
                let dest = dir.join("header.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "logo" => {
                let dest = dir.join("logo.png");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            _ => String::new(),
        }
    }
}
