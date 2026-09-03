use std::path::Path;

use crate::SteamDataClient;
use ira_models::{AssetType, DlcInfo};

/// Steam CDN base URL for app asset images.
fn steam_cdn_url(app_id: &str, suffix: &str) -> String {
    format!(
        "https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}",
        app_id, suffix
    )
}

/// Generate the `{base}_small` thumbnail variant for an asset.
fn shrink(dir: &Path, asset: AssetType) {
    let (w, h) = asset.thumb_dims();
    ira_parser::ensure_small_image(dir, asset.file_base(), w, h);
}

impl SteamDataClient {
    pub fn ensure_sgdb_assets(&self, sgdb_id: &str) -> (String, String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        self.ensure_sgdb_assets_in_dir(&dir, sgdb_id)
    }

    pub fn ensure_sgdb_assets_in_dir(
        &self,
        dir: &Path,
        sgdb_id: &str,
    ) -> (String, String, String, String, String, String) {
        let _s = tracing::info_span!("ensure_sgdb_assets_in_dir", sgdb_id).entered();
        let _ = std::fs::create_dir_all(dir);

        let mut icon_path = String::new();
        let mut hero_path = String::new();
        let mut grid_path = String::new();
        let mut logo_path = String::new();
        let mut header_path = String::new();
        let mut square_path = String::new();

        for (asset_type, path) in [
            (AssetType::Icon, &mut icon_path),
            (AssetType::Hero, &mut hero_path),
            (AssetType::Grid, &mut grid_path),
            (AssetType::Logo, &mut logo_path),
            (AssetType::Header, &mut header_path),
            (AssetType::Square, &mut square_path),
        ] {
            *path = if let Some(existing) = ira_parser::find_image_file(dir, asset_type.file_base())
            {
                let p = existing.to_string_lossy().into_owned();
                shrink(dir, asset_type);
                p
            } else {
                self.force_download_sgdb(dir, sgdb_id, asset_type, false)
            };
        }

        (icon_path, hero_path, grid_path, logo_path, header_path, square_path)
    }

    pub fn ensure_assets(&self, app_id: &str, has_local_icon: bool) -> (String, String) {
        let _s = tracing::info_span!("ensure_assets", app_id).entered();
        let dir = self.game_dir(app_id);

        let icon_path = if has_local_icon {
            String::new()
        } else {
            self.cached_or_fresh_icon(app_id, &dir)
        };
        let hero_path = self.cached_steam_asset(
            app_id,
            &dir,
            AssetType::Hero,
            "jpg",
            &["library_hero_2x.jpg", "library_hero.jpg"],
        );

        if !icon_path.is_empty() {
            shrink(&dir, AssetType::Icon);
        }
        if !hero_path.is_empty() {
            shrink(&dir, AssetType::Hero);
        }

        (icon_path, hero_path)
    }

    /// Serve the cached icon if present, else pull from SGDB. SGDB icon URLs
    /// can be any format (.ico included), so bytes are decoded fully in
    /// memory before anything touches disk.
    fn cached_or_fresh_icon(&self, app_id: &str, dir: &Path) -> String {
        if let Some(cached) = self.find_cached_icon(app_id) {
            return cached.to_string_lossy().into_owned();
        }
        let Some(url) = crate::sgdb::sgdb_endpoint(AssetType::Icon, true, app_id)
            .and_then(|endpoint| self.fetch_sgdb_endpoint(&endpoint, &[]))
        else {
            return String::new();
        };
        ira_parser::remove_image_variants(dir, AssetType::Icon.file_base());
        self.download_webp(dir, AssetType::Icon.file_base(), &url)
    }

    pub fn ensure_grids(&self, app_id: &str) -> (String, String, String) {
        let _s = tracing::info_span!("ensure_grids", app_id).entered();
        let dir = self.game_dir(app_id);

        let grid_path = self.cached_steam_asset(
            app_id,
            &dir,
            AssetType::Grid,
            "jpg",
            &["library_600x900_2x.jpg", "library_600x900.jpg"],
        );
        let header_path =
            self.cached_steam_asset(app_id, &dir, AssetType::Header, "jpg", &["header.jpg"]);
        let logo_path =
            self.cached_steam_asset(app_id, &dir, AssetType::Logo, "png", &["logo.png"]);

        for (asset, path) in [
            (AssetType::Grid, &grid_path),
            (AssetType::Header, &header_path),
            (AssetType::Logo, &logo_path),
        ] {
            if !path.is_empty() {
                shrink(&dir, asset);
            }
        }

        (grid_path, header_path, logo_path)
    }

