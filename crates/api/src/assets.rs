use std::path::Path;

use crate::SteamDataClient;
use ira_models::{AssetType, DlcInfo};

impl SteamDataClient {
    pub fn ensure_sgdb_assets(&self, sgdb_id: &str) -> (String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        self.ensure_sgdb_assets_in_dir(&dir, sgdb_id)
    }

    pub fn ensure_sgdb_assets_in_dir(
        &self,
        dir: &Path,
        sgdb_id: &str,
    ) -> (String, String, String, String, String) {
        let _s = tracing::info_span!("ensure_sgdb_assets_in_dir", sgdb_id).entered();
        let _ = std::fs::create_dir_all(dir);

        let mut icon_path = String::new();
        let mut hero_path = String::new();
        let mut grid_path = String::new();
        let mut logo_path = String::new();
        let mut header_path = String::new();

        for (asset_type, path) in [
            (AssetType::Icon, &mut icon_path),
            (AssetType::Hero, &mut hero_path),
            (AssetType::Grid, &mut grid_path),
            (AssetType::Logo, &mut logo_path),
            (AssetType::Header, &mut header_path),
        ] {
            // Older icon downloads could leave an undecoded .ico behind;
            // convert or drop it so it stops shadowing fresh downloads.
            ira_parser::heal_ico_variant(dir, asset_type.file_base());
            *path = if let Some(existing) = ira_parser::find_image_file(dir, asset_type.file_base())
            {
                let p = existing.to_string_lossy().into_owned();
                ira_parser::ensure_small_image(
                    dir,
                    asset_type.file_base(),
                    asset_type.thumb_dims().0,
                    asset_type.thumb_dims().1,
                );
                p
            } else {
                self.force_download_sgdb(dir, sgdb_id, asset_type, false)
            };
        }

        (icon_path, hero_path, grid_path, logo_path, header_path)
    }

    pub fn ensure_assets(&self, app_id: &str, has_local_icon: bool) -> (String, String) {
        let _s = tracing::info_span!("ensure_assets", app_id).entered();
        let dir = self.game_dir(app_id);

        let icon_path = if has_local_icon {
            String::new()
        } else {
            let mut found = String::new();
            ira_parser::heal_ico_variant(&dir, AssetType::Icon.file_base());
            if let Some(cached) = self.find_cached_icon(app_id) {
                found = cached.to_string_lossy().into_owned();
            }
            if found.is_empty() {
                if let Some(url) = self.fetch_sgdb_icon_url(app_id) {
                    ira_parser::remove_image_variants(&dir, AssetType::Icon.file_base());
                    // Download as bytes and decode before anything touches
                    // disk: an icon URL can serve anything, and a raw dump
                    // used to strand icon.ico files that never converted.
                    let dest_webp = dir.join(format!("{}.webp", AssetType::Icon.file_base()));
                    if let Some(webp) = self
                        .download_bytes(&url)
                        .ok()
                        .filter(|bytes| ira_parser::is_decodable_image(bytes))
                        .and_then(|bytes| ira_parser::convert_bytes_to_lossless_webp(&bytes))
                    {
                        if std::fs::write(&dest_webp, &webp).is_ok() {
                            found = dest_webp.to_string_lossy().into_owned();
                        }
                    }
                }
            }
            found
        };

        let hero_path = if let Some(cached) = self.find_cached_hero(app_id) {
            cached.to_string_lossy().into_owned()
        } else {
            let dest = dir.join(format!("{}.jpg", AssetType::Hero.file_base()));
            let r = self.fetch_image_fallback(
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero_2x.jpg", app_id),
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", app_id),
                &dest,
            );
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join(format!("{}.webp", AssetType::Hero.file_base()));
            if webp.is_file() {
                webp.to_string_lossy().into_owned()
            } else {
                r
            }
        };

        if !icon_path.is_empty() {
            ira_parser::ensure_small_image(
                &dir,
                AssetType::Icon.file_base(),
                AssetType::Icon.thumb_dims().0,
                AssetType::Icon.thumb_dims().1,
            );
        }
        if !hero_path.is_empty() {
            ira_parser::ensure_small_image(
                &dir,
                AssetType::Hero.file_base(),
                AssetType::Hero.thumb_dims().0,
                AssetType::Hero.thumb_dims().1,
            );
        }

        (icon_path, hero_path)
    }

