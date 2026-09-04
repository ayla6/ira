use crate::types::SgdbAsset;
use crate::util::urlencode;
use crate::SteamDataClient;
use ira_models::AssetType;

impl SteamDataClient {
    fn sgdb_get_json(&self, url: &str) -> Option<serde_json::Value> {
        let _s = tracing::info_span!("sgdb_get_json", url).entered();
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() {
            return None;
        }
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json().ok()
    }

    /// Candidate asset URLs for `endpoint`, best first. SGDB serves its own
    /// popularity order and the API exposes no score/likes fields, so the
    /// auto-pick is simply the first entry left after filtered users are
    /// removed; icons keep the narrowest-at-most-128px rule instead. Empty
    /// when the request fails or nothing is left.
    pub(super) fn fetch_sgdb_candidates(
        &self,
        endpoint: &str,
        dimensions: &[&str],
    ) -> Vec<String> {
        let base = format!("https://www.steamgriddb.com/api/v2/{}", endpoint);
        let url = if dimensions.is_empty() {
            base
        } else {
            format!("{}?dimensions={}", base, dimensions.join(","))
        };
        let Some(json) = self.sgdb_get_json(&url) else {
            return Vec::new();
        };
        let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
            return Vec::new();
        };
        let items = not_filtered(&data.iter().collect::<Vec<_>>(), &self.sgdb_filtered_users());
        let items = not_style_filtered(&items, &self.sgdb_filtered_styles());
        if items.is_empty() {
            return Vec::new();
        }
        if endpoint.starts_with("icons") {
            best_icon_item(&items)
                .and_then(|item| item.get("url"))
                .and_then(|u| u.as_str())
                .map(|u| vec![u.to_string()])
                .unwrap_or_default()
        } else {
            items
                .iter()
                .filter_map(|item| item.get("url").and_then(|u| u.as_str()))
                .map(|u| u.to_string())
                .collect()
        }
    }

    pub fn list_sgdb_assets(
        &self,
        id: &str,
        asset: AssetType,
        is_steam_id: bool,
        dimensions: &[&str],
        page: u32,
    ) -> (Vec<SgdbAsset>, bool) {
        let _s = tracing::info_span!("list_sgdb_assets", id, asset = %asset, page).entered();
        let endpoint = match sgdb_endpoint(asset, is_steam_id, id) {
            Some(e) => e,
            None => return (Vec::new(), false),
        };
        let base = format!("https://www.steamgriddb.com/api/v2/{}", endpoint);
        let mut params: Vec<String> = Vec::new();
        if !dimensions.is_empty() {
            params.push(format!("dimensions={}", dimensions.join(",")));
        }
        if page > 0 {
            params.push(format!("page={}", page));
        }
        let url = if params.is_empty() {
            base
        } else {
            format!("{}?{}", base, params.join("&"))
        };
        let json = match self.sgdb_get_json(&url) {
            Some(j) => j,
            None => return (Vec::new(), false),
        };
        let total: u64 = json.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let data = match json.get("data").and_then(|d| d.as_array()) {
            Some(d) => d,
            None => return (Vec::new(), false),
        };
        let items: Vec<SgdbAsset> = data
            .iter()
            .filter_map(|item| {
                let url = item.get("url")?.as_str()?.to_string();
                let thumb = item
                    .get("thumb")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let width = item.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
                let height = item.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
                let style = item
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let author = item
                    .get("author")
                    .and_then(|a| a.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let author_steam64 = item
                    .get("author")
                    .and_then(|a| a.get("steam64"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mime = item
                    .get("mime")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SgdbAsset {
                    url,
                    thumb,
                    width,
                    height,
                    style,
                    author,
                    author_steam64,
                    mime,
                })
            })
            .collect();
        // Filtered authors and styles stay visible in the manual picker but
        // sink to the bottom, so they are only reached when nothing else
        // fits.
        let (visible, sunk): (Vec<SgdbAsset>, Vec<SgdbAsset>) = items.into_iter().partition(|a| {
            !self.user_filtered(&a.author) && !self.style_filtered(&a.style)
        });
        let items = visible.into_iter().chain(sunk).collect();
        let has_more = (page as u64 + 1) * 30 < total;
        (items, has_more)
    }

    pub fn search_sgdb(&self, term: &str) -> Vec<(String, String)> {
        let url = format!(
            "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
            urlencode(term)
        );
        let json = match self.sgdb_get_json(&url) {
            Some(j) => j,
            None => return Vec::new(),
        };
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
    }
}

/// An SGDB response item's author display name, empty when absent.
fn item_author(item: &serde_json::Value) -> &str {
    item.get("author")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Items not authored by a filtered user, preserving SGDB's order.
fn not_filtered<'a>(
    items: &[&'a serde_json::Value],
    filtered: &[String],
) -> Vec<&'a serde_json::Value> {
    items
        .iter()
        .copied()
        .filter(|item| {
            !filtered
                .iter()
                .any(|user| user.eq_ignore_ascii_case(item_author(item)))
        })
        .collect()
}

/// Items whose style is not filtered out, preserving SGDB's order.
fn not_style_filtered<'a>(
    items: &[&'a serde_json::Value],
    filtered: &[String],
) -> Vec<&'a serde_json::Value> {
    items
        .iter()
        .copied()
        .filter(|item| {
            let style = item.get("style").and_then(|v| v.as_str()).unwrap_or("");
            !filtered.iter().any(|s| s.eq_ignore_ascii_case(style))
        })
        .collect()
}

