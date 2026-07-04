use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub struct SteamClient {
    api_key: Mutex<String>,
    sgdb_api_key: Mutex<String>,
    cache_dir: PathBuf,
    http: reqwest::blocking::Client,
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
struct AppDetailsResponse {
    #[serde(flatten)]
    apps: std::collections::HashMap<String, AppDetailsEntry>,
}

#[derive(Debug, Deserialize)]
struct AppDetailsEntry {
    success: bool,
    data: SteamGameDetails,
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
    percent: serde_json::Number,
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
struct NemirtingasGameInfo {
    name: String,
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
        self.cache_dir.join(app_id)
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

        let raw: GlobalAchievementsResponse = match resp.json() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Global achievements decode error for {}: {}", app_id, e);
                return None;
            }
        };

        let mut m = std::collections::HashMap::new();
        for a in raw.achievementpercentages.achievements {
            if let Some(pct) = a.percent.as_f64() {
                m.insert(a.name, pct);
            }
        }
        let _ = std::fs::create_dir_all(self.game_dir(app_id));
        if let Ok(b) = serde_json::to_vec(&m) {
            let _ = std::fs::write(&cache_path, b);
        }
        Some(m)
    }

    fn download_file(&self, url: &str, dest: &Path) -> Result<(), String> {
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
        data[0].get("url")?.as_str().map(|s| s.to_string())
    }

    pub fn ensure_assets(
        &self,
        app_id: &str,
        details: Option<&SteamGameDetails>,
        has_local_icon: bool,
    ) -> (String, String) {
        let dir = self.game_dir(app_id);

        // Icon
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
                if found.is_empty() {
                    if let Some(d) = details {
                        if !d.capsule_image.is_empty() {
                            let dest = dir.join("icon.jpg");
                            if self.download_file(&d.capsule_image, &dest).is_ok() {
                                found = dest.to_string_lossy().into_owned();
                            }
                        }
                    }
                }
            }
            found
        };

        // Hero
        let hero_path = if let Some(cached) = self.find_cached_hero(app_id) {
            cached.to_string_lossy().into_owned()
        } else {
            let hero_url = format!(
                "https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero_2x.jpg",
                app_id
            );
            let dest = dir.join("library_hero.jpg");
            let mut found = String::new();
            if self.download_file(&hero_url, &dest).is_ok() {
                if let Ok(meta) = std::fs::metadata(&dest) {
                    if meta.len() >= 200 {
                        found = dest.to_string_lossy().into_owned();
                    } else {
                        let _ = std::fs::remove_file(&dest);
                    }
                }
            }
            if found.is_empty() {
                let fallback_url = format!(
                    "https://shared.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg",
                    app_id
                );
                if self.download_file(&fallback_url, &dest).is_ok() {
                    if let Ok(meta) = std::fs::metadata(&dest) {
                        if meta.len() >= 200 {
                            found = dest.to_string_lossy().into_owned();
                        } else {
                            let _ = std::fs::remove_file(&dest);
                        }
                    }
                }
            }
            found
        };

        (icon_path, hero_path)
    }

    pub fn fetch_nemirtingas_game_name(&self, app_id: &str) -> Option<String> {
        let url = format!("{}/{}/{}.json", NEMIRTINGAS_BASE_URL, app_id, app_id);
        let resp = self.http.get(&url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let info: NemirtingasGameInfo = resp.json().ok()?;
        if info.name.is_empty() {
            None
        } else {
            Some(info.name)
        }
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

    pub fn generate_steam_settings(&self, app_id: &str, game_dir: &Path) -> Result<(), String> {
        struct IconJob {
            url: String,
            dest: PathBuf,
        }

        let settings_dir = game_dir.join("steam_settings");
        let img_dir = settings_dir.join("achievement_images");
        std::fs::create_dir_all(&img_dir).map_err(|e| format!("could not create steam_settings dir: {}", e))?;

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

        println!("Generated steam_settings for app {}: {} achievements", app_id, out.len());
        Ok(())
    }
}

