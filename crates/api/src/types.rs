pub(crate) mod achievements;
pub(crate) mod input;
pub(crate) mod steamcmd;

pub use achievements::SteamReviewSummary;
pub(crate) use achievements::{
    GlobalAchievementsResponse, NemirtingasAchievement, SteamAppDetailsEntry, SteamReviewsResponse,
    SteamSchemaAchievement, SteamSchemaResponse,
};
pub(crate) use input::{PublishedFileEntry, QueryFilesResponse, RemoteStorageDetailsResponse};
pub use steamcmd::{SteamCmdInfo, SteamCmdLaunchInfo};
pub(crate) use steamcmd::{SteamCmdLaunch, SteamCmdLibraryAssetsFull, SteamCmdResponse};

pub use ira_models::{AppDetails, DlcInfo};

/// Which SGDB endpoint family a fetch targets: an explicit SGDB id
/// (`…/game/{id}`) or a Steam app id (`…/steam/{id}`, for games matched by
/// their Steam entry).
#[derive(Clone, Copy)]
pub enum SgdbId<'a> {
    Game(&'a str),
    Steam(&'a str),
}

impl<'a> SgdbId<'a> {
    pub fn as_str(self) -> &'a str {
        match self {
            SgdbId::Game(id) | SgdbId::Steam(id) => id,
        }
    }

    pub fn is_steam(self) -> bool {
        matches!(self, SgdbId::Steam(_))
    }
}

#[derive(Clone)]
pub struct SgdbAsset {
    pub url: String,
    pub thumb: String,
    pub width: i64,
    pub height: i64,
    pub style: String,
    pub author: String,
    pub author_steam64: String,
    pub mime: String,
}