/// Best-icon rule: prefer the narrowest candidate at most 128px wide;
/// fall back to the first entry otherwise.
fn best_icon_item<'a>(items: &[&'a serde_json::Value]) -> Option<&'a serde_json::Value> {
    let mut best: Option<&serde_json::Value> = None;
    let mut best_w = i64::MAX;
    for item in items {
        let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
        if best.is_none() || (w <= 128 && w < best_w) {
            best = Some(item);
            best_w = w;
        }
    }
    best.or_else(|| items.first().copied())
}

pub(super) fn sgdb_endpoint(asset: AssetType, is_steam_id: bool, id: &str) -> Option<String> {
    Some(match (asset, is_steam_id) {
        (AssetType::Icon, true) => format!("icons/steam/{}", id),
        (AssetType::Icon, false) => format!("icons/game/{}", id),
        (AssetType::Hero, true) => format!("heroes/steam/{}", id),
        (AssetType::Hero, false) => format!("heroes/game/{}", id),
        (AssetType::Grid, true)
        | (AssetType::Header, true)
        | (AssetType::Square, true) => format!("grids/steam/{}", id),
        (AssetType::Grid, false)
        | (AssetType::Header, false)
        | (AssetType::Square, false) => format!("grids/game/{}", id),
        (AssetType::Logo, true) => format!("logos/steam/{}", id),
        (AssetType::Logo, false) => format!("logos/game/{}", id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn grid(id: i64, author: &str, width: i64) -> serde_json::Value {
        json!({
            "id": id,
            "url": format!("https://cdn.example/{id}.png"),
            "width": width,
            "author": { "name": author },
        })
    }

    #[test]
    fn test_not_filtered_drops_filtered_authors_case_insensitive() {
        let data = [
            grid(1, "Kept", 600),
            grid(2, "BadGuy", 600),
            grid(3, "kept", 600),
            grid(4, "badguy", 600),
        ];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        let filtered = vec!["badguy".to_string()];
        let items = not_filtered(&refs, &filtered);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item["id"] != 2 && item["id"] != 4));
    }

    #[test]
    fn test_not_filtered_keeps_items_without_author() {
        let data = [json!({"id": 1, "url": "https://cdn.example/1.png"})];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        assert_eq!(not_filtered(&refs, &["x".to_string()]).len(), 1);
    }

    #[test]
    fn test_not_filtered_preserves_sgdb_order() {
        let data = [grid(1, "a", 600), grid(2, "bad", 600), grid(3, "b", 1024)];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        let items = not_filtered(&refs, &["bad".to_string()]);
        let ids: Vec<i64> = items.iter().map(|item| item["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn test_not_style_filtered_drops_named_styles() {
        let data = [
            json!({"id": 1, "url": "https://cdn.example/1.png", "style": "alternate"}),
            json!({"id": 2, "url": "https://cdn.example/2.png", "style": "blurred"}),
            json!({"id": 3, "url": "https://cdn.example/3.png", "style": "material"}),
        ];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        let items = not_style_filtered(&refs, &["blurred".to_string()]);
        let ids: Vec<i64> = items.iter().map(|item| item["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn test_best_icon_item_prefers_narrowest_small_icon() {
        let data = [grid(1, "a", 512), grid(2, "b", 64), grid(3, "c", 128)];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        assert_eq!(best_icon_item(&refs).unwrap()["id"], 2);
    }

    #[test]
    fn test_best_icon_item_falls_back_to_first_when_all_large() {
        let data = [grid(1, "a", 512), grid(2, "b", 256)];
        let refs: Vec<&serde_json::Value> = data.iter().collect();
        assert_eq!(best_icon_item(&refs).unwrap()["id"], 1);
    }

    #[test]
    fn test_sgdb_endpoint_icon_steam() {
        assert_eq!(
            sgdb_endpoint(AssetType::Icon, true, "12345").as_deref(),
            Some("icons/steam/12345")
        );
    }

    #[test]
    fn test_sgdb_endpoint_icon_game() {
        assert_eq!(
            sgdb_endpoint(AssetType::Icon, false, "12345").as_deref(),
            Some("icons/game/12345")
        );
    }

    #[test]
    fn test_sgdb_endpoint_hero_steam() {
        assert_eq!(
            sgdb_endpoint(AssetType::Hero, true, "67890").as_deref(),
            Some("heroes/steam/67890")
        );
    }

    #[test]
    fn test_sgdb_endpoint_hero_game() {
        assert_eq!(
            sgdb_endpoint(AssetType::Hero, false, "67890").as_deref(),
            Some("heroes/game/67890")
        );
    }

    #[test]
    fn test_sgdb_endpoint_grid_steam() {
        assert_eq!(
            sgdb_endpoint(AssetType::Grid, true, "abc").as_deref(),
            Some("grids/steam/abc")
        );
    }

    #[test]
    fn test_sgdb_endpoint_grid_game() {
        assert_eq!(
            sgdb_endpoint(AssetType::Grid, false, "abc").as_deref(),
            Some("grids/game/abc")
        );
    }

    #[test]
    fn test_sgdb_endpoint_header_steam() {
        assert_eq!(
            sgdb_endpoint(AssetType::Header, true, "def").as_deref(),
            Some("grids/steam/def")
        );
    }

    #[test]
    fn test_sgdb_endpoint_header_game() {
        assert_eq!(
            sgdb_endpoint(AssetType::Header, false, "def").as_deref(),
            Some("grids/game/def")
        );
    }

    #[test]
    fn test_sgdb_endpoint_logo_steam() {
        assert_eq!(
            sgdb_endpoint(AssetType::Logo, true, "xyz").as_deref(),
            Some("logos/steam/xyz")
        );
    }

    #[test]
    fn test_sgdb_endpoint_logo_game() {
        assert_eq!(
            sgdb_endpoint(AssetType::Logo, false, "xyz").as_deref(),
            Some("logos/game/xyz")
        );
    }
}
