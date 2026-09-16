use std::path::Path;

use crate::types::{
    AppDetails, DlcInfo, GlobalAchievementsResponse, SteamAppDetailsEntry, SteamCmdInfo,
    SteamCmdLaunch, SteamCmdLaunchInfo, SteamCmdResponse, SteamReviewSummary, SteamReviewsResponse,
};
use crate::util::{pick_lang, urlencode};
use crate::SteamDataClient;

/// One achievement icon to download into the achievements image directory.
struct IconJob {
    url: String,
    dest: std::path::PathBuf,
}

/// Achievement source-normalized for settings generation
/// (Nemirtingas DB entries and Steam schema entries differ only in
/// how their fields arrive).
struct AchEntry {
    name: String,
    display_name: String,
    description: String,
    hidden: bool,
    icon: String,
    icon_gray: String,
}

/// What the PC garnish takes from the store page: the short
/// description and every age-rating board that answered.
#[derive(Debug, Clone)]
pub struct StoreExtras {
    pub synopsis: String,
    pub ratings: Vec<(String, String)>,
}

#[derive(serde::Deserialize)]
struct StoreAppDetailsAnswer {
    success: bool,
    #[serde(default)]
    data: Option<StoreAppShortInfo>,
}

#[derive(serde::Deserialize)]
struct StoreAppShortInfo {
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    ratings: Option<std::collections::HashMap<String, StoreBoardRating>>,
}

#[derive(serde::Deserialize)]
struct StoreBoardRating {
    #[serde(default)]
    rating: String,
}

impl SteamDataClient {
    pub fn fetch_global_achievements(
        &self,
        app_id: &str,
    ) -> Option<std::collections::HashMap<String, f64>> {
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
        let text = match self.http_get_text(&url) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Global achievements unavailable for {}: {}", app_id, e);
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
        let raw: SteamReviewsResponse = self.http_get_json(&url)?;
        if raw.success != 1 {
            eprintln!("Steam reviews returned success=0 for {}", app_id);
            return None;
        }
        Some(raw.query_summary)
    }

    pub fn fetch_app_details(&self, app_id: &str) -> Option<AppDetails> {
        let raw = self.ensure_steamcmd_raw(app_id)?;
        let mut details = extract_app_details(&raw, app_id)?;
        self.fill_dlc_names(&mut details);
        Some(details)
    }

