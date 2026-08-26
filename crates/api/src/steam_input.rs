//! Search and download Steam community controller layouts ("Steam Input"
//! configs) through the public Steam Web API, mirroring what
//! steaminputdb.com does: `IPublishedFileService/QueryFiles` with the
//! controller-config filetype under app 241100, plus the anonymous
//! `ISteamRemoteStorage/GetPublishedFileDetails` endpoint for the CDN URL.

use crate::types::{QueryFilesResponse, RemoteStorageDetailsResponse};
use crate::SteamDataClient;

/// App id Valve publishes community controller layouts under — including
/// layouts shared for non-Steam games.
pub const STEAM_INPUT_CONFIGS_APP_ID: u32 = 241100;

/// `k_EWorkshopFileTypeControllerGenerated`; the QueryFiles filetype filter
/// value used in practice for controller layouts.
const CONTROLLER_CONFIG_FILE_TYPE: u32 = 15;
/// EPublishedFileQueryType::RankedByTextSearch.
const QUERY_TYPE_TEXT_SEARCH: u32 = 25;
/// EPublishedFileQueryType::RankedByTrend.
const QUERY_TYPE_TRENDING: u32 = 4;
const TRENDING_PERIOD_DAYS: i64 = 30;
/// EPublishedFileQueryType::RankedByTotalUniqueSubscriptions.
const QUERY_TYPE_MOST_SUBSCRIBED: u32 = 10;
/// EPublishedFileQueryType::RankedByPublicationDate.
const QUERY_TYPE_NEWEST: u32 = 1;

const QUERY_FILES_URL: &str = "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/";
const FILE_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// Result ordering offered by the layout search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamLayoutSort {
    /// Ranked by text-search relevance (requires a search text).
    BestMatch,
    Trending30Days,
    MostSubscribed,
    Newest,
}

impl SteamLayoutSort {
    fn query_type(self) -> u32 {
        match self {
            Self::BestMatch => QUERY_TYPE_TEXT_SEARCH,
            Self::Trending30Days => QUERY_TYPE_TRENDING,
            Self::MostSubscribed => QUERY_TYPE_MOST_SUBSCRIBED,
            Self::Newest => QUERY_TYPE_NEWEST,
        }
    }
}

/// One community controller layout returned by the search.
#[derive(Debug, Clone)]
pub struct SteamLayout {
    pub published_file_id: String,
    pub title: String,
    pub description: String,
    /// App id of the game this layout was published for (kv tag "app"),
    /// empty when published without one.
    pub app_id: String,
    /// Controller kind from tags, e.g. "controller_ps5".
    pub controller_type: String,
    pub time_updated: i64,
    pub votes_up: i64,
    pub lifetime_subscriptions: i64,
}

impl SteamLayout {
    fn from_entry(entry: &crate::types::PublishedFileEntry) -> Option<Self> {
        if entry.publishedfileid.is_empty() || entry.result == Some(0) {
            return None;
        }
        let kv_app = entry
            .kv_tags
            .iter()
            .find(|tag| tag.key.as_deref() == Some("app"))
            .and_then(|tag| tag.value.clone())
            .unwrap_or_default();
        let controller_type = entry
            .tags
            .iter()
            .map(|tag| tag.tag.as_str())
            .find(|tag| tag.starts_with("controller_"))
            .unwrap_or_default()
            .to_string();
        Some(Self {
            published_file_id: entry.publishedfileid.clone(),
            title: entry.title.clone(),
            description: entry.file_description.clone(),
            app_id: kv_app,
            controller_type,
            time_updated: entry.time_updated.unwrap_or(0),
            votes_up: entry
                .vote_data
                .as_ref()
                .and_then(|votes| votes.votes_up)
                .or(entry.votes_up)
                .unwrap_or(0),
            lifetime_subscriptions: entry.lifetime_subscriptions.unwrap_or(0),
        })
    }
}

/// Search parameters for [`SteamDataClient::query_steam_layouts`].
#[derive(Debug, Clone)]
pub struct SteamLayoutQuery {
    pub search_text: String,
    /// Only return layouts published for this Steam app id. Layouts shared
    /// for non-Steam games carry no such tag and are excluded by this.
    pub app_id: Option<String>,
    /// Require a plain workshop tag, e.g. `controller_ps5` or
    /// `feature_gyro`; results must carry every tag listed here.
    pub required_tags: Vec<String>,
    pub page: u32,
    pub page_size: u32,
    pub sort: SteamLayoutSort,
}