    pub fn ensure_grids(&self, app_id: &str) -> (String, String, String) {
        let _s = tracing::info_span!("ensure_grids", app_id).entered();
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| {
            format!(
                "https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}",
                app_id, suffix
            )
        };

        let grid_path = if let Some(existing) =
            ira_parser::find_image_file(&dir, AssetType::Grid.file_base())
        {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join(format!("{}.jpg", AssetType::Grid.file_base()));
            let r = self.fetch_image_fallback(
                &cdn("library_600x900_2x.jpg"),
                &cdn("library_600x900.jpg"),
                &dest,
            );
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join(format!("{}.webp", AssetType::Grid.file_base()));
            if webp.is_file() {
                webp.to_string_lossy().into_owned()
            } else {
                r
            }
        };

        let header_path = if let Some(existing) =
            ira_parser::find_image_file(&dir, AssetType::Header.file_base())
        {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join(format!("{}.jpg", AssetType::Header.file_base()));
            let r = self.fetch_image_fallback(&cdn("header.jpg"), "", &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join(format!("{}.webp", AssetType::Header.file_base()));
            if webp.is_file() {
                webp.to_string_lossy().into_owned()
            } else {
                r
            }
        };

        let logo_path = if let Some(existing) =
            ira_parser::find_image_file(&dir, AssetType::Logo.file_base())
        {
            existing.to_string_lossy().into_owned()
        } else {
            let dest = dir.join(format!("{}.png", AssetType::Logo.file_base()));
            let r = self.fetch_image_fallback(&cdn("logo.png"), "", &dest);
            if !r.is_empty() {
                ira_parser::convert_to_lossless_webp(&dest);
            }
            let webp = dir.join(format!("{}.webp", AssetType::Logo.file_base()));
            if webp.is_file() {
                webp.to_string_lossy().into_owned()
            } else {
                r
            }
        };

        if !grid_path.is_empty() {
            ira_parser::ensure_small_image(
                &dir,
                AssetType::Grid.file_base(),
                AssetType::Grid.thumb_dims().0,
                AssetType::Grid.thumb_dims().1,
            );
        }
        if !header_path.is_empty() {
            ira_parser::ensure_small_image(
                &dir,
                AssetType::Header.file_base(),
                AssetType::Header.thumb_dims().0,
                AssetType::Header.thumb_dims().1,
            );
        }
        if !logo_path.is_empty() {
            ira_parser::ensure_small_image(
                &dir,
                AssetType::Logo.file_base(),
                AssetType::Logo.thumb_dims().0,
                AssetType::Logo.thumb_dims().1,
            );
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
        let cdn = |suffix: &str| {
            format!(
                "https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}",
                app_id, suffix
            )
        };
        match asset {
            AssetType::Hero => {
                ira_parser::remove_image_variants(&dir, AssetType::Hero.file_base());
                let dest = dir.join("hero.jpg");
                let r = self.fetch_image(&cdn("library_hero_2x.jpg"), &dest);
                let r = if r.is_empty() {
                    self.fetch_image(&cdn("library_hero.jpg"), &dest)
                } else {
                    r
                };
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("hero.webp");
                    let r = if webp.is_file() {
                        webp.to_string_lossy().into_owned()
                    } else {
                        r
                    };
                    ira_parser::ensure_small_image(
                        &dir,
                        AssetType::Hero.file_base(),
                        AssetType::Hero.thumb_dims().0,
                        AssetType::Hero.thumb_dims().1,
                    );
                    r
                } else {
                    r
                }
            }
            AssetType::Grid => {
                ira_parser::remove_image_variants(&dir, AssetType::Grid.file_base());
                let dest = dir.join("vertical.jpg");
                let r = self.fetch_image(&cdn("library_600x900_2x.jpg"), &dest);
                let r = if r.is_empty() {
                    self.fetch_image(&cdn("library_600x900.jpg"), &dest)
                } else {
                    r
                };
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("vertical.webp");
                    let r = if webp.is_file() {
                        webp.to_string_lossy().into_owned()
                    } else {
                        r
                    };
                    ira_parser::ensure_small_image(
                        &dir,
                        AssetType::Grid.file_base(),
                        AssetType::Grid.thumb_dims().0,
                        AssetType::Grid.thumb_dims().1,
                    );
                    r
                } else {
                    r
                }
            }
            AssetType::Header => {
                ira_parser::remove_image_variants(&dir, AssetType::Header.file_base());
                let dest = dir.join("header.jpg");
                let r = self.fetch_image(&cdn("header.jpg"), &dest);
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("header.webp");
                    let r = if webp.is_file() {
                        webp.to_string_lossy().into_owned()
                    } else {
                        r
                    };
                    ira_parser::ensure_small_image(
                        &dir,
                        AssetType::Header.file_base(),
                        AssetType::Header.thumb_dims().0,
                        AssetType::Header.thumb_dims().1,
                    );
                    r
                } else {
                    r
                }
            }
            AssetType::Logo => {
                ira_parser::remove_image_variants(&dir, AssetType::Logo.file_base());
                let dest = dir.join("logo.png");
                let r = self.fetch_image(&cdn("logo.png"), &dest);
                if !r.is_empty() {
                    ira_parser::convert_to_lossless_webp(&dest);
                    let webp = dir.join("logo.webp");
                    let r = if webp.is_file() {
                        webp.to_string_lossy().into_owned()
                    } else {
                        r
                    };
                    ira_parser::ensure_small_image(
                        &dir,
                        AssetType::Logo.file_base(),
                        AssetType::Logo.thumb_dims().0,
                        AssetType::Logo.thumb_dims().1,
                    );
                    r
                } else {
                    r
                }
            }
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
        let dims: &[&str] = match asset {
            AssetType::Grid => &["600x900"],
            AssetType::Header => &["460x215", "920x430"],
            _ => &[],
        };
        let url = match self.fetch_sgdb_endpoint(&endpoint, dims) {
            Some(u) => u,
            None => return String::new(),
        };

        let base_name = asset.file_base();
        ira_parser::remove_image_variants(dir, base_name);

        let r = if asset == AssetType::Icon {
            // Icons: decode in memory and write WebP directly. Icon URLs
            // frequently end in .ico and a raw dump used to strand files
            // that never converted.
            let dest_webp = dir.join(format!("{base_name}.webp"));
            match self
                .download_bytes(&url)
                .ok()
                .filter(|bytes| ira_parser::is_decodable_image(bytes))
                .and_then(|bytes| ira_parser::convert_bytes_to_lossless_webp(&bytes))
            {
                Some(webp) if std::fs::write(&dest_webp, &webp).is_ok() => {
                    dest_webp.to_string_lossy().into_owned()
                }
                _ => String::new(),
            }
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
            let (sw, sh) = asset.thumb_dims();
            ira_parser::ensure_small_image(dir, base_name, sw, sh);
        }
        r
    }
}