    /// Fetch DLC names from steamcmd for any DLCs with empty names.
    fn fill_dlc_names(&self, details: &mut AppDetails) {
        let empty_dlcs: Vec<String> = details
            .dlcs
            .iter()
            .filter(|(_, d)| d.name.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        for dlc_id in empty_dlcs {
            if let Some(raw) = self.ensure_steamcmd_raw(&dlc_id) {
                if let Some(entry) = raw.data.get(&dlc_id) {
                    if let Some(d) = details.dlcs.get_mut(&dlc_id) {
                        d.name = entry.common.name.clone();
                    }
                }
            }
        }
    }

    /// The directory holding this app's store images, taken from the
    /// store's own `appdetails` answer: new releases publish assets under
    /// hashed directories the fixed legacy paths never cover. Cached per
    /// app (misses too) because every asset fetch consults it.
    pub(crate) fn store_image_base(&self, app_id: &str) -> Option<String> {
        if let Some(base) = self.store_image_cache.lock().unwrap().get(app_id) {
            return base.clone();
        }
        let base = self.fetch_store_image_base(app_id);
        self.store_image_cache
            .lock()
            .unwrap()
            .insert(app_id.to_string(), base.clone());
        base
    }

    /// The app's own library art URLs — grid, hero and logo — from the
    /// appinfo's library_assets_full: per-release hash directories the
    /// fixed CDN paths never cover on newer releases. English art first,
    /// the 2x size before the 1x; empty when the appinfo carries none.
    pub(crate) fn store_library_urls(&self, app_id: &str, key: &str) -> Vec<String> {
        let Some(raw) = self.ensure_steamcmd_raw(app_id) else {
            return Vec::new();
        };
        let Some(relative) = raw
            .data
            .get(app_id)
            .and_then(|app| app.common.library_assets_full.as_ref())
            .and_then(|full| library_image_relative(full, key))
        else {
            return Vec::new();
        };
        let base =
            format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{app_id}");
        let mut urls = Vec::new();
        if let Some(two_x) = two_x_variant(relative) {
            urls.push(format!("{base}/{two_x}"));
        }
        urls.push(format!("{base}/{relative}"));
        urls
    }

    fn fetch_store_image_base(&self, app_id: &str) -> Option<String> {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={app_id}");
        let raw: std::collections::HashMap<String, SteamAppDetailsEntry> =
            self.http_get_json(&url)?;
        let entry = raw.get(app_id)?;
        if !entry.success {
            return None;
        }
        header_image_base(&entry.data.as_ref()?.header_image)
    }

    /// The store page's short description plus the per-board age
    /// ratings — the synopsis PC matches fall back to when
    /// ScreenScraper's entry carries none, and the age classification
    /// boards (`PEGI 18`, `ESRB M`, ...) that PC entries never have.
    /// The store answers with the boards for the requester's region
    /// plus whatever else it holds.
    pub fn fetch_store_extras(&self, app_id: &str) -> Option<StoreExtras> {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={app_id}&l=english");
        let resp = self.http.get(&url).send().ok()?;
        let raw: std::collections::HashMap<String, StoreAppDetailsAnswer> = resp.json().ok()?;
        let entry = raw.get(app_id)?;
        if !entry.success {
            return None;
        }
        let data = entry.data.as_ref()?;
        let synopsis = data.short_description.trim();
        let mut ratings: Vec<(String, String)> = data
            .ratings
            .as_ref()
            .map(|boards| {
                boards
                    .iter()
                    .filter(|(_, board)| !board.rating.trim().is_empty())
                    .map(|(board, info)| {
                        (board.to_uppercase(), info.rating.trim().to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        ratings.sort();
        Some(StoreExtras {
            synopsis: synopsis.to_string(),
            ratings,
        })
    }

    pub fn search_steam_store(&self, term: &str) -> Vec<(String, String)> {
        let url = format!(
            "https://store.steampowered.com/api/storesearch/?term={}&l=en&cc=US",
            urlencode(term)
        );
        let json = self
            .http_get_json::<serde_json::Value>(&url)
            .unwrap_or_default();
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
    }

    pub fn generate_steam_settings(&self, app_id: &str) -> Result<(), String> {
        let _s = tracing::info_span!("generate_steam_settings", app_id).entered();

        let settings_dir = self.game_dir(app_id).join("achievements");
        let img_dir = settings_dir.join("achievement_images");
        // If the directories already exist, skip creation entirely
        if img_dir.is_dir() {
            // Already exists and is a directory — nothing to do
        } else {
            // Remove any file/symlink that conflicts with the paths we need
            for dir in [&settings_dir, &img_dir] {
                if let Ok(meta) = std::fs::symlink_metadata(dir) {
                    if !meta.is_dir() {
                        let _ = std::fs::remove_file(dir);
                    }
                }
            }
            std::fs::create_dir_all(&img_dir)
                .map_err(|e| format!("could not create achievements dir: {}", e))?;
        }

        let entries: Vec<AchEntry> = match self.fetch_nemirtingas_achievements(app_id) {
            Some(nem_achs) => nem_achs
                .into_iter()
                .map(|a| AchEntry {
                    name: a.name,
                    display_name: pick_lang(&a.display_name),
                    description: pick_lang(&a.description),
                    hidden: a.hidden,
                    icon: a.icon,
                    icon_gray: a.icon_gray,
                })
                .collect(),
            None => {
                eprintln!(
                    "games-infos-datas unavailable for {}, falling back to Steam schema",
                    app_id
                );
                self.fetch_steam_schema_achievements(app_id)?
                    .into_iter()
                    .map(|a| AchEntry {
                        name: a.name,
                        display_name: a.display_name,
                        description: a.description,
                        hidden: a.hidden != 0,
                        icon: a.icon,
                        icon_gray: a.icon_gray,
                    })
                    .collect()
            }
        };

        let mut jobs: Vec<IconJob> = Vec::new();
        let mut out: Vec<serde_json::Value> = Vec::new();
        for a in entries {
            let icon_base = Path::new(&a.icon)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let icon_gray_base = Path::new(&a.icon_gray)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.push(serde_json::json!({
                "name": a.name,
                "displayName": a.display_name,
                "description": a.description,
                "hidden": if a.hidden { "1" } else { "0" },
                "icon": format!("achievement_images/{}", icon_base),
                "icon_gray": format!("achievement_images/{}", icon_gray_base),
            }));
            if !a.icon.is_empty() {
                jobs.push(IconJob {
                    url: a.icon,
                    dest: img_dir.join(&icon_base),
                });
            }
            if !a.icon_gray.is_empty() {
                jobs.push(IconJob {
                    url: a.icon_gray,
                    dest: img_dir.join(&icon_gray_base),
                });
            }
        }

        let b = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
        std::fs::write(settings_dir.join("achievements.json"), b).map_err(|e| e.to_string())?;

        self.download_icon_files(&jobs);

        eprintln!(
            "Generated achievements for app {}: {} achievements",
            app_id,
            out.len()
        );
        Ok(())
    }

    /// Download missing achievement icons, logging per-file failures. The
    /// schema still hands community images out on the retired akamai host;
    /// when that 404s, the same file is retried on the shared mirrors under
    /// the current community_assets path.
    fn download_icon_files(&self, jobs: &[IconJob]) {
        for j in jobs {
            if j.dest.exists() {
                continue;
            }
            // The retired host 404s every community image now, so the
            // rewritten mirrors go first and the schema URL is the
            // fallback — half the requests, no dead-host round trips.
            let mut urls = Vec::new();
            if let Some(modern) = modernize_community_image_url(&j.url) {
                urls.extend(mirror_variants(&modern));
            }
            urls.push(j.url.clone());
            for (index, url) in urls.iter().enumerate() {
                let last = index + 1 == urls.len();
                match self.http.get(url).send() {
                    Ok(r) if r.status().is_success() => match r.bytes() {
                        Ok(bytes) => {
                            if let Err(e) = std::fs::write(&j.dest, &bytes) {
                                eprintln!("  icon write failed {url}: {e}");
                            }
                            break;
                        }
                        Err(e) => eprintln!("  icon read failed {url}: {e}"),
                    },
                    // A non-final candidate that comes up empty stays
                    // quiet: the current-host retry is expected to succeed.
                    Ok(r) if !last => {
                        let _ = r.status();
                    }
                    Ok(r) => eprintln!("  icon download failed {url}: HTTP {}", r.status()),
                    Err(e) if !last => {
                        let _ = e;
                    }
                    Err(e) => eprintln!("  icon download failed {url}: {e}"),
                }
            }
        }
    }

    /// Read the cached steamcmd.net response for an app. Rejects unreadable
    /// files, payloads that fail to parse, and non-"success" responses —
    /// callers can treat `None` as "no usable cache".
    fn load_appdetails_cache(&self, app_id: &str) -> Option<SteamCmdResponse> {
        let data = std::fs::read(self.game_dir(app_id).join("appdetails.json")).ok()?;
        parse_appdetails_response(&data)
    }

    /// The app's clienticon hash, fetching the steamcmd appinfo on first
    /// use: callers reach this before anything else populated the cache,
    /// and the hash only exists there.
    pub fn clienticon_hash(&self, app_id: &str) -> Option<String> {
        self.cached_clienticon(app_id)
            .or_else(|| {
                self.ensure_steamcmd_raw(app_id)?;
                self.cached_clienticon(app_id)
            })
    }

    /// Resolve the clienticon hash for a Steam app from cached steamcmd.net data.
    /// Returns `None` if no valid cached data is available.
    pub fn cached_clienticon(&self, app_id: &str) -> Option<String> {
        let raw = self.load_appdetails_cache(app_id)?;
        let hash = raw.data.get(app_id)?.common.clienticon.clone();
        if hash.is_empty() {
            None
        } else {
            Some(hash)
        }
    }

    /// True when a usable (parseable, "success") appdetails cache exists,
    /// refreshing it from steamcmd.net otherwise.
    pub fn ensure_steamcmd_cache(&self, app_id: &str) -> bool {
        self.ensure_steamcmd_raw(app_id).is_some()
    }

    pub fn fetch_steamcmd_info(&self, app_id: &str) -> Option<SteamCmdInfo> {
        let raw = self.ensure_steamcmd_raw(app_id)?;
        Self::parse_steamcmd_app(&raw, app_id)
    }

    fn ensure_steamcmd_raw(&self, app_id: &str) -> Option<SteamCmdResponse> {
        if let Some(raw) = self.load_appdetails_cache(app_id) {
            return Some(raw);
        }

        let url = format!("https://api.steamcmd.net/v1/info/{}", app_id);
        let raw_bytes = match self.download_bytes(&url) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("steamcmd.net unavailable for {}: {}", url, e);
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
        let _ = std::fs::write(self.game_dir(app_id).join("appdetails.json"), &raw_bytes);

        Some(raw)
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
            oslist: entry.common.oslist.clone(),
            launches: sorted_launches(&entry.config.launch),
            logo_position: convert_pinned_position(
                &entry.common.library_assets.logo_position.pinned_position,
            ),
            logo_size: entry
                .common
                .library_assets
                .logo_position
                .width_pct
                .parse::<f64>()
                .unwrap_or(0.0)
                .round() as i32,
        })
    }
}

/// Parse steamcmd.net JSON bytes, rejecting anything that fails to parse or
/// doesn't report `"success"` status. Single gate for every cache consumer.
fn parse_appdetails_response(data: &[u8]) -> Option<SteamCmdResponse> {
    let raw: SteamCmdResponse = serde_json::from_slice(data).ok()?;
    if raw.status == "success" {
        Some(raw)
    } else {
        None
    }
}

/// Convert Steam's CamelCase pinned_position to kebab-case.
/// "BottomCenter" → "bottom-center", "BottomLeft" → "bottom-left"
fn convert_pinned_position(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push('-');
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// Collect launch entries sorted by their numeric key (0, 1, 2, …) so that
/// launch.0 — the default — comes first. `config.launch` is a JSON object
/// whose keys are stringified indices, so HashMap iteration order is random.
fn sorted_launches(
    launch: &std::collections::HashMap<String, SteamCmdLaunch>,
) -> Vec<SteamCmdLaunchInfo> {
    let mut entries: Vec<(u32, &SteamCmdLaunch)> = launch
        .iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|n| (n, v)))
        .collect();
    entries.sort_by_key(|(n, _)| *n);
    entries
        .into_iter()
        .map(|(_, l)| SteamCmdLaunchInfo {
            executable: l.executable.clone(),
            oslist: l.config.oslist.clone(),
            description: l.description.clone(),
        })
        .collect()
}

fn extract_app_details(raw: &SteamCmdResponse, app_id: &str) -> Option<AppDetails> {
    let entry = raw.data.get(app_id)?;
    let languages: Vec<String> = entry.common.supported_languages.keys().cloned().collect();

    let mut dlcs = std::collections::HashMap::new();
    if !entry.extended.listofdlc.is_empty() {
        let launch_names: std::collections::HashMap<&str, &str> = entry
            .config
            .launch
            .values()
            .filter_map(|l| {
                let dlc_id = l.config.ownsdlc.as_str();
                if dlc_id.is_empty() {
                    None
                } else {
                    Some((dlc_id, l.description.as_str()))
                }
            })
            .collect();

        for dlc_id_str in entry.extended.listofdlc.split(',') {
            let dlc_id_str = dlc_id_str.trim();
            if dlc_id_str.is_empty() {
                continue;
            }
            let app_id_val: i64 = dlc_id_str.parse().unwrap_or(0);
            let name = launch_names
                .get(dlc_id_str)
                .map(|s| s.to_string())
                .unwrap_or_default();
            dlcs.insert(
                dlc_id_str.to_string(),
                DlcInfo {
                    name,
                    app_id: app_id_val,
                    image_url: String::new(),
                    enabled: true,
                },
            );
        }
    }

    let ufs_savefiles: Vec<ira_models::UfsSaveFile> = entry
        .ufs
        .savefiles
        .values()
        .map(|sf| ira_models::UfsSaveFile {
            path: sf.path.clone(),
            root: sf.root.clone(),
            recursive: sf.recursive == "1",
        })
        .collect();

    let ufs_rootoverrides: Vec<ira_models::UfsRootOverride> = entry
        .ufs
        .rootoverrides
        .values()
        .map(|ro| ira_models::UfsRootOverride {
            os: ro.os.clone(),
            root: ro.root.clone(),
            useinstead: ro.useinstead.clone(),
            addpath: ro.addpath.clone(),
            pathtransforms: ro
                .pathtransforms
                .values()
                .map(|pt| ira_models::UfsPathTransform {
                    find: pt.find.clone(),
                    replace: pt.replace.clone(),
                })
                .collect(),
        })
        .collect();

    Some(AppDetails {
        name: entry.common.name.clone(),
        languages,
        dlcs,
        ufs_savefiles,
        ufs_rootoverrides,
    })
}

pub fn read_app_details_from_cache(path: &Path) -> Option<AppDetails> {
    let data = std::fs::read(path).ok()?;
    let raw = parse_appdetails_response(&data)?;
    let app_id = raw.data.keys().next()?.clone();
    extract_app_details(&raw, &app_id)
}

/// The steam schema still names community images on the retired
/// `steamcdn-a.akamaihd.net/steamcommunity/public` host, where they now
/// 404; the same files live on the shared mirrors under
/// `community_assets`. Returns the modern URL, or `None` for anything
/// else (those URLs are served where they point).
fn modernize_community_image_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://steamcdn-a.akamaihd.net/steamcommunity/public/")
        .or_else(|| url.strip_prefix("https://cdn.akamai.steamstatic.com/steamcommunity/public/"))?;
    Some(format!("https://shared.akamai.steamstatic.com/community_assets/{rest}"))
}

/// The shared Steam CDNs are mirrors of one path: akamai, fastly and
/// cloudflare serve identical trees, and files do go missing from single
/// mirrors. Every `shared.*` URL is therefore tried on each mirror in
/// turn; URLs on other hosts are returned unchanged.
pub(crate) fn mirror_variants(url: &str) -> Vec<String> {
    const MIRRORS: [&str; 3] = [
        "https://shared.akamai.steamstatic.com",
        "https://shared.fastly.steamstatic.com",
        "https://shared.cloudflare.steamstatic.com",
    ];
    let Some(rest) = url
        .strip_prefix("https://shared.akamai.steamstatic.com")
        .or_else(|| url.strip_prefix("https://shared.fastly.steamstatic.com"))
        .or_else(|| url.strip_prefix("https://shared.cloudflare.steamstatic.com"))
        .or_else(|| url.strip_prefix("https://shared.steamstatic.com"))
    else {
        return vec![url.to_string()];
    };
    MIRRORS.iter().map(|mirror| format!("{mirror}{rest}")).collect()
}

/// Picks the library art relative path for `key` out of the appinfo's
/// library_assets_full: English first, then any language. Values without a
/// `<hash>/` component (empty strings, bare file names) are unusable.
fn library_image_relative<'a>(
    full: &'a crate::types::SteamCmdLibraryAssetsFull,
    key: &str,
) -> Option<&'a str> {
    let entry = match key {
        "library_capsule" => full.library_capsule.as_ref(),
        "library_hero" => full.library_hero.as_ref(),
        "library_logo" => full.library_logo.as_ref(),
        _ => None,
    }?;
    let image = &entry.image;
    let usable = |relative: &String| relative.contains('/');
    image
        .get("english")
        .filter(|relative| usable(relative))
        .map(String::as_str)
        .or_else(|| image.values().find(|relative| usable(relative)).map(String::as_str))
}

