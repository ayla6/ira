use crate::SteamDataClient;
use crate::types::SgdbAsset;
use crate::util::urlencode;
use ira_models::AssetType;

impl SteamDataClient {
    fn sgdb_get_json(&self, url: &str) -> Option<serde_json::Value> {
        let _s = tracing::info_span!("sgdb_get_json", url).entered();
        let sgdb_key = self.sgdb_api_key();
        if sgdb_key.is_empty() { return None; }
        let resp = self.http
            .get(url)
            .header("Authorization", format!("Bearer {}", sgdb_key))
            .send().ok()?;
        if !resp.status().is_success() { return None; }
        resp.json().ok()
    }

    pub(super) fn fetch_sgdb_icon_url(&self, app_id: &str) -> Option<String> {
        let json = self.sgdb_get_json(&format!("https://www.steamgriddb.com/api/v2/icons/steam/{}", app_id))?;
        let data = json.get("data")?.as_array()?;
        if data.is_empty() { return None; }
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

    pub(super) fn fetch_sgdb_endpoint(&self, endpoint: &str, dimensions: &[&str]) -> Option<String> {
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
        if endpoint.starts_with("icons") {
            let mut best: Option<(&serde_json::Value, i64)> = None;
            for item in data {
                let w = item.get("width").and_then(|v| v.as_i64()).unwrap_or(9999);
                if best.is_none() || (w <= 128 && w < best.unwrap().1) {
                    best = Some((item, w));
                }
            }
            best.map(|(item, _)| item).or(data.first())
                .and_then(|item| item.get("url")?.as_str().map(|s| s.to_string()))
        } else {
            data[0].get("url")?.as_str().map(|s| s.to_string())
        }
    }

    pub fn list_sgdb_assets(&self, id: &str, asset: AssetType, is_steam_id: bool, dimensions: &[&str], page: u32) -> (Vec<SgdbAsset>, bool) {
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
        let items: Vec<SgdbAsset> = data.iter().filter_map(|item| {
            let url = item.get("url")?.as_str()?.to_string();
            let thumb = item.get("thumb").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let width = item.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
            let height = item.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
            let style = item.get("style").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let author = item.get("author")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mime = item.get("mime").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(SgdbAsset { url, thumb, width, height, style, author, mime })
        }).collect();
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

pub(super) fn sgdb_endpoint(asset: AssetType, is_steam_id: bool, id: &str) -> Option<String> {
    Some(match (asset, is_steam_id) {
        (AssetType::Icon, true) => format!("icons/steam/{}", id),
        (AssetType::Icon, false) => format!("icons/game/{}", id),
        (AssetType::Hero, true) => format!("heroes/steam/{}", id),
        (AssetType::Hero, false) => format!("heroes/game/{}", id),
        (AssetType::Grid, true) | (AssetType::Header, true) => format!("grids/steam/{}", id),
        (AssetType::Grid, false) | (AssetType::Header, false) => format!("grids/game/{}", id),
        (AssetType::Logo, true) => format!("logos/steam/{}", id),
        (AssetType::Logo, false) => format!("logos/game/{}", id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sgdb_endpoint_icon_steam() {
        assert_eq!(sgdb_endpoint(AssetType::Icon, true, "12345").as_deref(), Some("icons/steam/12345"));
    }

    #[test]
    fn test_sgdb_endpoint_icon_game() {
        assert_eq!(sgdb_endpoint(AssetType::Icon, false, "12345").as_deref(), Some("icons/game/12345"));
    }

    #[test]
    fn test_sgdb_endpoint_hero_steam() {
        assert_eq!(sgdb_endpoint(AssetType::Hero, true, "67890").as_deref(), Some("heroes/steam/67890"));
    }

    #[test]
    fn test_sgdb_endpoint_hero_game() {
        assert_eq!(sgdb_endpoint(AssetType::Hero, false, "67890").as_deref(), Some("heroes/game/67890"));
    }

    #[test]
    fn test_sgdb_endpoint_grid_steam() {
        assert_eq!(sgdb_endpoint(AssetType::Grid, true, "abc").as_deref(), Some("grids/steam/abc"));
    }

    #[test]
    fn test_sgdb_endpoint_grid_game() {
        assert_eq!(sgdb_endpoint(AssetType::Grid, false, "abc").as_deref(), Some("grids/game/abc"));
    }

    #[test]
    fn test_sgdb_endpoint_header_steam() {
        assert_eq!(sgdb_endpoint(AssetType::Header, true, "def").as_deref(), Some("grids/steam/def"));
    }

    #[test]
    fn test_sgdb_endpoint_header_game() {
        assert_eq!(sgdb_endpoint(AssetType::Header, false, "def").as_deref(), Some("grids/game/def"));
    }

    #[test]
    fn test_sgdb_endpoint_logo_steam() {
        assert_eq!(sgdb_endpoint(AssetType::Logo, true, "xyz").as_deref(), Some("logos/steam/xyz"));
    }

    #[test]
    fn test_sgdb_endpoint_logo_game() {
        assert_eq!(sgdb_endpoint(AssetType::Logo, false, "xyz").as_deref(), Some("logos/game/xyz"));
    }
}