/// The `input_json` payload QueryFiles expects; split out so the encoding
/// stays testable without network access.
fn query_request_body(query: &SteamLayoutQuery) -> serde_json::Value {
    let search_text = query.search_text.trim().to_string();
    // Relevance ranking is only meaningful with a text query.
    let effective_sort = if search_text.is_empty() && query.sort == SteamLayoutSort::BestMatch {
        SteamLayoutSort::Trending30Days
    } else {
        query.sort
    };
    let mut body = serde_json::json!({
        "appid": STEAM_INPUT_CONFIGS_APP_ID,
        "filetype": CONTROLLER_CONFIG_FILE_TYPE,
        "return_kv_tags": true,
        "return_metadata": true,
        "return_vote_data": true,
        "required_kv_tags": [
            {"key": "visibility", "value": "public"},
            {"key": "deleted", "value": "0"},
        ],
        "query_type": effective_sort.query_type(),
        "numperpage": query.page_size.max(1),
        "page": query.page.max(1),
    });
    if !search_text.is_empty() {
        body["search_text"] = serde_json::Value::String(search_text);
    }
    if effective_sort == SteamLayoutSort::Trending30Days {
        body["days"] = serde_json::Value::Number(TRENDING_PERIOD_DAYS.into());
    }
    if let Some(app_id) = query
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body["required_kv_tags"]
            .as_array_mut()
            .expect("required_kv_tags starts as an array")
            .push(serde_json::json!({"key": "app", "value": app_id}));
    }
    let tags: Vec<&str> = query
        .required_tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .collect();
    if !tags.is_empty() {
        body["requiredtags"] = serde_json::json!(tags);
    }
    body
}

fn parse_query_files_json(text: &str) -> Result<Vec<SteamLayout>, String> {
    let parsed: QueryFilesResponse =
        serde_json::from_str(text).map_err(|e| format!("layout search decode error: {e}"))?;
    Ok(parsed
        .response
        .published_file_details
        .iter()
        .filter_map(SteamLayout::from_entry)
        .collect())
}

