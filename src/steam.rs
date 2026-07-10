use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone)]
pub struct SgdbAsset {
    pub url: String,
    pub width: i64,
    pub height: i64,
    pub style: String,
    pub author: String,
    pub mime: String,
}

pub struct SteamClient {
    api_key: Mutex<String>,
    sgdb_api_key: Mutex<String>,
    cache_dir: PathBuf,
    http: reqwest::blocking::Client,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppDetails {
    #[serde(default, alias = "Name")]
    pub name: String,
    #[serde(default, alias = "Languages")]
    pub languages: Vec<String>,
    #[serde(default, alias = "Dlcs")]
    pub dlcs: std::collections::HashMap<String, DlcInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DlcInfo {
    #[serde(default, alias = "Name")]
    pub name: String,
    #[serde(default, alias = "AppId")]
    pub app_id: i64,
    #[serde(default, alias = "ImageUrl")]
    pub image_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
struct AppDetailsResponse {
    #[serde(flatten)]
    apps: std::collections::HashMap<String, AppDetailsEntry>,
}

#[derive(Debug, Deserialize)]
struct AppDetailsEntry {
    success: bool,
    data: SteamGameDetails,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteamGameDetails {
    pub name: String,
    #[serde(rename = "header_image")]
    pub _header_image: String,
    #[serde(rename = "capsule_imagev5")]
    pub capsule_image: String,
}

#[derive(Debug, Deserialize)]
struct GlobalAchievementsResponse {
    achievementpercentages: GlobalAchievementsInner,
}

#[derive(Debug, Deserialize)]
struct GlobalAchievementsInner {
    achievements: Vec<GlobalAchievementEntry>,
}

#[derive(Debug, Deserialize)]
struct GlobalAchievementEntry {
    name: String,
    #[serde(deserialize_with = "deserialize_percent")]
    percent: f64,
}

fn deserialize_percent<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => s.parse::<f64>().map_err(serde::de::Error::custom),
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| serde::de::Error::custom("invalid number")),
        _ => Err(serde::de::Error::custom("expected string or number")),
    }
}

#[derive(Debug, Deserialize)]
struct SteamSchemaResponse {
    game: SteamSchemaGame,
}

#[derive(Debug, Deserialize)]
struct SteamSchemaGame {
    #[serde(rename = "availableGameStats")]
    available_game_stats: SteamSchemaStats,
}

#[derive(Debug, Deserialize)]
struct SteamSchemaStats {
    achievements: Vec<SteamSchemaAchievement>,
}

#[derive(Debug, Deserialize)]
struct SteamSchemaAchievement {
    name: String,
    #[allow(dead_code)]
    defaultvalue: i64,
    #[serde(rename = "displayName")]
    display_name: String,
    hidden: i64,
    description: String,
    icon: String,
    #[serde(rename = "icongray")]
    icon_gray: String,
}

#[derive(Debug, Deserialize)]
struct NemirtingasAchievement {
    name: String,
    hidden: bool,
    icon: String,
    #[serde(rename = "icongray")]
    icon_gray: String,
    #[serde(rename = "displayName")]
    display_name: std::collections::HashMap<String, String>,
    #[serde(default)]
    description: std::collections::HashMap<String, String>,
}

fn pick_lang(m: &std::collections::HashMap<String, String>) -> String {
    if let Some(v) = m.get("english") {
        if !v.is_empty() {
            return v.clone();
        }
    }
    for v in m.values() {
        if !v.is_empty() {
            return v.clone();
        }
    }
    String::new()
}

const NEMIRTINGAS_BASE_URL: &str =
    "https://raw.githubusercontent.com/Nemirtingas/games-infos-datas/refs/heads/main/steam";

const MIN_IMAGE_BYTES: u64 = 200;

impl SteamClient {
    pub fn new(api_key: String, sgdb_key: String, data_dir: &str) -> Self {
        SteamClient {
            api_key: Mutex::new(api_key),
            sgdb_api_key: Mutex::new(sgdb_key),
            cache_dir: PathBuf::from(data_dir),
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    pub fn update_keys(&self, api_key: &str, sgdb_key: &str) {
        *self.api_key.lock().unwrap() = api_key.to_string();
        *self.sgdb_api_key.lock().unwrap() = sgdb_key.to_string();
    }

    fn api_key(&self) -> String {
        self.api_key.lock().unwrap().clone()
    }

    fn sgdb_api_key(&self) -> String {
        self.sgdb_api_key.lock().unwrap().clone()
    }

    fn game_dir(&self, app_id: &str) -> PathBuf {
        self.cache_dir.join("steam").join(app_id)
    }

    fn sgdb_dir(&self, sgdb_id: &str) -> PathBuf {
        self.cache_dir.join("steamgriddb").join(sgdb_id)
    }

    /// Download `url` to `dest` and return its path if it's a real image
    /// (>= `MIN_IMAGE_BYTES`); otherwise delete the bad file and return "".
    fn fetch_image(&self, url: &str, dest: &Path) -> String {
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

    /// Return an existing `dest` as-is, otherwise download `primary` (then
    /// `fallback` if given and the primary yielded nothing).
    fn fetch_image_fallback(&self, primary: &str, fallback: &str, dest: &Path) -> String {
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

    pub fn fetch_game_details(&self, app_id: &str) -> Option<SteamGameDetails> {
        let cache_path = self.game_dir(app_id).join("appdetails.json");
        if let Ok(data) = std::fs::read(&cache_path) {
            if let Ok(d) = serde_json::from_slice::<SteamGameDetails>(&data) {
                return Some(d);
            }
        }

        let url = format!("https://store.steampowered.com/api/appdetails?appids={}", app_id);
        let resp = match self.http.get(&url).send() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Steam details unavailable for {}: {}", app_id, e);
                return None;
            }
        };

        let raw: AppDetailsResponse = match resp.json() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Steam details decode error for {}: {}", app_id, e);
                return None;
            }
        };

        let entry = match raw.apps.get(app_id) {
            Some(e) if e.success => e,
            _ => {
                eprintln!("No store data for app {}", app_id);
                return None;
            }
        };

        let _ = std::fs::create_dir_all(self.game_dir(app_id));
        if let Ok(b) = serde_json::to_vec(&entry.data) {
            let _ = std::fs::write(&cache_path, b);
        }
        Some(entry.data.clone())
    }

