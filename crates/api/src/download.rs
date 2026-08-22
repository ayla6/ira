use std::path::{Path, PathBuf};

use crate::util::MIN_IMAGE_BYTES;
use crate::SteamDataClient;

impl SteamDataClient {
    pub(super) fn game_dir(&self, app_id: &str) -> PathBuf {
        self.cache_dir.join("steam").join(app_id)
    }

    pub(super) fn sgdb_dir(&self, sgdb_id: &str) -> PathBuf {
        self.cache_dir.join("steamgriddb").join(sgdb_id)
    }

    pub(super) fn fetch_image(&self, url: &str, dest: &Path) -> String {
        let _s = tracing::info_span!("fetch_image", url).entered();
        if self.download_file(url, dest).is_ok() {
            if let Ok(meta) = std::fs::metadata(dest) {
                if meta.len() >= MIN_IMAGE_BYTES {
                    return dest.to_string_lossy().into_owned();
                }
                let _ = std::fs::remove_file(dest);
            }
        }
        String::new()
    }

    pub(super) fn fetch_image_fallback(
        &self,
        primary: &str,
        fallback: &str,
        dest: &Path,
    ) -> String {
        let _s = tracing::info_span!("fetch_image_fallback", url = primary).entered();
        if dest.exists() {
            return dest.to_string_lossy().into_owned();
        }
        let found = self.fetch_image(primary, dest);
        if found.is_empty() && !fallback.is_empty() {
            self.fetch_image(fallback, dest)
        } else {
            found
        }
    }

    pub fn download_file(&self, url: &str, dest: &Path) -> Result<(), String> {
        let _s = tracing::info_span!("download_file", url, dest = %dest.display()).entered();
        std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")))
            .map_err(|e| e.to_string())?;
        let resp = self.http.get(url).send().map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        std::fs::write(dest, &bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn download_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let _s = tracing::info_span!("download_bytes", url).entered();
        let resp = self.http.get(url).send().map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        Ok(bytes.to_vec())
    }

    pub(super) fn find_cached_icon(&self, app_id: &str) -> Option<PathBuf> {
        let dir = self.game_dir(app_id);
        // .ico is deliberately absent: icons are converted on download and
        // stale .ico files are healed away, never served.
        for ext in [".webp", ".png", ".jpg"] {
            let path = dir.join(format!("icon{}", ext));
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    pub(super) fn find_cached_hero(&self, app_id: &str) -> Option<PathBuf> {
        ira_parser::find_image_file(&self.game_dir(app_id), "hero")
    }
}