impl SteamDataClient {
    /// Search Steam's community controller layouts. Requires a Steam Web API
    /// key to be configured (Valve closed anonymous QueryFiles access).
    pub fn query_steam_layouts(
        &self,
        query: &SteamLayoutQuery,
    ) -> Result<Vec<SteamLayout>, String> {
        let api_key = self.api_key();
        if api_key.is_empty() {
            return Err("no Steam Web API key configured".into());
        }
        let url = reqwest::Url::parse_with_params(
            QUERY_FILES_URL,
            &[
                ("input_json", query_request_body(query).to_string()),
                ("key", api_key),
                ("format", "json".to_string()),
            ],
        )
        .map_err(|e| e.to_string())?;
        let resp = self.http.get(url).send().map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("layout search failed: HTTP {}", status));
        }
        parse_query_files_json(&text)
    }

    /// Download a layout's raw `controller_mappings` VDF text by workshop
    /// file id. Works anonymously: GetPublishedFileDetails resolves the CDN
    /// link even without an API key.
    pub fn fetch_steam_layout_vdf(&self, published_file_id: &str) -> Result<String, String> {
        // Workshop file ids are numeric; the POST body is hand-encoded
        // because our reqwest build has no form-encoding feature.
        if !published_file_id.bytes().all(|b| b.is_ascii_digit()) || published_file_id.is_empty() {
            return Err(format!("invalid layout id {published_file_id}"));
        }
        let body = format!("itemcount=1&publishedfileids%5B0%5D={}", published_file_id);
        let resp = self
            .http
            .post(FILE_DETAILS_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .map_err(|e| format!("layout details request failed: {e}"))?;
        let parsed: RemoteStorageDetailsResponse = resp.json().map_err(|e| e.to_string())?;
        let detail = parsed
            .response
            .publishedfiledetails
            .into_iter()
            .find(|d| d.publishedfileid == published_file_id)
            .ok_or_else(|| "layout not found".to_string())?;
        if detail.result != Some(1) {
            return Err(format!(
                "layout lookup failed with result {}",
                detail.result.unwrap_or(-1)
            ));
        }
        let url = detail.file_url.trim().to_string();
        if url.is_empty() {
            return Err("layout has no downloadable file".to_string());
        }
        let vdf_resp = self
            .http
            .get(&url)
            .send()
            .map_err(|e| format!("layout download failed: {e}"))?;
        if !vdf_resp.status().is_success() {
            return Err(format!(
                "layout download failed: HTTP {}",
                vdf_resp.status()
            ));
        }
        vdf_resp.text().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_request_body_always_filters_public_and_alive() {
        let query = SteamLayoutQuery {
            search_text: String::new(),
            app_id: None,
            required_tags: Vec::new(),
            page: 1,
            page_size: 20,
            sort: SteamLayoutSort::BestMatch,
        };
        let body = query_request_body(&query);
        assert_eq!(body["appid"], STEAM_INPUT_CONFIGS_APP_ID);
        assert_eq!(body["filetype"], CONTROLLER_CONFIG_FILE_TYPE);
        assert_eq!(body["required_kv_tags"][0]["key"], "visibility");
        assert_eq!(body["required_kv_tags"][0]["value"], "public");
        assert_eq!(body["required_kv_tags"][1]["key"], "deleted");
        assert_eq!(body["required_kv_tags"][1]["value"], "0");
        assert_eq!(body["required_kv_tags"].as_array().unwrap().len(), 2);
        // Empty search falls back to trending, so relevance rank never runs
        // against an empty text filter.
        assert_eq!(body["query_type"], QUERY_TYPE_TRENDING);
        assert_eq!(body["days"], TRENDING_PERIOD_DAYS);
        assert!(body.get("search_text").is_none());
        assert!(body.get("requiredtags").is_none());
    }

    #[test]
    fn test_query_request_body_search_and_app_filter() {
        let query = SteamLayoutQuery {
            search_text: "hollow knight".into(),
            app_id: Some("367520".into()),
            required_tags: vec!["controller_ps5".into(), "feature_gyro".into()],
            page: 2,
            page_size: 50,
            sort: SteamLayoutSort::MostSubscribed,
        };
        let body = query_request_body(&query);
        assert_eq!(body["search_text"], "hollow knight");
        assert_eq!(body["query_type"], QUERY_TYPE_MOST_SUBSCRIBED);
        assert!(body.get("days").is_none());
        let tags = body["required_kv_tags"].as_array().unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[2]["key"], "app");
        assert_eq!(tags[2]["value"], "367520");
        assert_eq!(
            body["requiredtags"],
            serde_json::json!(["controller_ps5", "feature_gyro"])
        );
        assert_eq!(body["numperpage"], 50);
        assert_eq!(body["page"], 2);
    }

    #[test]
    fn test_query_request_body_ignores_blank_app_filter() {
        let query = SteamLayoutQuery {
            search_text: "x".into(),
            app_id: Some("   ".into()),
            required_tags: vec!["   ".into()],
            page: 1,
            page_size: 20,
            sort: SteamLayoutSort::Newest,
        };
        let body = query_request_body(&query);
        assert_eq!(body["query_type"], QUERY_TYPE_NEWEST);
        assert_eq!(body["required_kv_tags"].as_array().unwrap().len(), 2);
        assert!(body.get("requiredtags").is_none());
    }

    #[test]
    fn test_parse_query_files_response_extracts_layout_fields() {
        let json = r#"{
            "response": {
                "result": 1,
                "resultcount": 2,
                "publishedfiledetails": [
                    {
                        "publishedfileid": "2894527036",
                        "result": 1,
                        "title": "Various Improvements for PS5 Controller",
                        "file_description": "Gyro camera.\nSecond line.",
                        "time_updated": 1669604764,
                        "lifetime_subscriptions": 34,
                        "vote_data": {"score": 0, "votes_up": 12, "votes_down": 1},
                        "tags": [{"tag": "hasactivators"}, {"tag": "controller_ps5"}],
                        "kvtags": [{"key": "visibility", "value": "public"},
                                   {"key": "app", "value": "123456"}]
                    },
                    {
                        "publishedfileid": "999",
                        "result": 0,
                        "title": "deleted item"
                    },
                    {
                        "title": "no id at all"
                    }
                ]
            }
        }"#;
        let layouts = parse_query_files_json(json).unwrap();
        assert_eq!(layouts.len(), 1);
        let layout = &layouts[0];
        assert_eq!(layout.published_file_id, "2894527036");
        assert_eq!(layout.controller_type, "controller_ps5");
        assert_eq!(layout.app_id, "123456");
        assert_eq!(layout.votes_up, 12);
        assert_eq!(layout.lifetime_subscriptions, 34);
        // The raw description is kept for the preview page.
        assert!(layout.description.contains('\n'));
    }

    #[test]
    fn test_parse_query_files_response_rejects_garbage() {
        assert!(parse_query_files_json("<html>Forbidden</html>").is_err());
    }

    #[test]
    fn test_remote_storage_details_parse_extracts_download_url() {
        let json = r#"{
            "response": {
                "result": 1,
                "resultcount": 1,
                "publishedfiledetails": [{
                    "publishedfileid": "2894527036",
                    "result": 1,
                    "filename": "2894527036_controller_config.vdf",
                    "file_url": "https://cdn.steamusercontent.com/ugc/abc/"
                }]
            }
        }"#;
        let parsed: RemoteStorageDetailsResponse = serde_json::from_str(json).unwrap();
        let detail = parsed.response.publishedfiledetails.first().unwrap();
        assert_eq!(detail.publishedfileid, "2894527036");
        assert_eq!(detail.file_url, "https://cdn.steamusercontent.com/ugc/abc/");
    }

    #[test]
    fn test_flexible_i64_accepts_quoted_and_bare_numbers() {
        #[derive(serde::Deserialize)]
        struct Probe {
            #[serde(deserialize_with = "crate::types::flexible_i64", default)]
            value: Option<i64>,
        }
        let quoted: Probe = serde_json::from_str(r#"{"value": "42"}"#).unwrap();
        assert_eq!(quoted.value, Some(42));
        let bare: Probe = serde_json::from_str(r#"{"value": 42}"#).unwrap();
        assert_eq!(bare.value, Some(42));
        let missing: Probe = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.value, None);
    }
}