/// `library_capsule.jpg` → `library_capsule_2x.jpg` — Steam serves the
/// higher-resolution variant under the derived name. `None` when the name
/// already is one or has no extension.
fn two_x_variant(file: &str) -> Option<String> {
    let (stem, ext) = file.rsplit_once('.')?;
    (!stem.ends_with("_2x")).then(|| format!("{stem}_2x.{ext}"))
}

/// `…/apps/<id>[/<hash>]/header.jpg?t=…` → `…/apps/<id>[/<hash>]`, so the
/// other store assets can be requested next to the header. `None` when the
/// URL does not name a header.jpg (then only the legacy paths remain).
fn header_image_base(header_image: &str) -> Option<String> {
    let without_query = header_image.split('?').next()?;
    let stripped = without_query.strip_suffix("/header.jpg")?;
    (!stripped.is_empty()).then(|| stripped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SteamCmdConfig;

    #[test]
    fn test_sorted_launches_orders_by_numeric_key() {
        // Keys deliberately out of order; iteration order is random in a HashMap.
        let json = r#"{
            "installdir": "MyGame",
            "launch": {
                "2": {"executable": "linux_run", "config": {"oslist": "linux"}},
                "0": {"executable": "game.exe", "config": {"oslist": "windows"}},
                "1": {"executable": "launcher.exe", "description": "Start Launcher", "config": {"oslist": "windows"}}
            }
        }"#;
        let config: SteamCmdConfig = serde_json::from_str(json).unwrap();
        let launches = sorted_launches(&config.launch);
        assert_eq!(launches.len(), 3);
        assert_eq!(launches[0].executable, "game.exe");
        assert_eq!(launches[0].oslist, "windows");
        assert_eq!(launches[1].executable, "launcher.exe");
        assert_eq!(launches[1].description, "Start Launcher");
        assert_eq!(launches[2].executable, "linux_run");
        assert_eq!(launches[2].oslist, "linux");
    }

    #[test]
    fn test_modernize_community_image_url_rewrites_retired_hosts() {
        assert_eq!(
            modernize_community_image_url(
                "https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/4659620/                 f2ea0a53ef2a93a8377fb3578f0c61e78aa9727b.jpg"
            ),
            Some(
                "https://shared.akamai.steamstatic.com/community_assets/images/apps/4659620/                 f2ea0a53ef2a93a8377fb3578f0c61e78aa9727b.jpg"
                    .to_string()
            )
        );
        assert_eq!(
            modernize_community_image_url(
                "https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/413410/abc.jpg"
            ),
            Some("https://shared.akamai.steamstatic.com/community_assets/images/apps/413410/abc.jpg".to_string())
        );
        assert_eq!(
            modernize_community_image_url("https://shared.akamai.steamstatic.com/community_assets/images/apps/413410/abc.jpg"),
            None
        );
        assert_eq!(modernize_community_image_url("https://example.com/image.jpg"), None);
    }

    #[test]
    fn test_mirror_variants_covers_all_shared_mirrors() {
        let url = "https://shared.akamai.steamstatic.com/community_assets/images/apps/4659620/a.jpg";
        let variants = mirror_variants(url);
        assert_eq!(variants.len(), 3);
        assert!(variants[0].starts_with("https://shared.akamai.steamstatic.com/"));
        assert!(variants[1].starts_with("https://shared.fastly.steamstatic.com/"));
        assert!(variants[2].starts_with("https://shared.cloudflare.steamstatic.com/"));
        // The mirror-less shared host expands to the same three.
        assert_eq!(
            mirror_variants("https://shared.steamstatic.com/x/y.png"),
            mirror_variants("https://shared.akamai.steamstatic.com/x/y.png")
        );
        // Other hosts pass through untouched.
        assert_eq!(
            mirror_variants("https://steamcdn-a.akamaihd.net/x.jpg"),
            vec!["https://steamcdn-a.akamaihd.net/x.jpg".to_string()]
        );
    }

    #[test]
    fn test_two_x_variant_inserts_before_extension() {
        assert_eq!(
            two_x_variant("library_capsule.jpg"),
            Some("library_capsule_2x.jpg".to_string())
        );
        assert_eq!(two_x_variant("logo.png"), Some("logo_2x.png".to_string()));
        assert_eq!(two_x_variant("library_hero_2x.jpg"), None);
        assert_eq!(two_x_variant("noextension"), None);
    }

    #[test]
    fn test_library_image_relative_prefers_english_usable_paths() {
        let full: crate::types::SteamCmdLibraryAssetsFull = serde_json::from_value(serde_json::json!({
            "library_capsule": {"image": {"english": "571b9e19/library_capsule.jpg"}},
            "library_hero": {"image": {"koreana": "7bf388d4/library_hero.jpg"}},
            "library_logo": {"image": {"english": ""}},
        }))
        .unwrap();

        assert_eq!(
            library_image_relative(&full, "library_capsule"),
            Some("571b9e19/library_capsule.jpg")
        );
        // No English: any usable language wins.
        assert_eq!(
            library_image_relative(&full, "library_hero"),
            Some("7bf388d4/library_hero.jpg")
        );
        // Empty and bare-name values are unusable; nothing picks them.
        assert_eq!(library_image_relative(&full, "library_logo"), None);
        assert_eq!(library_image_relative(&full, "unknown"), None);
    }

    #[test]
    fn test_header_image_base_strips_hash_and_query() {
        assert_eq!(
            header_image_base(
                "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/4659620/                 e1fc78d7ff003b772b8637de6440c541c25fa0b9/header.jpg?t=1788964897"
            ),
            Some(
                "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/4659620/                 e1fc78d7ff003b772b8637de6440c541c25fa0b9"
                    .to_string()
            )
        );
        assert_eq!(
            header_image_base("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/413410/header.jpg"),
            Some("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/413410".to_string())
        );
        assert_eq!(header_image_base("https://example.com/some/other.png"), None);
    }

    #[test]
    fn test_sorted_launches_empty() {
        let config: SteamCmdConfig = serde_json::from_str(r#"{"installdir":"x"}"#).unwrap();
        assert!(sorted_launches(&config.launch).is_empty());
    }

    #[test]
    fn test_sorted_launches_skips_non_numeric_keys() {
        let json = r#"{
            "installdir": "x",
            "launch": {
                "0": {"executable": "a.exe", "config": {}},
                "beta": {"executable": "b.exe", "config": {}}
            }
        }"#;
        let config: SteamCmdConfig = serde_json::from_str(json).unwrap();
        let launches = sorted_launches(&config.launch);
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].executable, "a.exe");
    }

    #[test]
    fn test_convert_pinned_position() {
        assert_eq!(convert_pinned_position("BottomLeft"), "bottom-left");
        assert_eq!(convert_pinned_position("BottomCenter"), "bottom-center");
        assert_eq!(convert_pinned_position("TopLeft"), "top-left");
        assert_eq!(convert_pinned_position("Center"), "center");
        assert_eq!(convert_pinned_position(""), "");
    }
}