    pub fn fetch_global_achievements(&self, app_id: &str) -> Option<std::collections::HashMap<String, f64>> {
        let cache_path = self.game_dir(app_id).join("global_achievements.json");
        if let Ok(data) = std::fs::read(&cache_path) {
            if let Ok(m) = serde_json::from_slice::<std::collections::HashMap<String, f64>>(&data) {
                return Some(m);
            }
        }

        let url = format!(
            "https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v0002/?gameid={}&format=json",
            app_id
        );
        let resp = match self.http.get(&url).send() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Global achievements unavailable for {}: {}", app_id, e);
                return None;
            }
        };

        let text = match resp.text() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Global achievements read error for {}: {}", app_id, e);
                return None;
            }
        };

        let raw: GlobalAchievementsResponse = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Global achievements decode error for {}: {}", app_id, e);
                eprintln!("Response body: {}", &text[..text.len().min(500)]);
                return None;
            }
        };

        let mut m = std::collections::HashMap::new();
        for a in raw.achievementpercentages.achievements {
            m.insert(a.name, a.percent);
        }
        let _ = std::fs::create_dir_all(self.game_dir(app_id));
        if let Ok(b) = serde_json::to_vec(&m) {
            let _ = std::fs::write(&cache_path, b);
        }
        Some(m)
    }

    pub fn download_file(&self, url: &str, dest: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")))
            .map_err(|e| e.to_string())?;
        let resp = self
            .http
            .get(url)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        std::fs::write(dest, &bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn find_cached_icon(&self, app_id: &str) -> Option<PathBuf> {
        let dir = self.game_dir(app_id);
        for ext in [".png", ".ico", ".jpg", ".webp"] {
            let path = dir.join(format!("icon{}", ext));
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    fn find_cached_hero(&self, app_id: &str) -> Option<PathBuf> {
        let path = self.game_dir(app_id).join("library_hero.jpg");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    fn fetch_sgdb_icon_url(&self, app_id: &str) -> Option<String> {
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() {
            return None;
        }
        let resp = self
            .http
            .get(format!("https://www.steamgriddb.com/api/v2/icons/steam/{}", app_id))
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let raw: serde_json::Value = resp.json().ok()?;
        let data = raw.get("data")?.as_array()?;
        if data.is_empty() {
            return None;
        }
        // Pick the smallest icon (prefer 128x128 or lower)
        let mut best: Option<(&serde_json::Value, i64)> = None;
        for item in data {
            let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
            if w <= 128 {
                if best.is_none() || w < best.unwrap().1 {
                    best = Some((item, w));
                }
            } else if best.is_none() {
                best = Some((item, w));
            }
        }
        let chosen = best.map(|(item, _)| item).unwrap_or(&data[0]);
        chosen.get("url")?.as_str().map(|s| s.to_string())
    }

    /// Fetch a single asset URL from SGDB by game ID and asset type.
    /// `asset_type` is "heroes", "grids", or "logos".
    fn fetch_sgdb_asset_url(&self, sgdb_id: &str, asset_type: &str) -> Option<String> {
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() {
            return None;
        }
        let resp = self
            .http
            .get(format!("https://www.steamgriddb.com/api/v2/{}/game/{}", asset_type, sgdb_id))
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let raw: serde_json::Value = resp.json().ok()?;
        let data = raw.get("data")?.as_array()?;
        if data.is_empty() {
            return None;
        }
        data[0].get("url")?.as_str().map(|s| s.to_string())
    }

    /// Download all images for an SGDB game (no Steam ID).
    /// Returns (icon_path, hero_path, grid_path, logo_path, header_path).
    /// If `skip_icon` is true, the icon download is skipped and an empty string is returned for it.
    pub fn ensure_sgdb_assets(&self, sgdb_id: &str, skip_icon: bool) -> (String, String, String, String, String) {
        let dir = self.sgdb_dir(sgdb_id);
        let _ = std::fs::create_dir_all(&dir);

        // Icon — use SGDB icons API with small dimensions preference
        let icon_path = if skip_icon {
            String::new()
        } else {
            let sgdb_key = self.sgdb_api_key();
            if sgdb_key.is_empty() {
                String::new()
            } else {
                let resp = self.http
                    .get(format!("https://www.steamgriddb.com/api/v2/icons/game/{}", sgdb_id))
                    .header("Authorization", format!("Bearer {}", sgdb_key))
                    .send();
                match resp {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(raw) = r.json::<serde_json::Value>() {
                            if let Some(data) = raw.get("data").and_then(|d| d.as_array()) {
                                // Pick smallest <= 128
                                let mut best: Option<(&serde_json::Value, i64)> = None;
                                for item in data {
                                    let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
                                    if w <= 128 && (best.is_none() || w < best.unwrap().1) {
                                        best = Some((item, w));
                                    } else if best.is_none() {
                                        best = Some((item, w));
                                    }
                                }
                                if let Some(chosen) = best.map(|(item, _)| item) {
                                    if let Some(url) = chosen.get("url").and_then(|u| u.as_str()) {
                                        let ext = Path::new(url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                                        let dest = dir.join(format!("icon.{}", ext));
                                        if self.download_file(url, &dest).is_ok() {
                                            let converted = crate::parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                                            converted.to_string_lossy().into_owned()
                                        } else { String::new() }
                                    } else { String::new() }
                                } else { String::new() }
                            } else { String::new() }
                        } else { String::new() }
                    }
                    _ => String::new(),
                }
            }
        };

        // Hero
        let hero_path = if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "heroes") {
            self.fetch_image(&url, &dir.join("library_hero.jpg"))
        } else { String::new() };

        // Grid
        let grid_path = if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "grids") {
            self.fetch_image(&url, &dir.join("library_600x900.jpg"))
        } else { String::new() };

        // Logo
        let logo_path = if let Some(url) = self.fetch_sgdb_asset_url(sgdb_id, "logos") {
            self.fetch_image(&url, &dir.join("logo.png"))
        } else { String::new() };

        // Header
        let header_path = self.force_download_sgdb(sgdb_id, "header", false);

        (icon_path, hero_path, grid_path, logo_path, header_path)
    }

    pub fn ensure_assets(
        &self,
        app_id: &str,
        has_local_icon: bool,
    ) -> (String, String) {
        let dir = self.game_dir(app_id);

        // Icon — SGDB only (Steam capsule_image is a banner, not an icon)
        let icon_path = if has_local_icon {
            String::new()
        } else {
            let mut found = String::new();
            if let Some(cached) = self.find_cached_icon(app_id) {
                found = cached.to_string_lossy().into_owned();
            }
            if found.is_empty() {
                if let Some(url) = self.fetch_sgdb_icon_url(app_id) {
                    let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                    let dest = dir.join(format!("icon.{}", ext));
                    if self.download_file(&url, &dest).is_ok() {
                        let converted = crate::parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                        found = converted.to_string_lossy().into_owned();
                    }
                }
            }
            found
        };

        // Hero
        let hero_path = if let Some(cached) = self.find_cached_hero(app_id) {
            cached.to_string_lossy().into_owned()
        } else {
            self.fetch_image_fallback(
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero_2x.jpg", app_id),
                &format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", app_id),
                &dir.join("library_hero.jpg"),
            )
        };

        (icon_path, hero_path)
    }

    /// Download Steam grid assets: vertical grid (600x900), header (460x215), logo.
    /// Prioritises official Steam CDN.
    pub fn ensure_grids(&self, app_id: &str) -> (String, String, String) {
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| {
            format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix)
        };

        // Vertical grid (library capsule, 600x900)
        let grid_path = self.fetch_image_fallback(
            &cdn("library_600x900_2x.jpg"),
            &cdn("library_600x900.jpg"),
            &dir.join("library_600x900.jpg"),
        );

        // Header (horizontal, 460x215)
        let header_path = self.fetch_image_fallback(&cdn("header.jpg"), "", &dir.join("header.jpg"));

        // Logo (transparent PNG for hero overlay)
        let logo_path = self.fetch_image_fallback(&cdn("logo.png"), "", &dir.join("logo.png"));

        (grid_path, header_path, logo_path)
    }

    /// Download DLC header images for all DLCs listed in `appdetails.json`.
    /// Images are stored in `dlc/{dlc_app_id}.jpg` within the game's data dir.
    /// After downloading, replaces `image_url` with the local relative path
    /// (e.g. `dlc/2924410.jpg`) so callers reference the file directly.
    pub fn ensure_dlc_images(&self, app_id: &str, dlcs: &mut std::collections::HashMap<String, DlcInfo>) {
        let base_dir = self.game_dir(app_id);
        let dlc_dir = base_dir.join("dlc");
        let _ = std::fs::create_dir_all(&dlc_dir);

        for (_, dlc) in dlcs.iter_mut() {
            if dlc.image_url.is_empty() {
                continue;
            }
            // If image_url is already a local path, skip download
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

    /// Force-download a specific image type from Steam CDN, overwriting existing.
    /// `asset`: "hero", "grid", "header", "logo".
    pub fn force_download_steam(&self, app_id: &str, asset: &str) -> String {
        let dir = self.game_dir(app_id);
        let cdn = |suffix: &str| format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{}/{}", app_id, suffix);
        match asset {
            "hero" => {
                let dest = dir.join("library_hero.jpg");
                let _ = std::fs::remove_file(&dest);
                let r = self.fetch_image(&cdn("library_hero_2x.jpg"), &dest);
                if r.is_empty() { self.fetch_image(&cdn("library_hero.jpg"), &dest) } else { r }
            }
            "grid" => {
                let dest = dir.join("library_600x900.jpg");
                let _ = std::fs::remove_file(&dest);
                let r = self.fetch_image(&cdn("library_600x900_2x.jpg"), &dest);
                if r.is_empty() { self.fetch_image(&cdn("library_600x900.jpg"), &dest) } else { r }
            }
            "header" => {
                let dest = dir.join("header.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&cdn("header.jpg"), &dest)
            }
            "logo" => {
                let dest = dir.join("logo.png");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&cdn("logo.png"), &dest)
            }
            _ => String::new(),
        }
    }

    /// Force-download a specific image type from SGDB.
    /// `asset`: "icon", "hero", "grid", "header", "logo".
    /// `id`: Steam app ID (uses icons/steam/ endpoint) or SGDB game ID (uses game/ endpoint).
    pub fn force_download_sgdb(&self, id: &str, asset: &str, is_steam_id: bool) -> String {
        let dir = if is_steam_id { self.game_dir(id) } else { self.sgdb_dir(id) };
        let _ = std::fs::create_dir_all(&dir);
        let endpoint = match (asset, is_steam_id) {
            ("icon", true) => format!("icons/steam/{}", id),
            ("icon", false) => format!("icons/game/{}", id),
            ("hero", true) => format!("heroes/steam/{}", id),
            ("hero", false) => format!("heroes/game/{}", id),
            ("grid", true) | ("header", true) => format!("grids/steam/{}", id),
            ("grid", false) | ("header", false) => format!("grids/game/{}", id),
            ("logo", true) => format!("logos/steam/{}", id),
            ("logo", false) => format!("logos/game/{}", id),
            _ => return String::new(),
        };
        let dims: &[&str] = match asset {
            "grid" => &["600x900"],
            "header" => &["460x215", "920x430"],
            _ => &[],
        };
        let url = match self.fetch_sgdb_endpoint(&endpoint, dims) {
            Some(u) => u,
            None => return String::new(),
        };
        match asset {
            "icon" => {
                let ext = Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png");
                let dest = dir.join(format!("icon.{}", ext));
                let _ = std::fs::remove_file(&dest);
                if self.download_file(&url, &dest).is_ok() {
                    let converted = crate::parser::convert_ico_to_png(&dest).unwrap_or_else(|_| dest.clone());
                    converted.to_string_lossy().into_owned()
                } else { String::new() }
            }
            "hero" => {
                let dest = dir.join("library_hero.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "grid" => {
                let dest = dir.join("library_600x900.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "header" => {
                let dest = dir.join("header.jpg");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            "logo" => {
                let dest = dir.join("logo.png");
                let _ = std::fs::remove_file(&dest);
                self.fetch_image(&url, &dest)
            }
            _ => String::new(),
        }
    }

    fn fetch_sgdb_endpoint(&self, endpoint: &str, dimensions: &[&str]) -> Option<String> {
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() { return None; }
        let base = format!("https://www.steamgriddb.com/api/v2/{}", endpoint);
        let url = if dimensions.is_empty() {
            base
        } else {
            format!("{}?dimensions={}", base, dimensions.join(","))
        };
        let resp = self.http
            .get(&url)
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send().ok()?;
        if !resp.status().is_success() { return None; }
        let raw: serde_json::Value = resp.json().ok()?;
        let data = raw.get("data")?.as_array()?;
        if data.is_empty() { return None; }
        // For icons, pick smallest <= 128
        if endpoint.starts_with("icons") {
            let mut best: Option<(&serde_json::Value, i64)> = None;
            for item in data {
                let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
                if w <= 128 && (best.is_none() || w < best.unwrap().1) {
                    best = Some((item, w));
                } else if best.is_none() {
                    best = Some((item, w));
                }
            }
            best.map(|(item, _)| item).or(data.first())
                .and_then(|item| item.get("url")?.as_str().map(|s| s.to_string()))
        } else {
            data[0].get("url")?.as_str().map(|s| s.to_string())
        }
    }

    /// List all available assets from SGDB for a given game and asset type.
    /// `asset`: "icon", "hero", "grid", "header", "logo".
    /// `id`: Steam app ID (if is_steam_id) or SGDB game ID.
    /// `dimensions`: Optional dimension filters (e.g. `["600x900"]`, `["460x215", "920x430"]`).
    pub fn list_sgdb_assets(&self, id: &str, asset: &str, is_steam_id: bool, dimensions: &[&str]) -> Vec<SgdbAsset> {
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() {
            return Vec::new();
        }
        let endpoint = match (asset, is_steam_id) {
            ("icon", true) => format!("icons/steam/{}", id),
            ("icon", false) => format!("icons/game/{}", id),
            ("hero", true) => format!("heroes/steam/{}", id),
            ("hero", false) => format!("heroes/game/{}", id),
            ("grid", true) | ("header", true) => format!("grids/steam/{}", id),
            ("grid", false) | ("header", false) => format!("grids/game/{}", id),
            ("logo", true) => format!("logos/steam/{}", id),
            ("logo", false) => format!("logos/game/{}", id),
            _ => return Vec::new(),
        };
        let base = format!("https://www.steamgriddb.com/api/v2/{}", endpoint);
        let url = if dimensions.is_empty() {
            base
        } else {
            format!("{}?dimensions={}", base, dimensions.join(","))
        };
        let resp = match self.http
            .get(&url)
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send()
        {
            Ok(r) if r.status().is_success() => r,
            _ => return Vec::new(),
        };
        let raw: serde_json::Value = match resp.json() {
            Ok(j) => j,
            Err(_) => return Vec::new(),
        };
        let data = match raw.get("data").and_then(|d| d.as_array()) {
            Some(d) => d,
            None => return Vec::new(),
        };
        data.iter().filter_map(|item| {
            let url = item.get("url")?.as_str()?.to_string();
            let width = item.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
            let height = item.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
            let style = item.get("style").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let author = item.get("author")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mime = item.get("mime").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(SgdbAsset { url, width, height, style, author, mime })
        }).collect()
    }

    pub fn fetch_app_details(&self, app_id: &str) -> Option<AppDetails> {
        let cache_path = self.game_dir(app_id).join("appdetails.json");

        // Try cache first
        if let Ok(data) = std::fs::read(&cache_path) {
            if let Ok(d) = serde_json::from_slice::<AppDetails>(&data) {
                return Some(d);
            }
        }

        // Try nemirtingas repo first (has full data: name, languages, DLCs)
        let url = format!("{}/{}/{}.json", NEMIRTINGAS_BASE_URL, app_id, app_id);
        if let Ok(resp) = self.http.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(details) = resp.json::<AppDetails>() {
                    if !details.name.is_empty() {
                        let _ = std::fs::create_dir_all(self.game_dir(app_id));
                        if let Ok(b) = serde_json::to_vec(&details) {
                            let _ = std::fs::write(&cache_path, b);
                        }
                        return Some(details);
                    }
                }
            }
        }

        // Fall back to Steam Store API (only has name)
        let url = format!("https://store.steampowered.com/api/appdetails?appids={}", app_id);
        if let Ok(resp) = self.http.get(&url).send() {
            if let Ok(raw) = resp.json::<AppDetailsResponse>() {
                if let Some(entry) = raw.apps.get(app_id) {
                    if entry.success && !entry.data.name.is_empty() {
                        let details = AppDetails {
                            name: entry.data.name.clone(),
                            languages: Vec::new(),
                            dlcs: std::collections::HashMap::new(),
                        };
                        let _ = std::fs::create_dir_all(self.game_dir(app_id));
                        if let Ok(b) = serde_json::to_vec(&details) {
                            let _ = std::fs::write(&cache_path, b);
                        }
                        return Some(details);
                    }
                }
            }
        }

        None
    }

    fn fetch_nemirtingas_achievements(&self, app_id: &str) -> Option<Vec<NemirtingasAchievement>> {
        let url = format!("{}/{}/achievements_db.json", NEMIRTINGAS_BASE_URL, app_id);
        let resp = self.http.get(&url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let achs: Vec<NemirtingasAchievement> = resp.json().ok()?;
        if achs.is_empty() {
            None
        } else {
            Some(achs)
        }
    }

    fn fetch_steam_schema_achievements(&self, app_id: &str) -> Result<Vec<SteamSchemaAchievement>, String> {
        let api_key = self.api_key();
        if api_key.is_empty() {
            return Err("no Steam API key configured".into());
        }
        let url = format!(
            "https://api.steampowered.com/ISteamUserStats/GetSchemaForGame/v2/?key={}&appid={}&format=json",
            api_key, app_id
        );
        let resp = self.http.get(&url).send().map_err(|e| e.to_string())?;
        let raw: SteamSchemaResponse = resp.json().map_err(|e| e.to_string())?;
        let achs = raw.game.available_game_stats.achievements;
        if achs.is_empty() {
            return Err(format!("Steam returned 0 achievements for app {}", app_id));
        }
        Ok(achs)
    }

    pub fn generate_steam_settings(&self, app_id: &str) -> Result<(), String> {
        struct IconJob {
            url: String,
            dest: PathBuf,
        }

        let settings_dir = self.game_dir(app_id).join("achievements");
        let img_dir = settings_dir.join("achievement_images");
        std::fs::create_dir_all(&img_dir).map_err(|e| format!("could not create achievements dir: {}", e))?;

        let mut jobs: Vec<IconJob> = Vec::new();
        let mut out: Vec<serde_json::Value> = Vec::new();

        if let Some(nem_achs) = self.fetch_nemirtingas_achievements(app_id) {
            for a in nem_achs {
                let hidden = if a.hidden { "1" } else { "0" };
                let icon_base = Path::new(&a.icon).file_name().unwrap_or_default().to_string_lossy().into_owned();
                let icon_gray_base = Path::new(&a.icon_gray).file_name().unwrap_or_default().to_string_lossy().into_owned();
                out.push(serde_json::json!({
                    "name": a.name,
                    "displayName": pick_lang(&a.display_name),
                    "description": pick_lang(&a.description),
                    "hidden": hidden,
                    "icon": format!("achievement_images/{}", icon_base),
                    "icon_gray": format!("achievement_images/{}", icon_gray_base),
                }));
                if !a.icon.is_empty() {
                    jobs.push(IconJob {
                        url: a.icon.clone(),
                        dest: img_dir.join(&icon_base),
                    });
                }
                if !a.icon_gray.is_empty() {
                    jobs.push(IconJob {
                        url: a.icon_gray.clone(),
                        dest: img_dir.join(&icon_gray_base),
                    });
                }
            }
        } else {
            eprintln!("games-infos-datas unavailable for {}, falling back to Steam schema", app_id);
            let achs = self.fetch_steam_schema_achievements(app_id)?;
            for a in achs {
                let hidden = if a.hidden != 0 { "1" } else { "0" };
                let icon_base = Path::new(&a.icon).file_name().unwrap_or_default().to_string_lossy().into_owned();
                let icon_gray_base = Path::new(&a.icon_gray).file_name().unwrap_or_default().to_string_lossy().into_owned();
                out.push(serde_json::json!({
                    "name": a.name,
                    "displayName": a.display_name,
                    "description": a.description,
                    "hidden": hidden,
                    "icon": format!("achievement_images/{}", icon_base),
                    "icon_gray": format!("achievement_images/{}", icon_gray_base),
                }));
                if !a.icon.is_empty() {
                    jobs.push(IconJob {
                        url: a.icon.clone(),
                        dest: img_dir.join(&icon_base),
                    });
                }
                if !a.icon_gray.is_empty() {
                    jobs.push(IconJob {
                        url: a.icon_gray.clone(),
                        dest: img_dir.join(&icon_gray_base),
                    });
                }
            }
        }

        let b = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
        std::fs::write(settings_dir.join("achievements.json"), b).map_err(|e| e.to_string())?;

        // Download icons
        for j in &jobs {
            if j.dest.exists() {
                continue;
            }
            match self.http.get(&j.url).send() {
                Ok(r) if r.status().is_success() => {
                    match r.bytes() {
                        Ok(bytes) => {
                            if let Err(e) = std::fs::write(&j.dest, &bytes) {
                                eprintln!("  icon write failed {}: {}", j.url, e);
                            }
                        }
                        Err(e) => eprintln!("  icon read failed {}: {}", j.url, e),
                    }
                }
                Ok(r) => eprintln!("  icon download failed {}: HTTP {}", j.url, r.status()),
                Err(e) => eprintln!("  icon download failed {}: {}", j.url, e),
            }
        }

        println!("Generated achievements for app {}: {} achievements", app_id, out.len());
        Ok(())
    }

    /// Search the Steam Store API for games matching `term`.
    /// Returns a list of (app_id, name) pairs.
    pub fn search_steam_store(&self, term: &str) -> Vec<(String, String)> {
        let url = format!(
            "https://store.steampowered.com/api/storesearch/?term={}&l=en&cc=US",
            urlencode(term)
        );
        match self.http.get(&url).send() {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>() {
                    json.get("items")
                        .and_then(|items| items.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|item| {
                                    let id = item.get("id")?.as_i64()?.to_string();
                                    let name = item.get("name")?.as_str()?.to_string();
                                    Some((id, name))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            Err(e) => {
                eprintln!("Steam Store search failed: {}", e);
                Vec::new()
            }
        }
    }

    /// Search SteamGridDB for games matching `term`.
    /// Returns a list of (app_id, name) pairs.
    pub fn search_sgdb(&self, term: &str) -> Vec<(String, String)> {
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() {
            return Vec::new();
        }
        let url = format!(
            "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
            urlencode(term)
        );
        match self.http
            .get(&url)
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send()
        {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>() {
                    json.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|item| {
                                    let id = item.get("id")?.as_i64()?.to_string();
                                    let name = item.get("name")?.as_str()?.to_string();
                                    Some((id, name))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            Err(e) => {
                eprintln!("SGDB search failed: {}", e);
                Vec::new()
            }
        }
    }
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect()
}

