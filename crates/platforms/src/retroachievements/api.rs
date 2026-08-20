use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use tracing::info_span;

use crate::retroachievements::paths;
use ira_config::Config;

pub use super::api_types::read_console_games_cache;
pub use super::api_types::{
    build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache,
    redownload_missing_ra_badges, RaAchievementDef, RaGameData, RaGameEntry, RaUnlockInfo,
};
use super::api_types::WebGameProgress;

const RA_WEB_GAME_LIST: &str = "https://retroachievements.org/API/API_GetGameList.php";
const RA_WEB_GAME_PROGRESS: &str =
    "https://retroachievements.org/API/API_GetGameInfoAndUserProgress.php";
const RA_BADGE_URL: &str = "https://media.retroachievements.org/Badge";
const RA_IMAGE_URL: &str = "https://media.retroachievements.org";
const CACHE_SECS: u64 = 3600;
const RA_RATE_LIMIT_MS: u64 = 500;

pub struct RaClient {
    http: reqwest::blocking::Client,
    username: String,
    api_key: String,
    last_request: Mutex<std::time::Instant>,
}

impl RaClient {
    pub fn new(username: &str, web_api_key: &str) -> Self {
        RaClient {
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("failed to build RA HTTP client"),
            username: username.to_string(),
            api_key: web_api_key.to_string(),
            last_request: Mutex::new(std::time::Instant::now() - Duration::from_secs(1)),
        }
    }

    pub fn from_config(cfg: &Config) -> Option<Self> {
        if cfg.ra_username.is_empty() {
            eprintln!("RA: username is empty, skipping");
            return None;
        }
        if cfg.ra_web_api_key.is_empty() {
            eprintln!("RA: Web API key is empty, skipping");
            return None;
        }
        Some(Self::new(&cfg.ra_username, &cfg.ra_web_api_key))
    }

    fn rate_limit(&self) {
        let mut last = self.last_request.lock().unwrap();
        let elapsed = last.elapsed();
        if elapsed < Duration::from_millis(RA_RATE_LIMIT_MS) {
            std::thread::sleep(Duration::from_millis(RA_RATE_LIMIT_MS) - elapsed);
        }
        *last = std::time::Instant::now();
    }

    fn get_web(&self, url: reqwest::Url) -> Result<String, String> {
        self.rate_limit();
        let resp = self
            .http
            .get(url.clone())
            .send()
            .map_err(|e| format!("RA web api request: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("RA web api HTTP {} for {}", status, url));
        }
        resp.text().map_err(|e| format!("RA web api body: {}", e))
    }

    fn cached_ok(cache: &std::path::Path) -> bool {
        cache.is_file()
            && std::fs::metadata(cache)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age < Duration::from_secs(CACHE_SECS))
                .unwrap_or(false)
    }

    /// True when a fresh, parseable console game-list cache exists — no
    /// refetch needed. Detects missing, stale, and legacy-format caches so
    /// the game list self-heals after the dorequest → Web API migration.
    pub(crate) fn console_cache_is_current(save_dir: &str, console_id: u32) -> bool {
        let cache = paths::console_games_path(save_dir, console_id);
        RaClient::cached_ok(&cache) && read_console_games_cache(save_dir, console_id).is_some()
    }

