use serde::Deserialize;

/// Numeric fields across Steam Web API responses arrive inconsistently
/// quoted (some uint64s serialize as strings, others as numbers).
pub(crate) fn flexible_i64<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid number")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "expected number or string, got {other}"
        ))),
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PublishedFileEntry {
    #[serde(default)]
    pub publishedfileid: String,
    #[serde(deserialize_with = "flexible_i64", default)]
    pub result: Option<i64>,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "file_description")]
    pub file_description: String,
    #[serde(deserialize_with = "flexible_i64", default)]
    pub time_updated: Option<i64>,
    #[serde(deserialize_with = "flexible_i64", default)]
    pub votes_up: Option<i64>,
    #[serde(deserialize_with = "flexible_i64", default)]
    pub lifetime_subscriptions: Option<i64>,
    #[serde(default)]
    pub vote_data: Option<PublishedFileVoteData>,
    #[serde(default)]
    pub tags: Vec<PublishedFileTag>,
    #[serde(default, rename = "kvtags")]
    pub kv_tags: Vec<PublishedFileKvTag>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PublishedFileTag {
    #[serde(default)]
    pub tag: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PublishedFileKvTag {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

/// Vote counts ride along as a nested object on QueryFiles results.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PublishedFileVoteData {
    #[serde(deserialize_with = "flexible_i64", default)]
    pub votes_up: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct QueryFilesResponse {
    #[serde(default)]
    pub response: QueryFilesResult,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct QueryFilesResult {
    /// Total matches across all pages.
    #[serde(deserialize_with = "flexible_i64", default)]
    pub total: Option<i64>,
    #[serde(default, rename = "publishedfiledetails")]
    pub published_file_details: Vec<PublishedFileEntry>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RemoteStorageFileDetail {
    #[serde(default)]
    pub publishedfileid: String,
    #[serde(deserialize_with = "flexible_i64", default)]
    pub result: Option<i64>,
    #[serde(default)]
    pub file_url: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RemoteStorageDetailsResult {
    #[serde(default)]
    pub publishedfiledetails: Vec<RemoteStorageFileDetail>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RemoteStorageDetailsResponse {
    #[serde(default)]
    pub response: RemoteStorageDetailsResult,
}
