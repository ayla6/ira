use std::path::Path;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use tracing::info_span;

use ira_config::Config;
use crate::retroachievements::paths;

use super::api_types::{ConsoleGamesResponse, GameDataResponse, LoginResponse, UnlocksResponse};
pub use super::api_types::{RaGameEntry, RaGameData, RaAchievementDef, build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache, redownload_missing_ra_badges};
pub use super::api_types::read_console_games_cache;

const RA_BASE_URL: &str = "https://retroachievements.org/dorequest.php";
const RA_BADGE_URL: &str = "https://media.retroachievements.org/Badge";
const UNLOCKS_CACHE_SECS: u64 = 3600;
const RA_RATE_LIMIT_MS: u64 = 500;
const RA_MAX_AUTH_FAILURES: u32 = 3;

static RA_AUTH_BROKEN: AtomicBool = AtomicBool::new(false);
static RA_AUTH_FAILURES: AtomicU32 = AtomicU32::new(0);
static RA_CACHED_TOKEN: Mutex<Option<String>> = Mutex::new(None);

pub struct RaClient {
    http: reqwest::blocking::Client,
    username: String,
    token: String,
    last_request: Mutex<std::time::Instant>,
}

impl RaClient {
    pub fn new(username: &str, token: &str, password: &str) -> Self {
        let mut client = RaClient {
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("failed to build RA HTTP client"),
            username: username.to_string(),
            token: token.to_string(),
            last_request: Mutex::new(std::time::Instant::now() - Duration::from_secs(1)),
        };

        {
            let cached = RA_CACHED_TOKEN.lock().unwrap();
            if let Some(ref t) = *cached {
                client.token = t.clone();
                return client;
            }
        }

        if !password.is_empty() {
            match client.login_with_password(password) {
                Ok(fresh_token) => {
                    eprintln!("RA: login successful, got fresh token (len {})", fresh_token.len());
                    client.token = fresh_token.clone();
                    *RA_CACHED_TOKEN.lock().unwrap() = Some(fresh_token);
                }
                Err(e) => {
                    eprintln!("RA: login failed: {}", e);
                }
            }
        }

        client
    }

    fn login_with_password(&self, password: &str) -> Result<String, String> {
        let params = [
            ("r", "login2"),
            ("u", &self.username),
            ("p", password),
        ];
        let text = self.get_raw(&params)?;
        let resp: LoginResponse = serde_json::from_str(&text)
            .map_err(|e| format!("parse login response: {}", e))?;
        if resp.token.is_empty() {
            return Err("login returned empty token".to_string());
        }
        Ok(resp.token)
    }

    pub fn from_config(cfg: &Config) -> Option<Self> {
        if cfg.ra_username.is_empty() {
            eprintln!("RA: username is empty, skipping");
            return None;
        }
        if cfg.ra_password.is_empty() && cfg.ra_token.is_empty() {
            eprintln!("RA: no password or token set, skipping");
            return None;
        }
        eprintln!("RA: creating client for user '{}' (password_len={}, token_len={})",
            cfg.ra_username, cfg.ra_password.len(), cfg.ra_token.len());
        Some(Self::new(&cfg.ra_username, &cfg.ra_token, &cfg.ra_password))
    }

    pub fn auth_is_broken() -> bool {
        RA_AUTH_BROKEN.load(Ordering::Relaxed)
    }

    fn rate_limit(&self) {
        let mut last = self.last_request.lock().unwrap();
        let elapsed = last.elapsed();
        if elapsed < Duration::from_millis(RA_RATE_LIMIT_MS) {
            std::thread::sleep(Duration::from_millis(RA_RATE_LIMIT_MS) - elapsed);
        }
        *last = std::time::Instant::now();
    }