    pub fn fetch_console_games(
        &self,
        save_dir: &str,
        console_id: u32,
    ) -> Result<Vec<RaGameEntry>, String> {
        let cache = paths::console_games_path(save_dir, console_id);
        if Self::cached_ok(&cache) {
            if let Some(games) = read_console_games_cache(save_dir, console_id) {
                return Ok(games);
            }
        }

        let console = console_id.to_string();
        let only_achievements = "1".to_string();
        let url = reqwest::Url::parse_with_params(
            RA_WEB_GAME_LIST,
            &[("i", &console), ("y", &self.api_key), ("f", &only_achievements)],
        )
        .map_err(|e| format!("game list url: {}", e))?;
        let text = self.get_web(url)?;
        let resp: Vec<RaGameEntry> = serde_json::from_str(&text)
            .map_err(|e| format!("parse game list: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(resp)
    }

    /// Search the console's full game list (fetched on demand from the Web API
    /// when no fresh cache exists) for titles matching `query`. Returns games
    /// that have achievements, excluding Subset/hack variants.
    pub fn search_ra_games(&self, save_dir: &str, console_id: u32, query: &str) -> Vec<RaGameEntry> {
        let games = match self.fetch_console_games(save_dir, console_id) {
            Ok(games) => games,
            Err(e) => {
                eprintln!("RA search: failed to load console game list: {}", e);
                match read_console_games_cache(save_dir, console_id) {
                    Some(games) => games,
                    None => return Vec::new(),
                }
            }
        };
        let mut results = filter_ra_games(games.clone(), query);
        if let Ok(id) = query.parse::<u32>() {
            if let Some(game) = games.into_iter().find(|g| g.id == id) {
                if !results.iter().any(|g| g.id == id) {
                    results.insert(0, game);
                }
            }
        }
        results
    }

    pub fn fetch_web_game_progress(
        &self,
        save_dir: &str,
        game_id: &str,
    ) -> Result<(RaGameData, std::collections::HashMap<u32, RaUnlockInfo>), String> {
        let cache = paths::web_progress_path(save_dir, game_id);

        if Self::cached_ok(&cache) {
            if let Ok(data) = std::fs::read(&cache) {
                if let Ok(resp) = serde_json::from_slice::<WebGameProgress>(&data) {
                    return Ok(super::api_types::web_progress_to_data(&resp));
                }
            }
        }

        let url =
            reqwest::Url::parse_with_params(RA_WEB_GAME_PROGRESS, &[
                ("g", game_id),
                ("u", &self.username),
                ("y", &self.api_key),
            ])
            .map_err(|e| format!("web api url: {}", e))?;
        let text = self.get_web(url)?;
        let progress: WebGameProgress = serde_json::from_str(&text)
            .map_err(|e| format!("parse web progress: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(super::api_types::web_progress_to_data(&progress))
    }

    pub fn download_badge(
        &self,
        save_dir: &str,
        game_id: &str,
        badge_name: &str,
        locked: bool,
    ) -> String {
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
            Ok(resp) if resp.status().is_success() => match resp.bytes() {
                Ok(bytes) => {
                    if std::fs::write(&tmp, &bytes).is_ok() {
                        ira_parser::convert_to_lossless_webp(&tmp);
                        return dest.to_string_lossy().into_owned();
                    }
                }
                Err(e) => eprintln!("RA badge download read error: {}", e),
            },
            Ok(resp) => eprintln!("RA badge HTTP {}", resp.status()),
            Err(e) => eprintln!("RA badge download error: {}", e),
        }
        String::new()
    }

    /// Fetch the bytes at an RA image path, applying the `RA_IMAGE_URL`
    /// prefix unless the path is already an absolute URL. Errors on network
    /// failure or a non-success status.
    fn fetch_ra_bytes(&self, image_icon: &str) -> Result<Vec<u8>, String> {
        let url = if image_icon.starts_with("http") {
            image_icon.to_string()
        } else {
            format!("{}{}", RA_IMAGE_URL, image_icon)
        };
        let resp = self
            .http
            .get(&url)
            .send()
            .map_err(|e| format!("RA icon download error: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("RA icon HTTP {}", resp.status()));
        }
        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| format!("RA icon download read error: {}", e))
    }

    /// Download the RA game icon as image bytes without touching any game
    /// data directory. Returns lossless WebP for PNG/ICO sources, the raw
    /// bytes for JPEG/WebP, and an error when the fetch fails or the body
    /// isn't a decodable image. Callers treat the result as a pending draft
    /// applied only on Save.
    pub fn download_game_icon_bytes(&self, image_icon: &str) -> Result<Vec<u8>, String> {
        let bytes = self.fetch_ra_bytes(image_icon)?;
        if bytes.is_empty() {
            return Err("RA icon: empty body".to_string());
        }
        let out = ira_parser::convert_bytes_to_lossless_webp(&bytes)
            .ok_or_else(|| "RA icon: body is not a decodable image".to_string())?;
        // convert passes JPEG/WebP through on magic bytes alone; fully decode
        // the payload so a corrupt body never reaches disk on Save.
        if !ira_parser::is_decodable_image(&bytes) {
            return Err("RA icon: body is not a decodable image".to_string());
        }
        Ok(out)
    }

    /// Download the game's icon from RA to `data/retro/{db_id}/icon.webp`,
    /// reusing the on-disk copy when it already exists. Returns the icon path
    /// on success, or an empty string when the fetch fails or the body isn't
    /// a decodable image.
    pub fn download_game_icon(&self, save_dir: &str, db_id: i64, image_icon: &str) -> String {
        let _s = info_span!("download_game_icon", db_id).entered();
        let dest = ira_parser::retro_data_dir(save_dir, db_id).join("icon.webp");
        if dest.is_file() {
            return dest.to_string_lossy().into_owned();
        }
        let bytes = match self.fetch_ra_bytes(image_icon) {
            Ok(b) if !b.is_empty() && ira_parser::is_decodable_image(&b) => b,
            Ok(_) => {
                eprintln!("RA icon: body is not a decodable image");
                return String::new();
            }
            Err(e) => {
                eprintln!("{}", e);
                return String::new();
            }
        };
        let _ = std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
        let tmp = dest.with_extension("png");
        if std::fs::write(&tmp, &bytes).is_ok() {
            ira_parser::convert_to_lossless_webp(&tmp);
            if dest.is_file() {
                return dest.to_string_lossy().into_owned();
            }
            if tmp.is_file() {
                return tmp.to_string_lossy().into_owned();
            }
        }
        let _ = std::fs::remove_file(&tmp);
        String::new()
    }
}

fn filter_ra_games(games: Vec<RaGameEntry>, q: &str) -> Vec<RaGameEntry> {
    let q = q.to_lowercase();
    games
        .into_iter()
        .filter(|g| !g.title.contains('~') && !g.title.contains("[Subset"))
        .filter(|g| g.title.to_lowercase().contains(&q))
        .collect()
}

#[cfg(test)]
mod cache_tests {
    use std::time::Duration;

    use super::RaClient;
    use crate::retroachievements::paths;

    fn write_cache(save_dir: &str, console_id: u32, contents: &str) {
        let path = paths::console_games_path(save_dir, console_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    #[test]
    fn test_new_format_cache_is_current() {
        let tmp = tempfile::tempdir().unwrap();
        write_cache(
            tmp.path().to_str().unwrap(),
            3,
            r#"[{"ID":1,"Title":"Game","ImageIcon":"","ImageUrl":"","NumAchievements":0,"Points":0}]"#,
        );
        assert!(RaClient::console_cache_is_current(tmp.path().to_str().unwrap(), 3));
    }

    #[test]
    fn test_legacy_format_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        // Pre-migration dorequest `systemgames` wrapper — the whole reason the
        // `needs_fetch` predicate must not just check `is_file()`.
        write_cache(
            tmp.path().to_str().unwrap(),
            3,
            r#"{"Success":true,"Error":"","Response":[{"ID":1,"Title":"Game"}]}"#,
        );
        assert!(!RaClient::console_cache_is_current(tmp.path().to_str().unwrap(), 3));
    }

    #[test]
    fn test_missing_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!RaClient::console_cache_is_current(tmp.path().to_str().unwrap(), 3));
    }

    #[test]
    fn test_garbage_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        write_cache(tmp.path().to_str().unwrap(), 3, "not json");
        assert!(!RaClient::console_cache_is_current(tmp.path().to_str().unwrap(), 3));
    }

    #[test]
    fn test_filter_ra_games_matches_substring_and_skips_subsets() {
        use super::{filter_ra_games, RaGameEntry};
        let entry = |id: u32, title: &str| RaGameEntry {
            id,
            title: title.to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 1,
            points: 1,
        };
        let games = vec![
            entry(1, "Super Mario World"),
            entry(2, "Super Mario World [Subset - Anything]"),
            entry(3, "~Hack~ Super Mario"),
            entry(4, "Kirby's Dream Land"),
        ];
        let result = filter_ra_games(games, "mario");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 1);
    }

