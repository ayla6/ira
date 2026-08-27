use crate::types::{NemirtingasAchievement, SteamSchemaAchievement, SteamSchemaResponse};
use crate::util::NEMIRTINGAS_BASE_URL;
use crate::SteamDataClient;

impl SteamDataClient {
    pub(super) fn fetch_nemirtingas_achievements(
        &self,
        app_id: &str,
    ) -> Option<Vec<NemirtingasAchievement>> {
        let url = format!("{}/{}/achievements_db.json", NEMIRTINGAS_BASE_URL, app_id);
        let achs: Vec<NemirtingasAchievement> = self.http_get_json(&url)?;
        if achs.is_empty() {
            None
        } else {
            Some(achs)
        }
    }

    pub(super) fn fetch_steam_schema_achievements(
        &self,
        app_id: &str,
    ) -> Result<Vec<SteamSchemaAchievement>, String> {
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
        let achs = raw
            .game
            .available_game_stats
            .map(|s| s.achievements)
            .unwrap_or_default();
        Ok(achs)
    }
}
