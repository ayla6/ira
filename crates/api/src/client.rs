use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::de::DeserializeOwned;

pub struct SteamDataClient {
    pub(crate) api_key: Mutex<String>,
    pub(crate) sgdb_api_key: Mutex<String>,
    pub(crate) cache_dir: PathBuf,
    pub(crate) http: reqwest::blocking::Client,
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
        }
    }

    pub fn update_keys(&self, api_key: &str, sgdb_key: &str) {
        *self.api_key.lock().unwrap() = api_key.to_string();
        *self.sgdb_api_key.lock().unwrap() = sgdb_key.to_string();
    }

    pub(crate) fn api_key(&self) -> String {
        self.api_key.lock().unwrap().clone()
    }

    pub(crate) fn sgdb_api_key(&self) -> String {
        self.sgdb_api_key.lock().unwrap().clone()
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
