use std::path::Path;

use crate::SteamDataClient;
use ira_models::{AssetType, DlcInfo};

impl SteamDataClient {
    pub fn ensure_sgdb_assets(&self, sgdb_id: &str) -> (String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        self.ensure_sgdb_assets_in_dir(&dir, sgdb_id)
    }

    pub fn ensure_sgdb_assets_in_dir(&self, dir: &Path, sgdb_id: &str) -> (String, String, String, String, String) {
        let _s = tracing::info_span!("ensure_sgdb_assets_in_dir", sgdb_id).entered();
        let _ = std::fs::create_dir_all(dir);

        let icon_path = if let Some(existing) = ira_parser::find_image_file(dir, "icon") {
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
                                        ira_parser::remove_image_variants(dir, "icon");
                                        let ext = ira_parser::url_extension(url);
                                        let dest = dir.join(format!("icon.{}", ext));
                                        if self.download_file(url, &dest).is_ok() {
                                            ira_parser::convert_to_lossless_webp(&dest);
                                            let webp = dir.join("icon.webp");
                                            if webp.is_file() { webp.to_string_lossy().into_owned() }
                                            else { dest.to_string_lossy().into_owned() }
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

        let hero_path = if let Some(existing) = ira_parser::find_image_file(dir, "hero") {
            existing.to_string_lossy().into_owned()
        } else if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "heroes") {
            ira_parser::remove_image_variants(dir, "hero");
            let ext = ira_parser::url_extension(&url);
            let dest = dir.join(format!("hero.{}", ext));
            let r = self.fetch_image(&url, &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("hero.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        } else { String::new() };

        let grid_path = if let Some(existing) = ira_parser::find_image_file(dir, "vertical") {
            existing.to_string_lossy().into_owned()
        } else {
            self.force_download_sgdb(dir, sgdb_id, AssetType::Grid, false)
        };

        let logo_path = if let Some(existing) = ira_parser::find_image_file(dir, "logo") {
            existing.to_string_lossy().into_owned()
        } else if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "logos") {
            ira_parser::remove_image_variants(dir, "logo");
            let ext = ira_parser::url_extension(&url);
            let dest = dir.join(format!("logo.{}", ext));
            let r = self.fetch_image(&url, &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("logo.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        } else { String::new() };

        let header_path = if let Some(existing) = ira_parser::find_image_file(dir, "header") {
            existing.to_string_lossy().into_owned()
        } else {
            self.force_download_sgdb(dir, sgdb_id, AssetType::Header, false)
        };

        if !icon_path.is_empty() { ira_parser::ensure_small_image(dir, AssetType::Icon.file_base(), AssetType::Icon.thumb_dims().0, AssetType::Icon.thumb_dims().1); }
        if !hero_path.is_empty() { ira_parser::ensure_small_image(dir, AssetType::Hero.file_base(), AssetType::Hero.thumb_dims().0, AssetType::Hero.thumb_dims().1); }
        if !grid_path.is_empty() { ira_parser::ensure_small_image(dir, AssetType::Grid.file_base(), AssetType::Grid.thumb_dims().0, AssetType::Grid.thumb_dims().1); }
        if !header_path.is_empty() { ira_parser::ensure_small_image(dir, AssetType::Header.file_base(), AssetType::Header.thumb_dims().0, AssetType::Header.thumb_dims().1); }
        if !logo_path.is_empty() { ira_parser::ensure_small_image(dir, AssetType::Logo.file_base(), AssetType::Logo.thumb_dims().0, AssetType::Logo.thumb_dims().1); }

        (icon_path, hero_path, grid_path, logo_path, header_path)
    }

    pub fn ensure_assets(
        &self,
        app_id: &str,
        has_local_icon: bool,
    ) -> (String, String) {
        let _s = tracing::info_span!("ensure_assets", app_id).entered();
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
                    ira_parser::remove_image_variants(&dir, "icon");
                    let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                    let dest = dir.join(format!("icon.{}", ext));
                    if self.download_file(&url, &dest).is_ok() {
                        ira_parser::convert_to_lossless_webp(&dest);
                        let webp = dir.join("icon.webp");
                        found = if webp.is_file() { webp.to_string_lossy().into_owned() }
                            else { dest.to_string_lossy().into_owned() };
                    }
                }
            }
            found
        };

        let hero_path = if let Some(cached) = self.find_cached_hero(app_id) {
            cached.to_string_lossy().into_owned()
        } else {
            let dest = dir.join("hero.jpg");
            let r = self.fetch_image_fallback(
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero_2x.jpg", app_id),
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", app_id),
                &dest,
            );
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("hero.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        };

        if !icon_path.is_empty() { ira_parser::ensure_small_image(&dir, AssetType::Icon.file_base(), AssetType::Icon.thumb_dims().0, AssetType::Icon.thumb_dims().1); }
        if !hero_path.is_empty() { ira_parser::ensure_small_image(&dir, AssetType::Hero.file_base(), AssetType::Hero.thumb_dims().0, AssetType::Hero.thumb_dims().1); }
 
        (icon_path, hero_path)
    }

    pub fn ensure_grids(&self, app_id: &str) -> (String, String, String) {
        let _s = tracing::info_span!("ensure_grids", app_id).entered();
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| {
            format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix)
        };

        let grid_path = if let Some(existing) = ira_parser::find_image_file(&dir, "vertical") {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join("vertical.jpg");
            let r = self.fetch_image_fallback(&cdn("library_600x900_2x.jpg"), &cdn("library_600x900.jpg"), &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("vertical.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        };

        let header_path = if let Some(existing) = ira_parser::find_image_file(&dir, "header") {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join("header.jpg");
            let r = self.fetch_image_fallback(&cdn("header.jpg"), "", &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("header.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        };

        let logo_path = if let Some(existing) = ira_parser::find_image_file(&dir, "logo") {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join("logo.png");
            let r = self.fetch_image_fallback(&cdn("logo.png"), "", &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join("logo.webp");
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        };

        if !grid_path.is_empty() { ira_parser::ensure_small_image(&dir, AssetType::Grid.file_base(), AssetType::Grid.thumb_dims().0, AssetType::Grid.thumb_dims().1); }
        if !header_path.is_empty() { ira_parser::ensure_small_image(&dir, AssetType::Header.file_base(), AssetType::Header.thumb_dims().0, AssetType::Header.thumb_dims().1); }
        if !logo_path.is_empty() { ira_parser::ensure_small_image(&dir, AssetType::Logo.file_base(), AssetType::Logo.thumb_dims().0, AssetType::Logo.thumb_dims().1); }

        (grid_path, header_path, logo_path)
    }

    pub fn ensure_dlc_images(&self, app_id: &str, dlcs: &mut std::collections::HashMap<String, DlcInfo>) {
        let _s = tracing::info_span!("ensure_dlc_images", app_id).entered();
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

    pub fn force_download_steam(&self, app_id: &str, asset: AssetType) -> String {
        let _s = tracing::info_span!("force_download_steam", app_id, asset = %asset).entered();
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix);
        match asset {
            AssetType::Hero => {
                ira_parser::remove_image_variants(&dir, AssetType::Hero.file_base());
                let dest = dir.join("hero.jpg");
                let r = self.fetch_image(&cdn("library_hero_2x.jpg"), &dest);
                let r = if r.is_empty() { self.fetch_image(&cdn("library_hero.jpg"), &dest) } else { r };
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("hero.webp");
                    let r = if webp.is_file() { webp.to_string_lossy().into_owned() } else { r };
                    ira_parser::ensure_small_image(&dir, AssetType::Hero.file_base(), AssetType::Hero.thumb_dims().0, AssetType::Hero.thumb_dims().1);
                    r
                } else { r }
            }
            AssetType::Grid => {
                ira_parser::remove_image_variants(&dir, AssetType::Grid.file_base());
                let dest = dir.join("vertical.jpg");
                let r = self.fetch_image(&cdn("library_600x900_2x.jpg"), &dest);
                let r = if r.is_empty() { self.fetch_image(&cdn("library_600x900.jpg"), &dest) } else { r };
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("vertical.webp");
                    let r = if webp.is_file() { webp.to_string_lossy().into_owned() } else { r };
                    ira_parser::ensure_small_image(&dir, AssetType::Grid.file_base(), AssetType::Grid.thumb_dims().0, AssetType::Grid.thumb_dims().1);
                    r
                } else { r }
            }
            AssetType::Header => {
                ira_parser::remove_image_variants(&dir, AssetType::Header.file_base());
                let dest = dir.join("header.jpg");
                let r = self.fetch_image(&cdn("header.jpg"), &dest);
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("header.webp");
                    let r = if webp.is_file() { webp.to_string_lossy().into_owned() } else { r };
                    ira_parser::ensure_small_image(&dir, AssetType::Header.file_base(), AssetType::Header.thumb_dims().0, AssetType::Header.thumb_dims().1);
                    r
                } else { r }
            }
            AssetType::Logo => {
                ira_parser::remove_image_variants(&dir, AssetType::Logo.file_base());
                let dest = dir.join("logo.png");
                let r = self.fetch_image(&cdn("logo.png"), &dest);
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("logo.webp");
                    let r = if webp.is_file() { webp.to_string_lossy().into_owned() } else { r };
                    ira_parser::ensure_small_image(&dir, AssetType::Logo.file_base(), AssetType::Logo.thumb_dims().0, AssetType::Logo.thumb_dims().1);
                    r
                } else { r }
            }
            _ => String::new(),
        }
    }

    pub fn force_download_sgdb(&self, dir: &Path, id: &str, asset: AssetType, is_steam_id: bool) -> String {
        let _s = tracing::info_span!("force_download_sgdb", sgdb_id = id, asset = %asset).entered();
        let _ = std::fs::create_dir_all(dir);
    let endpoint = match crate::sgdb::sgdb_endpoint(asset, is_steam_id, id) {
        Some(e) => e,
        None => return String::new(),
    };
        let dims: &[&str] = match asset {
            AssetType::Grid => &["600x900"],
            AssetType::Header => &["460x215", "920x430"],
            _ => &[],
        };
        let url = match self.fetch_sgdb_endpoint(&endpoint, dims) {
            Some(u) => u,
            None => return String::new(),
        };
        let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
        let is_png = ext.eq_ignore_ascii_case("png");

        let base_name = asset.file_base();
        ira_parser::remove_image_variants(dir, base_name);

        let dest = dir.join(format!("{}.{}", base_name, ext));
        let r = if is_png || asset == AssetType::Icon {
            if self.download_file(&url, &dest).is_ok() {
                dest.to_string_lossy().into_owned()
            } else { String::new() }
        } else {
            self.fetch_image(&url, &dest)
        };
        let r = if !r.is_empty() {
            ira_parser::convert_to_lossless_webp(&dest);
            let webp = dir.join(format!("{}.webp", base_name));
            if webp.is_file() { webp.to_string_lossy().into_owned() } else { r }
        } else { r };

        if !r.is_empty() {
            let (sw, sh) = asset.thumb_dims();
            ira_parser::ensure_small_image(dir, base_name, sw, sh);
        }
        r
    }
}
