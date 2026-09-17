use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use tracing::info_span;

use crate::retroachievements::paths;
use ira_config::Config;
use ira_models::normalize_name;

pub use super::api_types::read_console_games_cache;
use super::api_types::WebGameProgress;
pub use super::api_types::{
    build_ra_achievements, enrich_ra_game, load_ra_achievements_from_cache,
    redownload_missing_ra_badges, RaAchievementDef, RaGameData, RaGameEntry, RaUnlockInfo,
};

const RA_WEB_GAME_LIST: &str = "https://retroachievements.org/API/API_GetGameList.php";
const RA_WEB_GAME_PROGRESS: &str =
    "https://retroachievements.org/API/API_GetGameInfoAndUserProgress.php";
const RA_BADGE_URL: &str = "https://media.retroachievements.org/Badge";
const RA_IMAGE_URL: &str = "https://media.retroachievements.org";
/// RA's generic gamepad placeholder. Games without real artwork report this
/// path as their `ImageIcon`; persisting it would also block a real icon
/// from ever being fetched, since on-disk icons are never replaced.
const RA_FALLBACK_ICON: &str = "/Images/000001.png";

/// True when the given `ImageIcon` path is RA's placeholder for games
/// without artwork rather than a real icon.
pub fn is_fallback_game_icon(image_icon: &str) -> bool {
    let path = image_icon.strip_prefix(RA_IMAGE_URL).unwrap_or(image_icon);
    path.eq_ignore_ascii_case(RA_FALLBACK_ICON)
}
const CACHE_SECS: u64 = 3600;
const RA_RATE_LIMIT_MS: u64 = 500;

