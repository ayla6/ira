use std::path::Path;

use crate::SteamDataClient;
use crate::types::{
    AppDetails, DlcInfo, GlobalAchievementsResponse, SteamCmdInfo,
    SteamCmdResponse, SteamReviewSummary, SteamReviewsResponse,
};
use crate::util::{pick_lang, urlencode};

impl SteamDataClient {
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
        if let Some(inner) = raw.achievementpercentages {
            for a in inner.achievements {
                m.insert(a.name, a.percent);
            }
        }
        let _ = std::fs::create_dir_all(self.game_dir(app_id));
        if let Ok(b) = serde_json::to_vec(&m) {
            let _ = std::fs::write(&cache_path, b);
        }
        Some(m)
    }

    pub fn fetch_steam_reviews(&self, app_id: &str) -> Option<SteamReviewSummary> {
        let url = format!(
            "https://store.steampowered.com/appreviews/{}?json=1&num_per_page=0&purchase_type=all&language=all",
            app_id
        );
        let resp = match self.http.get(&url).send() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Steam reviews unavailable for {}: {}", app_id, e);
                return None;
            }
        };
        let raw: SteamReviewsResponse = match resp.json() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Steam reviews decode error for {}: {}", app_id, e);
                return None;
            }
        };
        if raw.success != 1 {
            eprintln!("Steam reviews returned success=0 for {}", app_id);
            return None;
        }
        Some(raw.query_summary)
    }

    pub fn fetch_app_details(&self, app_id: &str) -> Option<AppDetails> {
        let raw = self.ensure_steamcmd_raw(app_id)?;
        Self::extract_app_details(&raw, app_id)
    }

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

    pub fn generate_steam_settings(&self, app_id: &str) -> Result<(), String> {
        let _s = tracing::info_span!("generate_steam_settings", app_id).entered();
        struct IconJob {
            url: String,
            dest: std::path::PathBuf,
        }

        let settings_dir = self.game_dir(app_id).join("achievements");
        let img_dir = settings_dir.join("achievement_images");
        if img_dir.exists() && !img_dir.is_dir() {
            let _ = std::fs::remove_file(&img_dir);
        }
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

        eprintln!("Generated achievements for app {}: {} achievements", app_id, out.len());
        Ok(())
    }

    /// Resolve the clienticon hash for a Steam app from cached steamcmd.net data.
    /// Returns `None` if no cached data is available.
    pub fn cached_clienticon(&self, app_id: &str) -> Option<String> {
        let cached = self.game_dir(app_id).join("appdetails.json");
        let data = std::fs::read(&cached).ok()?;
        let raw: SteamCmdResponse = serde_json::from_slice(&data).ok()?;
        let entry = raw.data.get(app_id)?;
        if entry.common.clienticon.is_empty() { None } else { Some(entry.common.clienticon.clone()) }
    }

    pub fn cached_icon_hash(&self, app_id: &str) -> Option<String> {
        let cached = self.game_dir(app_id).join("appdetails.json");
        let data = std::fs::read(&cached).ok()?;
        let raw: SteamCmdResponse = serde_json::from_slice(&data).ok()?;
        let entry = raw.data.get(app_id)?;
        if entry.common.icon.is_empty() { None } else { Some(entry.common.icon.clone()) }
    }

    pub fn ensure_steamcmd_cache(&self, app_id: &str) -> bool {
        let cache_path = self.game_dir(app_id).join("appdetails.json");
        if cache_path.is_file() {
            return true;
        }
        self.fetch_steamcmd_info(app_id).is_some()
    }

    pub fn fetch_steamcmd_info(&self, app_id: &str) -> Option<SteamCmdInfo> {
        let raw = self.ensure_steamcmd_raw(app_id)?;
        Self::parse_steamcmd_app(&raw, app_id)
    }

    fn ensure_steamcmd_raw(&self, app_id: &str) -> Option<SteamCmdResponse> {
        let cache_path = self.game_dir(app_id).join("appdetails.json");

        if let Ok(data) = std::fs::read(&cache_path) {
            if let Ok(raw) = serde_json::from_slice::<SteamCmdResponse>(&data) {
                if raw.status == "success" {
                    return Some(raw);
                }
            }
        }

        let url = format!("https://api.steamcmd.net/v1/info/{}", app_id);
        let resp = match self.http.get(&url).send() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("steamcmd.net unavailable for {}: {}", app_id, e);
                return None;
            }
        };

        let raw_bytes = match resp.bytes() {
            Ok(b) => b.to_vec(),
            Err(e) => {
                eprintln!("steamcmd.net read error for {}: {}", app_id, e);
                return None;
            }
        };

        let raw: SteamCmdResponse = match serde_json::from_slice(&raw_bytes) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("steamcmd.net decode error for {}: {}", app_id, e);
                return None;
            }
        };

        if raw.status != "success" {
            eprintln!("steamcmd.net returned status={} for {}", raw.status, app_id);
            return None;
        }

        let _ = std::fs::create_dir_all(self.game_dir(app_id));
        let _ = std::fs::write(&cache_path, &raw_bytes);

        serde_json::from_slice(&raw_bytes).ok()
    }

    fn parse_steamcmd_app(raw: &SteamCmdResponse, app_id: &str) -> Option<SteamCmdInfo> {
        let entry = raw.data.get(app_id)?;
        Some(SteamCmdInfo {
            name: entry.common.name.clone(),
            release_timestamp: entry.steam_release_date.parse().unwrap_or(0),
            metacritic_score: entry.common.metacritic_score.parse().unwrap_or(-1),
            review_percentage: entry.common.review_percentage.parse().unwrap_or(-1),
            review_score: entry.common.review_score.parse().unwrap_or(-1),
            developer: entry.extended.developer.clone(),
            publisher: entry.extended.publisher.clone(),
            homepage: entry.extended.homepage.clone(),
            install_dir: entry.config.installdir.clone(),
            clienticon: entry.common.clienticon.clone(),
            icon: entry.common.icon.clone(),
        })
    }

    fn extract_app_details(raw: &SteamCmdResponse, app_id: &str) -> Option<AppDetails> {
        extract_app_details(raw, app_id)
    }
}

