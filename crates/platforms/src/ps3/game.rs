use std::path::Path;

use crate::ps4::parse_trop_xml;
use ira_models::{Game, MergedAchievement};

use super::discovery::Rpcs3Game;
use super::paths::{
    persistent_settings_path_in, rpcs3_config_dir, trophy_conf_path_in, trophy_icon_path_in,
    tropusr_path_in,
};
use super::persistent::{ms_to_hours, parse_persistent_settings};
use super::tropusr::parse_tropusr;

/// Metadata for an RPCS3 game, sourced from the DB entry.
/// Mirrors `ShadPS4GameMeta` but drops the shadPS4 version selector
/// (RPCS3 ships as a single binary — no version dropdown needed).
pub struct Rpcs3GameMeta {
    pub title: String,
    pub hidden: bool,
    pub logo_position: String,
    pub logo_size: i32,
    pub sort_title: String,
    pub sgdb_id: String,
    pub last_played: i64,
}

/// Load an RPCS3 game as a `Game` struct with achievements.
///
/// Trophy definitions come from TROPCONF.SFM (XML, same schema as PS4's TROP.XML).
/// Unlock state comes from TROPUSR.DAT (binary TLV format).
/// Playtime and last-played come from persistent_settings.dat (INI).
pub fn load_rpcs3_game(game: &Rpcs3Game, db_id: i64, meta: &Rpcs3GameMeta, save_dir: &str) -> Game {
    let npwr_id = &game.npwr_id;
    let serial = &game.serial;

    // Playtime + last-played from persistent_settings.dat.
    let persistent = parse_persistent_settings(&persistent_settings_path_in(&game.config_dir));
    let playtime = persistent
        .playtime_ms
        .get(serial)
        .map(|ms| ms_to_hours(*ms))
        .unwrap_or(0.0);
    let last_played = persistent
        .last_played
        .get(serial)
        .copied()
        .unwrap_or(meta.last_played);

    let mut out = Game {
        app_id: npwr_id.clone(),
        kind: ira_models::GameKind::Ps3,
        trophy_source: ira_models::TrophySource::Empty,
        platform_id: serial.clone(),
        db_id,
        name: if meta.title.is_empty() {
            game.title.clone()
        } else {
            meta.title.clone()
        },
        name_lower: String::new(),
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        square_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden: meta.hidden,
        slug: serial.clone(),
        playtime,
        last_played,
        logo_position: meta.logo_position.clone(),
        logo_size: meta.logo_size,
        manual_unmatch: false,
        sort_title: meta.sort_title.clone(),
        game_path: game.game_path.to_string_lossy().into_owned(),
        sgdb_id: meta.sgdb_id.clone(),
        shadps4_version: String::new(),
        release_date: String::new(),
        release_timestamp: 0,
        metacritic_score: -1,
        steam_review_score: -1,
        steam_review_count: 0,
        ra_core: String::new(),
        emulator_override: String::new(),
        rom_path: String::new(),
        game_folder: String::new(),
        variant_id: None,
    };
    out.name_lower = out.name.to_lowercase();

    // Default icon: copy the game's ICON0.PNG to data/ps3/{NPWR}/ and convert
    // to WebP. Only copies if no icon (webp/jpg) already exists in the data dir.
    let ps3_data_dir = Path::new(save_dir).join("data").join("ps3").join(npwr_id);
    if ira_parser::find_image_file(&ps3_data_dir, "icon").is_none() {
        let default_icon = game.game_path.join("ICON0.PNG");
        if default_icon.is_file() {
            let _ = std::fs::create_dir_all(&ps3_data_dir);
            let tmp_png = ps3_data_dir.join("icon.png");
            let _ = std::fs::copy(&default_icon, &tmp_png);
            ira_parser::convert_to_lossless_webp(&tmp_png);
        }
    }

    // Image paths — always use data/ps3/{NPWR_ID}/ to match game_data_dir/
    // entry_data_dir and the background SGDB enrichment download target.
    let image_dir = ps3_data_dir.clone();

    // Fallback to the emulator's original ICON0.PNG if no icon in data dir.
    if ira_parser::find_image_file(&image_dir, "icon").is_none() {
        let default_icon = game.game_path.join("ICON0.PNG");
        if default_icon.is_file() {
            out.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }

    ira_parser::populate_image_paths(&image_dir, &mut out);

    // Build achievements from TROPCONF.SFM definitions + TROPUSR.DAT unlock state.
    out.achievements = load_ps3_trophies_in(npwr_id, &game.config_dir);
    out.total_count = out.achievements.len();
    out.earned_count = out.achievements.iter().filter(|a| a.earned).count();

    out
}

/// Load PS3 trophy achievements from TROPCONF.SFM (definitions) + TROPUSR.DAT (unlock state).
/// Returns an empty vector if the NPWR ID is empty or the files are missing.
pub fn load_ps3_trophies(npwr_id: &str) -> Vec<MergedAchievement> {
    load_ps3_trophies_in(npwr_id, &rpcs3_config_dir())
}

pub fn load_ps3_trophies_in(npwr_id: &str, config_dir: &Path) -> Vec<MergedAchievement> {
    if npwr_id.is_empty() {
        return Vec::new();
    }

    let defs = parse_trop_xml(&trophy_conf_path_in(config_dir, npwr_id));
    let unlock_states = parse_tropusr(&tropusr_path_in(config_dir, npwr_id)).unwrap_or_default();

    let mut achievements = Vec::new();
    for def in &defs {
        // TROPCONF.SFM trophy IDs are zero-padded strings ("000", "001", ...).
        // TROPUSR.DAT uses numeric u32 IDs. Parse the string to u32 for lookup.
        let trophy_num: u32 = def.id.parse().unwrap_or(0);
        let (earned, timestamp) = unlock_states
            .get(&trophy_num)
            .cloned()
            .unwrap_or((false, 0));

        let icon_path = trophy_icon_path_in(config_dir, npwr_id, trophy_num);
        let icon_str = if icon_path.is_file() {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        achievements.push(MergedAchievement {
            name: format!("trophy_{}", def.id),
            display_name: def.name.clone(),
            description: def.detail.clone(),
            hidden: def.hidden,
            earned,
            earned_time: timestamp,
            icon_path: icon_str.clone(),
            icon_gray_path: icon_str, // PS3 doesn't have gray icons — reuse same
            global_percent: 0.0,
            trophy_type: def.ttype,
        });
    }

    achievements
}
