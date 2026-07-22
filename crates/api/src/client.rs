use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

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
}