    pub fn ensure_dlc_images(
        &self,
        app_id: &str,
        dlcs: &mut std::collections::HashMap<String, DlcInfo>,
    ) {
        let _s = tracing::info_span!("ensure_dlc_images", app_id).entered();
        let base_dir = self.game_dir(app_id);
        let dlc_dir = base_dir.join("dlc");
        let _ = std::fs::create_dir_all(&dlc_dir);

        for dlc in dlcs.values_mut() {
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
        match asset {
            AssetType::Hero => self.forced_steam_asset(
                app_id,
                &dir,
                asset,
                "jpg",
                &["library_hero_2x.jpg", "library_hero.jpg"],
            ),
            AssetType::Grid => self.forced_steam_asset(
                app_id,
                &dir,
                asset,
                "jpg",
                &["library_600x900_2x.jpg", "library_600x900.jpg"],
            ),
            AssetType::Header => {
                self.forced_steam_asset(app_id, &dir, asset, "jpg", &["header.jpg"])
            }
            AssetType::Logo => self.forced_steam_asset(app_id, &dir, asset, "png", &["logo.png"]),
            _ => String::new(),
        }
    }

    pub fn force_download_sgdb(
        &self,
        dir: &Path,
        id: &str,
        asset: AssetType,
        is_steam_id: bool,
    ) -> String {
        let _s = tracing::info_span!("force_download_sgdb", sgdb_id = id, asset = %asset).entered();
        let _ = std::fs::create_dir_all(dir);
        let endpoint = match crate::sgdb::sgdb_endpoint(asset, is_steam_id, id) {
            Some(e) => e,
            None => return String::new(),
        };
        let url = match self.fetch_sgdb_endpoint(&endpoint, asset.sgdb_dimensions()) {
            Some(u) => u,
            None => return String::new(),
        };

        let base_name = asset.file_base();
        ira_parser::remove_image_variants(dir, base_name);

        let r = if asset == AssetType::Icon {
            // Icon URLs frequently end in .ico; download_webp decodes fully
            // in memory so a bad dump never lands on disk.
            self.download_webp(dir, base_name, &url)
        } else {
            let ext = Path::new(&url)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png");
            let dest = dir.join(format!("{}.{}", base_name, ext));
            let is_png = ext.eq_ignore_ascii_case("png");
            let r = if is_png {
                if self.download_file(&url, &dest).is_ok() {
                    dest.to_string_lossy().into_owned()
                } else {
                    String::new()
                }
            } else {
                self.fetch_image(&url, &dest)
            };
            if r.is_empty() {
                r
            } else {
                ira_parser::convert_to_lossless_webp(&dest);
                let webp = dir.join(format!("{base_name}.webp"));
                if webp.is_file() {
                    webp.to_string_lossy().into_owned()
                } else {
                    r
                }
            }
        };

        if !r.is_empty() {
            shrink(dir, asset);
        }
        r
    }

    // ── shared Steam CDN / WebP flows ───────────────────────────────────

    /// Cached-first variant of [`Self::forced_steam_asset`]: returns the
    /// existing image variant for the asset's file base when one exists.
    fn cached_steam_asset(
        &self,
        app_id: &str,
        dir: &Path,
        asset: AssetType,
        dest_ext: &str,
        cdn_suffixes: &[&str],
    ) -> String {
        if let Some(existing) = ira_parser::find_image_file(dir, asset.file_base()) {
            return existing.to_string_lossy().into_owned();
        }
        self.fetch_steam_cdn_asset(app_id, dir, asset, dest_ext, cdn_suffixes)
    }

    /// Force a fresh download by removing existing variants first, then
    /// delegating to [`Self::fetch_steam_cdn_asset`].
    fn forced_steam_asset(
        &self,
        app_id: &str,
        dir: &Path,
        asset: AssetType,
        dest_ext: &str,
        cdn_suffixes: &[&str],
    ) -> String {
        ira_parser::remove_image_variants(dir, asset.file_base());
        self.fetch_steam_cdn_asset(app_id, dir, asset, dest_ext, cdn_suffixes)
    }

    /// Download from the Steam CDN onto `{file_base}.{dest_ext}`, trying each
    /// CDN suffix in order; convert to lossless WebP when possible, prefer the
    /// WebP result, and generate the thumbnail variant. Returns "" on total
    /// failure.
    fn fetch_steam_cdn_asset(
        &self,
        app_id: &str,
        dir: &Path,
        asset: AssetType,
        dest_ext: &str,
        cdn_suffixes: &[&str],
    ) -> String {
        let base = asset.file_base();
        let dest = dir.join(format!("{}.{}", base, dest_ext));

        let mut found = String::new();
        if dest.is_file() {
            found = dest.to_string_lossy().into_owned();
        } else {
            for suffix in cdn_suffixes {
                found = self.fetch_image(&steam_cdn_url(app_id, suffix), &dest);
                if !found.is_empty() {
                    break;
                }
            }
        }

        if found.is_empty() {
            return String::new();
        }
        ira_parser::convert_to_lossless_webp(&dest);

        let webp = dir.join(format!("{}.webp", base));
        let path = if webp.is_file() {
            webp.to_string_lossy().into_owned()
        } else {
            found
        };
        shrink(dir, asset);
        path
    }

    /// Download `url`, decode it fully in memory, and persist as lossless
    /// WebP at `{base_name}.webp`. Returns "" on any failure.
    fn download_webp(&self, dir: &Path, base_name: &str, url: &str) -> String {
        let dest_webp = dir.join(format!("{base_name}.webp"));
        match self
            .download_bytes(url)
            .ok()
            .filter(|bytes| ira_parser::is_decodable_image(bytes))
            .and_then(|bytes| ira_parser::convert_bytes_to_lossless_webp(&bytes))
        {
            Some(webp) if std::fs::write(&dest_webp, &webp).is_ok() => {
                dest_webp.to_string_lossy().into_owned()
            }
            _ => String::new(),
        }
    }
}