fn extract_app_details(raw: &SteamCmdResponse, app_id: &str) -> Option<AppDetails> {
    let entry = raw.data.get(app_id)?;
    let languages: Vec<String> = entry.common.supported_languages.keys()
        .cloned()
        .collect();

    let mut dlcs = std::collections::HashMap::new();
    if !entry.extended.listofdlc.is_empty() {
        let launch_names: std::collections::HashMap<&str, &str> = entry.config.launch.values()
            .filter_map(|l| {
                let dlc_id = l.config.ownsdlc.as_str();
                if dlc_id.is_empty() { None } else { Some((dlc_id, l.description.as_str())) }
            })
            .collect();

        for dlc_id_str in entry.extended.listofdlc.split(',') {
            let dlc_id_str = dlc_id_str.trim();
            if dlc_id_str.is_empty() { continue; }
            let app_id_val: i64 = dlc_id_str.parse().unwrap_or(0);
            let name = launch_names.get(dlc_id_str).map(|s| s.to_string()).unwrap_or_default();
            dlcs.insert(dlc_id_str.to_string(), DlcInfo {
                name,
                app_id: app_id_val,
                image_url: String::new(),
                enabled: true,
            });
        }
    }

    Some(AppDetails {
        name: entry.common.name.clone(),
        languages,
        dlcs,
    })
}

pub fn read_app_details_from_cache(path: &Path) -> Option<AppDetails> {
    let data = std::fs::read(path).ok()?;
    let raw: SteamCmdResponse = serde_json::from_slice(&data).ok()?;
    let app_id = raw.data.keys().next()?.clone();
    extract_app_details(&raw, &app_id)
}