    fn get_raw(&self, params: &[(&str, &str)]) -> Result<String, String> {
        self.rate_limit();
        let url = reqwest::Url::parse_with_params(RA_BASE_URL, params)
            .map_err(|e| e.to_string())?;
        let resp = self
            .http
            .get(url.clone())
            .send()
            .map_err(|e| e.to_string())?;

        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {} for {}", status, url));
        }
        resp.text().map_err(|e| e.to_string())
    }

    fn get(&self, params: &[(&str, &str)]) -> Result<String, String> {
        if RA_AUTH_BROKEN.load(Ordering::Relaxed) {
            return Err("RA auth broken — too many 401 errors, stopping".to_string());
        }

        self.rate_limit();
        let url = reqwest::Url::parse_with_params(RA_BASE_URL, params)
            .map_err(|e| e.to_string())?;
        let resp = self
            .http
            .get(url.clone())
            .send()
            .map_err(|e| e.to_string())?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let failures = RA_AUTH_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
            eprintln!("RA: 401 Unauthorized (failure {}/{}) for {}", failures, RA_MAX_AUTH_FAILURES, url);
            if failures >= RA_MAX_AUTH_FAILURES {
                RA_AUTH_BROKEN.store(true, Ordering::Relaxed);
                eprintln!("RA: auth marked as broken after {} failures — stopping all RA API calls", failures);
            }
            return Err(format!("HTTP {}", status));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            eprintln!("RA: 429 rate limited, backing off");
            std::thread::sleep(Duration::from_secs(5));
            return Err(format!("HTTP {}", status));
        }
        if !status.is_success() {
            return Err(format!("HTTP {} for {}", status, url));
        }

        RA_AUTH_FAILURES.store(0, Ordering::Relaxed);
        resp.text().map_err(|e| e.to_string())
    }

    pub fn fetch_console_games(&self, save_dir: &str, console_id: u32) -> Result<Vec<RaGameEntry>, String> {
        let cache = paths::console_games_path(save_dir, console_id);
        if cache.is_file() {
            if let Ok(data) = std::fs::read(&cache) {
                if let Ok(resp) = serde_json::from_slice::<ConsoleGamesResponse>(&data) {
                    return Ok(resp.response);
                }
            }
        }

        let params = [("r", "systemgames"), ("s", &console_id.to_string())];
        let text = self.get(&params)?;
        let resp: ConsoleGamesResponse = serde_json::from_str(&text)
            .map_err(|e| format!("parse console games: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(resp.response)
    }

    pub fn search_ra_games(save_dir: &str, console_id: u32, query: &str) -> Vec<RaGameEntry> {
        let cache = paths::console_games_path(save_dir, console_id);
        if let Ok(data) = std::fs::read(&cache) {
            if let Ok(resp) = serde_json::from_slice::<ConsoleGamesResponse>(&data) {
                let q = query.to_lowercase();
                return resp.response.into_iter()
                    .filter(|g| !g.title.contains('~') && !g.title.contains("[Subset"))
                    .filter(|g| g.title.to_lowercase().contains(&q))
                    .collect();
            }
        }
        Vec::new()
    }

    pub fn fetch_game_data(&self, save_dir: &str, game_id: &str) -> Result<RaGameData, String> {
        let cache = paths::game_data_path(save_dir, game_id);
        if cache.is_file() {
            if let Ok(data) = std::fs::read(&cache) {
                if let Ok(resp) = serde_json::from_slice::<GameDataResponse>(&data) {
                    return Ok(resp.patch_data);
                }
            }
        }

        let params = [
            ("r", "patch"),
            ("u", &self.username),
            ("t", &self.token),
            ("g", game_id),
        ];
        let text = self.get(&params)?;
        let resp: GameDataResponse = serde_json::from_str(&text)
            .map_err(|e| format!("parse game data: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(resp.patch_data)
    }

    pub fn fetch_user_unlocks(&self, save_dir: &str, game_id: &str) -> Result<Vec<u32>, String> {
        let cache = paths::unlocks_path(save_dir, game_id);

        if cache.is_file() {
            if let Ok(meta) = std::fs::metadata(&cache) {
                if let Ok(modified) = meta.modified() {
                    if modified.elapsed().unwrap_or(Duration::from_secs(UNLOCKS_CACHE_SECS + 1))
                        < Duration::from_secs(UNLOCKS_CACHE_SECS)
                    {
                        if let Ok(data) = std::fs::read(&cache) {
                            if let Ok(resp) = serde_json::from_slice::<UnlocksResponse>(&data) {
                                return Ok(resp.user_unlocks);
                            }
                        }
                    }
                }
            }
        }

        let params = [
            ("r", "unlocks"),
            ("u", &self.username),
            ("t", &self.token),
            ("g", game_id),
            ("h", "1"),
        ];
        let text = self.get(&params)?;
        let resp: UnlocksResponse = serde_json::from_str(&text)
            .map_err(|e| format!("parse unlocks: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(resp.user_unlocks)
    }

    pub fn download_badge(&self, save_dir: &str, game_id: &str, badge_name: &str, locked: bool) -> String {
        let _s = info_span!("download_badge", badge_name).entered();
        let dest = if locked {
            paths::badge_locked_path(save_dir, game_id, badge_name)
        } else {
            paths::badge_path(save_dir, game_id, badge_name)
        };
        if dest.is_file() {
            return dest.to_string_lossy().into_owned();
        }
        let _ = std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
        let suffix = if locked { "_lock" } else { "" };
        let url = format!("{}/{}{}.png", RA_BADGE_URL, badge_name, suffix);
        let tmp = dest.with_extension("png");
        match self.http.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes() {
                    Ok(bytes) => {
                        if std::fs::write(&tmp, &bytes).is_ok() {
                            ira_parser::convert_to_lossless_webp(&tmp);
                            return dest.to_string_lossy().into_owned();
                        }
                    }
                    Err(e) => eprintln!("RA badge download read error: {}", e),
                }
            }
            Ok(resp) => eprintln!("RA badge HTTP {}", resp.status()),
            Err(e) => eprintln!("RA badge download error: {}", e),
        }
        String::new()
    }

    pub fn download_game_icon(&self, save_dir: &str, db_id: i64, image_icon: &str) -> String {
        let _s = info_span!("download_game_icon", db_id).entered();
        let dest = ira_parser::retro_data_dir(save_dir, db_id).join("icon.webp");
        if dest.is_file() {
            return dest.to_string_lossy().into_owned();
        }
        let _ = std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
        let url = if image_icon.starts_with("http") {
            image_icon.to_string()
        } else {
            format!("https://retroachievements.org{}", image_icon)
        };
        let tmp = dest.with_extension("png");
        match self.http.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes() {
                    Ok(bytes) => {
                        if std::fs::write(&tmp, &bytes).is_ok() {
                            ira_parser::convert_to_lossless_webp(&tmp);
                            if dest.is_file() {
                                return dest.to_string_lossy().into_owned();
                            }
                            if tmp.is_file() {
                                return tmp.to_string_lossy().into_owned();
                            }
                        }
                    }
                    Err(e) => eprintln!("RA icon download read error: {}", e),
                }
            }
            Ok(resp) => eprintln!("RA icon HTTP {}", resp.status()),
            Err(e) => eprintln!("RA icon download error: {}", e),
        }
        String::new()
    }
}
