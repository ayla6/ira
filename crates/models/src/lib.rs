//! Shared domain types — dependency leaf (no crate-internal imports).
//! Used by parser, db, api, platforms, and ui modules.

pub mod achievement;
mod app_details;
mod asset_type;
mod consoles;
pub mod disc;
mod esde_consoles;
mod game;
mod game_entry;
mod group;
mod kind;
pub mod launch_config;
mod message;
pub mod session;
mod sort_mode;
mod steam_languages;
pub mod variant;

pub use achievement::{AchievementStatus, GogAchievementStatus, MergedAchievement, StringOrMap};
pub use app_details::{AppDetails, DlcInfo, UfsPathTransform, UfsRootOverride, UfsSaveFile};
pub use asset_type::{AssetType, LogoPosition};
pub use consoles::{all_consoles, find_console, ConsoleDef, CONSOLES};
pub use disc::GameDisc;
pub use game::parse_db_id;
pub use game::Game;
pub use game_entry::GameEntry;
pub use group::{Group, GroupSelection};
pub use kind::*;
pub use launch_config::{ControllerInputMode, GameLaunchConfig, WineConfig, WineProfile};
pub use message::{AppMessage, AppSender};
pub use session::PlaySession;
pub use sort_mode::SortMode;
pub use steam_languages::{steam_language_name, SteamLanguage, STEAM_LANGUAGES};
pub use variant::GameVariant;
