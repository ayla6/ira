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
    pub fn ensure_sgdb_assets(
        &self,
        sgdb_id: &str,
    ) -> (String, String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        self.ensure_sgdb_assets_in_dir(&dir, crate::types::SgdbId::Game(sgdb_id), &[])
    }

    /// Fetch every enabled SGDB asset type into `dir` (cached files are
    /// reused). Types in `skip` are not touched at all — used to keep the
    /// square slot native for console games. `id` picks the endpoint
    /// family: an SGDB id or a Steam app id.
    pub fn ensure_sgdb_assets_in_dir(
        &self,
        dir: &Path,
        id: crate::types::SgdbId,
        skip: &[AssetType],
    ) -> (String, String, String, String, String, String) {
        let _s = tracing::info_span!("ensure_sgdb_assets_in_dir", id = %id.as_str()).entered();
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
            if skip.contains(&asset_type) {
                continue;
            }
            // Disabled categories keep whatever is already on disk but are
            // never fetched; existing files are returned either way.
            if !self.sgdb_auto_enabled(asset_type) {
                continue;
            }
            *path = if let Some(existing) = ira_parser::find_image_file(dir, asset_type.file_base())
            {
                let p = existing.to_string_lossy().into_owned();
                shrink(dir, asset_type);
                p
            } else {
                self.force_download_sgdb(dir, id, asset_type)
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
        let candidates = crate::sgdb::sgdb_endpoint(AssetType::Icon, true, app_id)
            .map(|endpoint| self.fetch_sgdb_candidates(&endpoint, &[]))
            .unwrap_or_default();
        ira_parser::remove_image_variants(dir, AssetType::Icon.file_base());
        for url in candidates {
            let path = self.download_webp(dir, AssetType::Icon.file_base(), &url);
            if !path.is_empty() {
                return path;
            }
        }
        String::new()
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

    pub fn force_download_sgdb(&self, dir: &Path, id: crate::types::SgdbId, asset: AssetType) -> String {
        let _s = tracing::info_span!("force_download_sgdb", sgdb_id = id.as_str(), asset = %asset)
            .entered();
        let _ = std::fs::create_dir_all(dir);
        let endpoint = match crate::sgdb::sgdb_endpoint(asset, id.is_steam(), id.as_str()) {
            Some(e) => e,
            None => return String::new(),
        };
        let candidates = self.fetch_sgdb_candidates(&endpoint, asset.sgdb_dimensions());
        if candidates.is_empty() {
            return String::new();
        }

        let base_name = asset.file_base();
        ira_parser::remove_image_variants(dir, base_name);

        // Try candidates in SGDB's popularity order, rejecting DMCA takedown
        // notice cards before anything lands on disk.
        for url in candidates {
            let Ok(bytes) = self.download_bytes(&url) else {
                continue;
            };
            if ira_parser::is_dmca_placeholder(&bytes) {
                continue;
            }
            let path = self.persist_sgdb_bytes(dir, base_name, &url, asset, &bytes);
            if !path.is_empty() {
                shrink(dir, asset);
                return path;
            }
        }
        String::new()
    }

    /// Persist already-validated image bytes under `{base_name}`: icons are
    /// re-encoded to lossless WebP in memory so a bad dump (.ico payloads
    /// included) never lands on disk; other assets keep their original
    /// extension and are converted in place afterwards.
    fn persist_sgdb_bytes(
        &self,
        dir: &Path,
        base_name: &str,
        url: &str,
        asset: AssetType,
        bytes: &[u8],
    ) -> String {
        if asset == AssetType::Icon {
            return self.store_image_bytes(dir, base_name, bytes);
        }
        let ext = Path::new(url)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png");
        let dest = dir.join(format!("{}.{}", base_name, ext));
        if std::fs::write(&dest, bytes).is_err() {
            return String::new();
        }
        ira_parser::convert_to_lossless_webp(&dest);
        let webp = dir.join(format!("{base_name}.webp"));
        if webp.is_file() {
            webp.to_string_lossy().into_owned()
        } else {
            dest.to_string_lossy().into_owned()
        }
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
            .filter(|bytes| !ira_parser::is_dmca_placeholder(bytes))
            .and_then(|bytes| ira_parser::convert_bytes_to_lossless_webp(&bytes))
        {
            Some(webp) if std::fs::write(&dest_webp, &webp).is_ok() => {
                dest_webp.to_string_lossy().into_owned()
            }
            _ => String::new(),
        }
    }

    /// Re-encode already-downloaded image bytes as lossless WebP at
    /// `{base_name}.webp` in `dir`.
    fn store_image_bytes(&self, dir: &Path, base_name: &str, bytes: &[u8]) -> String {
        let dest_webp = dir.join(format!("{base_name}.webp"));
        match ira_parser::convert_bytes_to_lossless_webp(bytes) {
            Some(webp) if std::fs::write(&dest_webp, &webp).is_ok() => {
                dest_webp.to_string_lossy().into_owned()
            }
            _ => String::new(),
        }
    }
}
