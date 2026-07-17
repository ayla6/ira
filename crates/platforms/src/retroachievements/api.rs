use std::path::Path;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use serde::Deserialize;

use ira_config::Config;
use ira_models::{Game, MergedAchievement};
use crate::retroachievements::paths;

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

        // Check for cached token from a previous login this session
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
                            // RA servers return PNG — convert to lossless WebP
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

    pub fn download_game_icon(&self, save_dir: &str, game_id: &str, image_icon: &str) -> String {
        let dest = paths::game_icon_path(save_dir, game_id);
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
                            return dest.to_string_lossy().into_owned();
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

#[derive(Debug, Deserialize)]
struct LoginResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "Token")]
    token: String,
}

#[derive(Debug, Deserialize)]
struct ConsoleGamesResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "Response")]
    response: Vec<RaGameEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaGameEntry {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(default, rename = "ImageIcon")]
    pub image_icon: String,
    #[serde(default, rename = "ImageUrl")]
    pub image_url: String,
    #[serde(default, rename = "NumAchievements")]
    pub num_achievements: u32,
    #[serde(default, rename = "Points")]
    pub points: u32,
}

#[derive(Debug, Deserialize)]
struct GameDataResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(rename = "PatchData")]
    patch_data: RaGameData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaGameData {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(default, rename = "ImageIcon")]
    pub image_icon: String,
    #[serde(default, rename = "Achievements")]
    pub achievements: Vec<RaAchievementDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RaAchievementDef {
    #[serde(rename = "ID")]
    pub id: u32,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Description")]
    pub description: String,
    #[serde(default, rename = "Points")]
    pub points: u32,
    #[serde(default, rename = "BadgeName")]
    pub badge_name: String,
    #[serde(default, rename = "Rarity")]
    pub rarity: f64,
    #[serde(default, rename = "RarityHardcore")]
    pub rarity_hardcore: f64,
}

#[derive(Debug, Deserialize)]
struct UnlocksResponse {
    #[serde(default)]
    _success: bool,
    #[serde(default)]
    _error: String,
    #[serde(default, rename = "UserUnlocks")]
    user_unlocks: Vec<u32>,
}

pub fn build_ra_achievements(
    game_data: &RaGameData,
    unlocks: &[u32],
    client: &RaClient,
    save_dir: &str,
    game_id: &str,
) -> (Vec<MergedAchievement>, String, String) {
    let mut achievements = Vec::new();
    let mut icon_path = String::new();
    let mut icon_gray_path = String::new();

    for def in &game_data.achievements {
        let earned = unlocks.contains(&def.id);
        let badge = if def.badge_name.is_empty() {
            String::new()
        } else if earned {
            client.download_badge(save_dir, game_id, &def.badge_name, false)
        } else {
            client.download_badge(save_dir, game_id, &def.badge_name, true)
        };

        let (icon, icon_gray) = if earned {
            let locked_badge = if def.badge_name.is_empty() {
                String::new()
            } else {
                client.download_badge(save_dir, game_id, &def.badge_name, true)
            };
            (badge.clone(), locked_badge)
        } else {
            let unlocked_badge = if def.badge_name.is_empty() {
                String::new()
            } else {
                client.download_badge(save_dir, game_id, &def.badge_name, false)
            };
            (unlocked_badge, badge.clone())
        };

        if icon_path.is_empty() && !icon.is_empty() {
            icon_path = icon.clone();
        }
        if icon_gray_path.is_empty() && !icon_gray.is_empty() {
            icon_gray_path = icon_gray.clone();
        }

        achievements.push(MergedAchievement {
            name: format!("{}", def.id),
            display_name: def.title.clone(),
            description: def.description.clone(),
            hidden: false,
            earned,
            earned_time: 0,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }

    (achievements, icon_path, icon_gray_path)
}

pub fn enrich_ra_game(game: &mut Game, save_dir: &str, username: &str, token: &str, password: &str) {
    if RaClient::auth_is_broken() {
        return;
    }

    let client = RaClient::new(username, token, password);

    let game_data = match client.fetch_game_data(save_dir, &game.app_id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("RA game data fetch failed for {}: {}", game.app_id, e);
            return;
        }
    };

    let unlocks = client.fetch_user_unlocks(save_dir, &game.app_id).unwrap_or_default();

    let (achievements, icon_path, _icon_gray) = build_ra_achievements(&game_data, &unlocks, &client, save_dir, &game.app_id);

    game.total_count = achievements.len();
    game.earned_count = achievements.iter().filter(|a| a.earned).count();
    game.achievements = achievements;

    if game.icon_path.is_empty() && !game_data.image_icon.is_empty() {
        let icon = client.download_game_icon(save_dir, &game.app_id, &game_data.image_icon);
        if !icon.is_empty() {
            game.icon_path = icon;
        }
    }
    if game.icon_path.is_empty() && !icon_path.is_empty() {
        game.icon_path = icon_path;
    }
}

pub fn load_ra_achievements_from_cache(save_dir: &str, game_id: &str) -> Vec<MergedAchievement> {
    let game_data_path = paths::game_data_path(save_dir, game_id);
    let game_data: RaGameData = match std::fs::read(&game_data_path) {
        Ok(data) => match serde_json::from_slice::<GameDataResponse>(&data) {
            Ok(resp) => resp.patch_data,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };

    let unlocks: Vec<u32> = match std::fs::read(paths::unlocks_path(save_dir, game_id)) {
        Ok(data) => serde_json::from_slice::<UnlocksResponse>(&data)
            .map(|r| r.user_unlocks)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let ach_dir = paths::achievements_dir(save_dir, game_id);
    let mut achievements = Vec::new();
    for def in &game_data.achievements {
        let earned = unlocks.contains(&def.id);
        let (icon, icon_gray) = if def.badge_name.is_empty() {
            (String::new(), String::new())
        } else {
            let earned_badge = ach_dir.join(format!("{}.webp", def.badge_name));
            let locked_badge = ach_dir.join(format!("{}_lock.webp", def.badge_name));
            let earned_path = if earned && earned_badge.is_file() {
                earned_badge.to_string_lossy().into_owned()
            } else {
                String::new()
            };
            let locked_path = if locked_badge.is_file() {
                locked_badge.to_string_lossy().into_owned()
            } else {
                String::new()
            };
            if earned {
                (earned_path, locked_path)
            } else {
                (locked_path.clone(), locked_path)
            }
        };

        achievements.push(MergedAchievement {
            name: format!("{}", def.id),
            display_name: def.title.clone(),
            description: def.description.clone(),
            hidden: false,
            earned,
            earned_time: 0,
            icon_path: icon,
            icon_gray_path: icon_gray,
            global_percent: def.rarity,
            trophy_type: '\0',
        });
    }
    achievements
}
