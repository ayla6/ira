//! Shared domain types — dependency leaf (no crate-internal imports).
//! Used by parser, db, api, platforms, and ui modules.

pub mod achievement;
mod app_details;
mod asset_type;
pub mod auto_group;
mod consoles;
pub mod disc;
mod esde_consoles;
mod fullscreen;
mod game;
mod game_entry;
mod group;
mod kind;
pub mod launch_config;
mod message;
pub mod ratings;
pub mod session;
mod group_order;
mod screenscraper;
mod group_by;
mod sort_mode;
mod steam_languages;
mod title;
pub mod variant;

pub use achievement::{AchievementStatus, GogAchievementStatus, MergedAchievement, StringOrMap};
pub use app_details::{AppDetails, DlcInfo, UfsPathTransform, UfsRootOverride, UfsSaveFile};
pub use asset_type::{AssetType, LogoPosition};
pub use auto_group::{AutoCriterion, AutoDimension, AutoGroup, AutoGroupContext, AutoLogic, AutoNode};
pub use consoles::{
    all_consoles, console_has_ra, find_console, is_compressed_switch_extension,
    ConsoleDef, COMPRESSED_SWITCH_EXTENSIONS, CONSOLES,
};
pub use disc::GameDisc;
pub use fullscreen::fullscreen_args;
pub use game::parse_db_id;
pub use game::Game;
pub use game_entry::{GameEntry, RomHashes};
pub use title::{normalize_name, normalize_phrase};
pub use group::{derived_group_id, Group, GroupSelection};
pub use group_by::GroupBy;
pub use kind::*;
pub use launch_config::{ControllerInputMode, GameLaunchConfig, WineConfig, WineProfile};
pub use message::{AppMessage, AppSender};
pub use session::PlaySession;
pub use group_order::GroupOrder;
pub use screenscraper::{
    company_tokens, screenscraper_hashes_content, screenscraper_matches_by_serial,
    scraper_console_id, screenscraper_pc_system_id, screenscraper_system_id,
    title_from_trusted_source,
    ScraperClassification, ScraperEntity, ScraperMetadata,
};
pub use sort_mode::SortMode;
pub use steam_languages::{steam_language_name, SteamLanguage, STEAM_LANGUAGES};
pub use variant::GameVariant;
