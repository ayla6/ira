//! Shared domain types — dependency leaf (no crate-internal imports).
//! Used by parser, db, api, platforms, and ui modules.

mod game;
mod game_entry;
mod group;
mod sort_mode;
pub mod achievement;
pub mod launch_config;
pub mod variant;
pub mod disc;
pub mod session;
mod message;
mod kind;
mod consoles;
mod app_details;
mod asset_type;

pub use game::Game;
pub use game::parse_db_id;
pub use consoles::{ConsoleDef, CONSOLES, find_console};
pub use app_details::{AppDetails, DlcInfo};
pub use game_entry::GameEntry;
pub use group::{Group, GroupSelection};
pub use sort_mode::SortMode;
pub use launch_config::{GameLaunchConfig, WineConfig, WineProfile};
pub use session::PlaySession;
pub use variant::GameVariant;
pub use disc::GameDisc;
pub use achievement::{AchievementStatus, MergedAchievement, StringOrMap, GogAchievementStatus};
pub use message::{AppMessage, AppSender};
pub use kind::*;
pub use asset_type::{AssetType, LogoPosition};