/// The web key rides in the query string; error text must not carry it.
fn redact_url(url: &reqwest::Url) -> String {
    match url.query() {
        Some(_) => format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            url.path()
        ),
        None => url.to_string(),
    }
}

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
            .map_err(|e| format!("RA web api request: {}", e.to_string().replace(url.as_str(), &redact_url(&url))))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("RA web api HTTP {} for {}", status, redact_url(&url)));
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
        let with_hashes = "1".to_string();
        // `f` is omitted so the list covers every game on the console,
        // including achievement-less ones that are still worth matching as
        // icon sources; `h=1` inlines each game's supported hashes so
        // matching can resolve exact game IDs locally.
        let url = reqwest::Url::parse_with_params(
            RA_WEB_GAME_LIST,
            &[("i", &console), ("y", &self.api_key), ("h", &with_hashes)],
        )
        .map_err(|e| format!("game list url: {}", e))?;
        let text = self.get_web(url)?;
        let resp: Vec<RaGameEntry> =
            serde_json::from_str(&text).map_err(|e| format!("parse game list: {}", e))?;

        let _ = std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&cache, &text);

        Ok(resp)
    }

    /// The console's game list: fetched (or served from a fresh cache) via
    /// [`Self::fetch_console_games`], falling back to a stale cache when the
    /// fetch fails. None only when there is no list at all.
    fn console_games(&self, save_dir: &str, console_id: u32) -> Option<Vec<RaGameEntry>> {
        match self.fetch_console_games(save_dir, console_id) {
            Ok(games) => Some(games),
            Err(e) => {
                eprintln!("RA: failed to load console game list: {}", e);
                read_console_games_cache(save_dir, console_id)
            }
        }
    }

    /// Search the console's full game list (fetched on demand from the Web API
    /// when no fresh cache exists) for titles matching `query`, excluding
    /// Subset/hack variants. Matching is punctuation-insensitive, and a query
    /// with no hit retries on the part before its first " - ".
    pub fn search_ra_games(
        &self,
        save_dir: &str,
        console_id: u32,
        query: &str,
    ) -> Vec<RaGameEntry> {
        let Some(games) = self.console_games(save_dir, console_id) else {
            return Vec::new();
        };
        let mut results = search_games(games.clone(), query);
        if let Ok(id) = query.parse::<u32>() {
            if let Some(game) = games.into_iter().find(|g| g.id == id) {
                if !results.iter().any(|g| g.id == id) {
                    results.insert(0, game);
                }
            }
        }
        results
    }

    /// Returns the game in the console's list whose supported hashes contain
    /// `rom_hash` (case-insensitive), or None when the hash is empty, the
    /// list can't be loaded, or no game claims the hash.
    pub fn find_game_by_hash(
        &self,
        save_dir: &str,
        console_id: u32,
        rom_hash: &str,
    ) -> Option<RaGameEntry> {
        if rom_hash.is_empty() {
            return None;
        }
        let games = self.console_games(save_dir, console_id)?;
        find_game_by_hash(&games, rom_hash)
    }

    /// Resolves a ROM to its RA game the way the library scan does: exact
    /// hash first, then a title equal to one of `names` under the shared
    /// normalizer (tried in order), preferring entries that have
    /// achievements. Hack/subset variants are never returned, and a name
    /// that is merely *contained* in a title is not a match — the caller is
    /// an automatic matcher and must not guess.
    pub fn match_ra_game(
        &self,
        save_dir: &str,
        console_id: u32,
        rom_hash: &str,
        names: &[&str],
    ) -> Option<RaGameEntry> {
        let games = self.console_games(save_dir, console_id)?;
        find_game_by_hash(&games, rom_hash).or_else(|| match_by_title(&games, names))
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

        let url = reqwest::Url::parse_with_params(
            RA_WEB_GAME_PROGRESS,
            &[("g", game_id), ("u", &self.username), ("y", &self.api_key)],
        )
        .map_err(|e| format!("web api url: {}", e))?;
        let text = self.get_web(url)?;
        let progress: WebGameProgress =
            serde_json::from_str(&text).map_err(|e| format!("parse web progress: {}", e))?;

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
        match self.fetch_ra_bytes(&url) {
            Ok(bytes) => {
                if std::fs::write(&tmp, &bytes).is_ok() {
                    ira_parser::convert_to_lossless_webp(&tmp);
                    return dest.to_string_lossy().into_owned();
                }
            }
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
    /// bytes for JPEG/WebP, and an error when the fetch fails, the body
    /// isn't a decodable image, or the path is RA's fallback placeholder.
    /// Callers treat the result as a pending draft applied only on Save.
    pub fn download_game_icon_bytes(&self, image_icon: &str) -> Result<Vec<u8>, String> {
        if is_fallback_game_icon(image_icon) {
            return Err("RA icon: game has no custom icon".to_string());
        }
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
    /// on success, or an empty string when the fetch fails, the body isn't
    /// a decodable image, or the path is RA's fallback placeholder.
    pub fn download_game_icon(&self, save_dir: &str, db_id: i64, image_icon: &str) -> String {
        let _s = info_span!("download_game_icon", db_id).entered();
        let dest = ira_parser::retro_data_dir(save_dir, db_id).join("icon.webp");
        if dest.is_file() {
            return dest.to_string_lossy().into_owned();
        }
        if is_fallback_game_icon(image_icon) {
            return String::new();
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

/// Whether an RA list entry is a real game rather than a `~Hack~` or
/// `[Subset - …]` variant, which matchers never resolve to.
pub fn is_main_ra_entry(game: &RaGameEntry) -> bool {
    !game.title.contains('~') && !game.title.contains("[Subset")
}

/// Punctuation-insensitive title filter: both sides go through
/// [`normalize_name`], so "Castlevania - Dawn of Sorrow" finds
/// "Castlevania: Dawn of Sorrow" and "World Ends With You, The" finds
/// "The World Ends With You". Hack/subset variants stay excluded.
fn filter_ra_games(games: Vec<RaGameEntry>, q: &str) -> Vec<RaGameEntry> {
    let norm_q = normalize_name(q);
    games
        .into_iter()
        .filter(is_main_ra_entry)
        .filter(|g| normalize_name(&g.title).contains(&norm_q))
        .collect()
}

/// The first of `names` that equals a main entry's title under the shared
/// normalizer; among equal titles an entry with achievements wins over a
/// same-titled regional duplicate without a set.
fn match_by_title(games: &[RaGameEntry], names: &[&str]) -> Option<RaGameEntry> {
    names
        .iter()
        .map(|name| normalize_name(name))
        .filter(|key| !key.is_empty())
        .find_map(|key| {
            let mut without_set = None;
            for game in games.iter().filter(|g| is_main_ra_entry(g)) {
                if normalize_name(&game.title) != key {
                    continue;
                }
                if game.num_achievements > 0 {
                    return Some(game.clone());
                }
                without_set.get_or_insert_with(|| game.clone());
            }
            without_set
        })
}

/// [`filter_ra_games`], broadened: when the full query has no hit, only the
/// part before its first " - " is searched — file names hang region dumps
/// and subtitles off it that RA titles spell with ":" or not at all.
fn search_games(games: Vec<RaGameEntry>, query: &str) -> Vec<RaGameEntry> {
    let results = filter_ra_games(games.clone(), query);
    if !results.is_empty() {
        return results;
    }
    match query.split_once(" - ") {
        Some((prefix, _)) if !prefix.trim().is_empty() => filter_ra_games(games, prefix),
        _ => results,
    }
}

fn find_game_by_hash(games: &[RaGameEntry], rom_hash: &str) -> Option<RaGameEntry> {
    if rom_hash.is_empty() {
        return None;
    }
    games
        .iter()
        .find(|g| g.hashes.iter().any(|h| h.eq_ignore_ascii_case(rom_hash)))
        .cloned()
}

#[cfg(test)]
mod redact_tests {
    use super::redact_url;

    #[test]
    fn test_redact_url_hides_the_web_key() {
        let url = reqwest::Url::parse(
            "https://retroachievements.org/API/API_GetGameList.php?i=7&y=sekrit&h=1",
        )
        .unwrap();
        assert_eq!(redact_url(&url), "https://retroachievements.org/API/API_GetGameList.php");
    }
}

#[cfg(test)]
mod cache_tests {
    use std::time::Duration;

    use super::{RaClient, RaGameEntry};
    use crate::retroachievements::paths;

    fn entry(id: u32, title: &str) -> RaGameEntry {
        RaGameEntry {
            id,
            title: title.to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 1,
            points: 1,
            hashes: Vec::new(),
        }
    }

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
        assert!(RaClient::console_cache_is_current(
            tmp.path().to_str().unwrap(),
            3
        ));
    }

    #[test]
    fn test_console_cache_parses_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        write_cache(
            tmp.path().to_str().unwrap(),
            3,
            r#"[{"ID":1,"Title":"Game","ImageIcon":"","ImageUrl":"","NumAchievements":0,"Points":0,"Hashes":["abc123","def456"]}]"#,
        );
        let games =
            super::read_console_games_cache(tmp.path().to_str().unwrap(), 3).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].hashes, vec!["abc123", "def456"]);
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
        assert!(!RaClient::console_cache_is_current(
            tmp.path().to_str().unwrap(),
            3
        ));
    }

    #[test]
    fn test_missing_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!RaClient::console_cache_is_current(
            tmp.path().to_str().unwrap(),
            3
        ));
    }

    #[test]
    fn test_garbage_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        write_cache(tmp.path().to_str().unwrap(), 3, "not json");
        assert!(!RaClient::console_cache_is_current(
            tmp.path().to_str().unwrap(),
            3
        ));
    }

    #[test]
    fn test_filter_ra_games_matches_substring_and_skips_subsets() {
        use super::filter_ra_games;
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
        use super::filter_ra_games;
        let games = vec![entry(1, "Mario")];
        let result = filter_ra_games(games, "");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_ra_games_normalizes_punctuation_and_articles() {
        use super::filter_ra_games;
        let games = vec![
            entry(1, "Castlevania: Dawn of Sorrow"),
            entry(2, "The World Ends with You"),
        ];
        // File-name punctuation and trailing articles find the RA spelling.
        let hit = |q: &str, id: u32| {
            let result = filter_ra_games(games.clone(), q);
            assert_eq!(result.len(), 1, "query {q:?}");
            assert_eq!(result[0].id, id, "query {q:?}");
        };
        hit("Castlevania - Dawn of Sorrow", 1);
        hit("castlevania dawn of sorrow", 1);
        hit("World Ends With You, The", 2);
        hit("the world ends with you", 2);
    }

    #[test]
    fn test_search_games_falls_back_to_prefix_before_dash() {
        use super::search_games;
        let games = vec![
            entry(1, "Castlevania: Aria of Sorrow"),
            entry(2, "Castlevania: Dawn of Sorrow"),
            entry(3, "Castlevania Judgment"),
            entry(4, "Bubble Bobble"),
        ];
        // No title contains the whole file-name text ("USA" is outside any
        // bracket group), so the part before " - " decides.
        let result = search_games(games, "Castlevania - Dawn of Sorrow USA");
        assert_eq!(
            result.iter().map(|g| g.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn test_search_games_full_query_wins_when_it_matches() {
        use super::search_games;
        let games = vec![
            entry(1, "Castlevania: Aria of Sorrow"),
            entry(2, "Castlevania: Dawn of Sorrow"),
        ];
        let result = search_games(games, "Castlevania - Dawn of Sorrow");
        assert_eq!(result.iter().map(|g| g.id).collect::<Vec<_>>(), [2]);
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
    fn test_find_game_by_hash_resolves_cached_list() {
        let client = RaClient::new("user", "key");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        write_cache(
            dir,
            3,
            r#"[{"ID":1,"Title":"Super Mario World","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1,"Hashes":["aaa"]},
                {"ID":24933,"Title":"Shin Megami Tensei: Devil Survivor","ImageIcon":"/Images/x.png","ImageUrl":"","NumAchievements":0,"Points":0,"Hashes":["abc123"]}]"#,
        );
        let hit = client.find_game_by_hash(dir, 3, "abc123").unwrap();
        assert_eq!(hit.id, 24933);
        // Case-insensitive on the ROM side; RA hashes are lowercase.
        assert_eq!(client.find_game_by_hash(dir, 3, "ABC123").unwrap().id, 24933);
        assert!(client.find_game_by_hash(dir, 3, "nomatch").is_none());
        assert!(client.find_game_by_hash(dir, 3, "").is_none());
    }

    #[test]
    fn test_find_game_by_hash_empty_list_is_none() {
        assert!(super::find_game_by_hash(&[], "abc123").is_none());
    }

    #[test]
    fn test_match_ra_game_hash_then_exact_title() {
        let client = RaClient::new("user", "key");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        write_cache(
            dir,
            3,
            r#"[
                {"ID":1,"Title":"Pokémon Emerald Version","ImageIcon":"","ImageUrl":"","NumAchievements":0,"Points":0,"Hashes":["aaa"]},
                {"ID":2,"Title":"Pokémon Emerald Version","ImageIcon":"","ImageUrl":"","NumAchievements":40,"Points":9,"Hashes":["bbb"]},
                {"ID":3,"Title":"~Hack~ Pokemon Emerald Version","ImageIcon":"","ImageUrl":"","NumAchievements":5,"Points":5},
                {"ID":4,"Title":"The Legend of Zelda: Phantom Hourglass","ImageIcon":"","ImageUrl":"","NumAchievements":1,"Points":1}
            ]"#,
        );
        let matched = |hash: &str, names: &[&str]| client.match_ra_game(dir, 3, hash, names).map(|g| g.id);
        // The hash wins outright, even over a same-titled entry with a set.
        assert_eq!(matched("AAA", &["Pokemon - Emerald Version"]), Some(1));
        // Title equality ignores accents and " - " vs ":"; the entry with
        // achievements beats the regional duplicate without one.
        assert_eq!(matched("", &["Pokemon - Emerald Version"]), Some(2));
        // Later names are tried when earlier ones miss; the No-Intro
        // article-before-subtitle form matches the store spelling.
        assert_eq!(
            matched("", &["Custom Title", "Legend of Zelda, The - Phantom Hourglass (USA)"]),
            Some(4)
        );
        // Containment is not a match for an automatic matcher, and hacks
        // never resolve.
        assert_eq!(matched("", &["Pokemon"]), None);
        assert_eq!(matched("", &["~Hack~ Pokemon Emerald Version"]), None);
        assert_eq!(matched("", &[""]), None);
    }

    #[test]
    fn test_is_fallback_game_icon_detection() {
        use super::is_fallback_game_icon;
        assert!(is_fallback_game_icon("/Images/000001.png"));
        assert!(is_fallback_game_icon(
            "https://media.retroachievements.org/Images/000001.png"
        ));
        assert!(!is_fallback_game_icon("/Images/123456.png"));
        assert!(!is_fallback_game_icon(""));
    }

    #[test]
    fn test_download_game_icon_refuses_fallback() {
        let client = RaClient::new("user", "key");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        // Both entry points must refuse before any network access.
        assert_eq!(client.download_game_icon(dir, 1, "/Images/000001.png"), "");
        assert!(client
            .download_game_icon_bytes("/Images/000001.png")
            .is_err());
    }

    #[test]
    fn test_old_cache_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let path = paths::console_games_path(tmp.path().to_str().unwrap(), 3);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(60 * 60 * 24);
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_modified(old).unwrap();
        assert!(!RaClient::console_cache_is_current(
            tmp.path().to_str().unwrap(),
            3
        ));
    }
}
