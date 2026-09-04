use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ira_models::AssetType;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct SteamDataClient {
    pub(crate) api_key: Mutex<String>,
    pub(crate) sgdb_api_key: Mutex<String>,
    pub(crate) cache_dir: PathBuf,
    pub(crate) http: reqwest::blocking::Client,
    /// Bit per [`AssetType`]: set = auto-download allowed. Defaults to all
    /// enabled; the settings page rebuilds it from the disabled list.
    pub(crate) sgdb_auto_mask: AtomicU64,
    /// SGDB authors whose art is skipped by automatic downloads and sunk to
    /// the bottom of manual pickers.
    pub(crate) sgdb_filtered_users: Mutex<Vec<String>>,
    /// SGDB styles ("blurred", "white_logo", ...) treated the same way.
    pub(crate) sgdb_filtered_styles: Mutex<Vec<String>>,
}

impl SteamDataClient {
    pub fn new(api_key: String, sgdb_key: String, data_dir: &str) -> Self {
        SteamDataClient {
            api_key: Mutex::new(api_key),
            sgdb_api_key: Mutex::new(sgdb_key),
            cache_dir: PathBuf::from(data_dir),
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .user_agent("Ira/0.1 (https://github.com/ayla6/ira)")
                .build()
                .expect("failed to build HTTP client"),
            sgdb_auto_mask: AtomicU64::new(u64::MAX),
            sgdb_filtered_users: Mutex::new(Vec::new()),
            sgdb_filtered_styles: Mutex::new(Vec::new()),
        }
    }

    pub fn update_keys(&self, api_key: &str, sgdb_key: &str) {
        *self.api_key.lock().unwrap() = api_key.to_string();
        *self.sgdb_api_key.lock().unwrap() = sgdb_key.to_string();
    }

    /// Disable auto-downloads for the named SGDB asset types ("logo",
    /// "square", ...). Absent names stay enabled.
    pub fn set_sgdb_disabled_assets(&self, disabled: &[String]) {
        let mut mask = 0u64;
        for name in disabled {
            if let Some(asset) = AssetType::from_string(name) {
                mask |= sgdb_asset_bit(asset);
            }
        }
        self.sgdb_auto_mask.store(!mask, Ordering::Relaxed);
    }

    pub(crate) fn sgdb_auto_enabled(&self, asset: AssetType) -> bool {
        self.sgdb_auto_mask.load(Ordering::Relaxed) & sgdb_asset_bit(asset) != 0
    }

    /// Replace the list of SGDB authors whose art is skipped by automatic
    /// downloads and sunk to the bottom of manual pickers, as `(name,
    /// steam64)` pairs. Only the name matters here; matched
    /// case-insensitively, blank names ignored.
    pub fn set_sgdb_filtered_users(&self, users: &[(String, String)]) {
        *self.sgdb_filtered_users.lock().unwrap() = users
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !name.trim().is_empty())
            .collect();
    }

    pub(crate) fn sgdb_filtered_users(&self) -> Vec<String> {
        self.sgdb_filtered_users.lock().unwrap().clone()
    }

    pub(crate) fn user_filtered(&self, author: &str) -> bool {
        self.sgdb_filtered_users
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.eq_ignore_ascii_case(author))
    }

    /// Replace the list of SGDB image styles ("blurred", "white_logo", ...)
    /// excluded from automatic downloads and sunk to the bottom of manual
    /// pickers. Matched case-insensitively.
    pub fn set_sgdb_filtered_styles(&self, styles: &[String]) {
        *self.sgdb_filtered_styles.lock().unwrap() = styles.to_vec();
    }

    pub(crate) fn sgdb_filtered_styles(&self) -> Vec<String> {
        self.sgdb_filtered_styles.lock().unwrap().clone()
    }

    pub(crate) fn style_filtered(&self, style: &str) -> bool {
        self.sgdb_filtered_styles
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.eq_ignore_ascii_case(style))
    }

    /// True when assets by this author or in this style are excluded from
    /// automatic downloads and sunk to the bottom of manual pickers.
    pub fn asset_filtered(&self, author: &str, style: &str) -> bool {
        self.user_filtered(author) || self.style_filtered(style)
    }

    pub(crate) fn api_key(&self) -> String {
        self.api_key.lock().unwrap().clone()
    }

    pub(crate) fn sgdb_api_key(&self) -> String {
        self.sgdb_api_key.lock().unwrap().clone()
    }

    /// True when a SteamGridDB key is configured, so SGDB flows can skip
    /// pointless network calls.
    pub fn has_sgdb_key(&self) -> bool {
        !self.sgdb_api_key().is_empty()
    }

    /// GET `url` and return the response body as text. Errors include the URL
    /// and HTTP status so callers can log without reassembling context.
    pub(crate) fn http_get_text(&self, url: &str) -> Result<String, String> {
        let _s = tracing::info_span!("http_get_text", url).entered();
        let resp = self
            .http
            .get(url)
            .send()
            .map_err(|e| format!("GET {url} failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("GET {url} failed: HTTP {status}"));
        }
        resp.text()
            .map_err(|e| format!("GET {url} failed: read error: {e}"))
    }

    /// GET `url` and decode the body as JSON. Failures are logged with the
    /// URL (and status where applicable); returns `None` on any failure.
    pub(crate) fn http_get_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        let _s = tracing::info_span!("http_get_json", url).entered();
        let text = match self.http_get_text(url) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e}");
                return None;
            }
        };
        match serde_json::from_str(&text) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("GET {url} failed: decode error: {e}");
                None
            }
        }
    }
}

/// One bit per SGDB-downloadable asset type, for the auto-download mask.
fn sgdb_asset_bit(asset: AssetType) -> u64 {
    match asset {
        AssetType::Icon => 1 << 0,
        AssetType::Hero => 1 << 1,
        AssetType::Grid => 1 << 2,
        AssetType::Header => 1 << 3,
        AssetType::Logo => 1 << 4,
        AssetType::Square => 1 << 5,
    }
}
