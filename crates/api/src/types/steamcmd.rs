// ── steamcmd.net API ────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level response from `api.steamcmd.net/v1/info/{appid}`.
#[derive(Debug, Deserialize)]
pub(crate) struct SteamCmdResponse {
    pub data: HashMap<String, SteamCmdApp>,
    pub status: String,
}

/// Per-app data from steamcmd.net.
#[derive(Debug, Deserialize)]
pub(crate) struct SteamCmdApp {
    #[serde(default)]
    pub common: SteamCmdCommon,
    #[serde(default)]
    pub extended: SteamCmdExtended,
    #[serde(default)]
    pub config: SteamCmdConfig,
    #[serde(default)]
    pub ufs: SteamCmdUfs,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdCommon {
    #[serde(default)]
    pub name: String,
    /// Unix-seconds string, e.g. "1100592000". Lives under `common` in
    /// the appinfo shape steamcmd.net mirrors — never at the app level.
    #[serde(default)]
    pub steam_release_date: String,
    #[serde(default)]
    pub metacritic_score: String,
    #[serde(default)]
    pub review_percentage: String,
    #[serde(default)]
    pub review_score: String,
    #[serde(default)]
    pub clienticon: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub oslist: String,
    #[serde(default)]
    pub supported_languages: HashMap<String, SteamCmdLanguage>,
    #[serde(default)]
    pub library_assets: SteamCmdLibraryAssets,
    /// Steam's own library art pointers: per-language `<hash>/<file>`
    /// paths relative to the app's store_item_assets directory. Newer
    /// releases publish grid, hero and logo only under these hash
    /// directories — the fixed CDN paths never cover them.
    #[serde(default)]
    pub library_assets_full: Option<SteamCmdLibraryAssetsFull>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLibraryAssets {
    #[serde(default)]
    pub logo_position: SteamCmdLogoPosition,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLibraryAssetsFull {
    #[serde(default)]
    pub library_capsule: Option<SteamCmdLibraryImage>,
    #[serde(default)]
    pub library_hero: Option<SteamCmdLibraryImage>,
    #[serde(rename = "library_logo", default)]
    pub library_logo: Option<SteamCmdLibraryImage>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLibraryImage {
    /// `<hash>/<file>` per language.
    #[serde(default)]
    pub image: HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLogoPosition {
    #[serde(default)]
    pub pinned_position: String,
    #[serde(default)]
    pub width_pct: String,
    #[serde(default)]
    pub _height_pct: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdExtended {
    #[serde(default)]
    pub developer: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub listofdlc: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLanguage {
    #[serde(default)]
    pub _supported: String,
    #[serde(default)]
    pub _full_audio: String,
    #[serde(default)]
    pub _subtitles: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdConfig {
    #[serde(default)]
    pub installdir: String,
    #[serde(default)]
    pub launch: HashMap<String, SteamCmdLaunch>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLaunch {
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub config: SteamCmdLaunchConfig,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdLaunchConfig {
    #[serde(default)]
    pub ownsdlc: String,
    #[serde(default)]
    pub oslist: String,
}

// ── UFS (Unified File System) ───────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdUfs {
    #[serde(default)]
    pub savefiles: HashMap<String, SteamCmdSaveFile>,
    #[serde(default)]
    pub rootoverrides: HashMap<String, SteamCmdRootOverride>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdSaveFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub recursive: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdRootOverride {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub useinstead: String,
    #[serde(default)]
    pub addpath: String,
    #[serde(default)]
    pub pathtransforms: HashMap<String, SteamCmdPathTransform>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SteamCmdPathTransform {
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub replace: String,
}

// ── Parsed app details ──────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SteamCmdInfo {
    pub name: String,
    pub release_timestamp: i64,
    pub metacritic_score: i64,
    pub review_percentage: i64,
    pub review_score: i64,
    pub developer: String,
    pub publisher: String,
    pub homepage: String,
    pub install_dir: String,
    /// Client icon hash (used for `steam/games/<hash>.ico` and CDN).
    pub clienticon: String,
    /// Community icon hash (used for CDN `apps/<appid>/<hash>.ico`).
    pub icon: String,
    /// Comma-separated OS list from `common.oslist` (e.g. "windows,macos,linux").
    #[serde(default)]
    pub oslist: String,
    /// Sorted launch entries (launch.0 is the default). Empty if none.
    #[serde(default)]
    pub launches: Vec<SteamCmdLaunchInfo>,
    /// Logo position from Steam library_assets (kebab-case, e.g. "bottom-left").
    #[serde(default)]
    pub logo_position: String,
    /// Logo width percentage from Steam (0 if not available).
    #[serde(default)]
    pub logo_size: i32,
}

/// A single launch entry from steamcmd.net `config.launch`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SteamCmdLaunchInfo {
    /// Executable path relative to the install directory.
    pub executable: String,
    /// OS list for this launch (e.g. "windows", "linux").
    pub oslist: String,
    /// Optional human-readable description (e.g. "Start Launcher").
    pub description: String,
}