    #[test]
    fn test_filter_ra_games_empty_query_matches_all() {
        use super::{filter_ra_games, RaGameEntry};
        let games = vec![RaGameEntry {
            id: 1,
            title: "Mario".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 1,
            points: 1,
        }];
        let result = filter_ra_games(games, "");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_search_ra_games_numeric_id_prepends_exact_match() {
        let client = RaClient::new("user", "key");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        write_cache(
            dir,
            3,
            r#"[
                {"ID":1,"Title":"Super Mario World","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1},
                {"ID":5678,"Title":"Mario Kart 64","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1},
                {"ID":9000,"Title":"~Hack~ Super Mario","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1}
            ]"#,
        );
        let results = client.search_ra_games(dir, 3, "5678");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 5678);
        assert_eq!(results[0].title, "Mario Kart 64");
    }

    #[test]
    fn test_search_ra_games_numeric_id_no_match_returns_empty() {
        let client = RaClient::new("user", "key");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        write_cache(
            dir,
            3,
            r#"[{"ID":1,"Title":"Super Mario World","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1}]"#,
        );
        let results = client.search_ra_games(dir, 3, "9999");
        assert!(results.is_empty());
    }

    #[test]
    fn test_old_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let path = paths::console_games_path(tmp.path().to_str().unwrap(), 3);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(60 * 60 * 24);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap();
        f.set_modified(old).unwrap();
        assert!(!RaClient::console_cache_is_current(tmp.path().to_str().unwrap(), 3));
    }
}