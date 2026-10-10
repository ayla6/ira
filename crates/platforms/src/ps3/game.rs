use std::collections::HashMap;
use std::path::Path;

use crate::sony::parse_trop_xml;
use ira_models::{Game, MergedAchievement};

use super::discovery::Rpcs3Game;
use super::paths::{
    persistent_settings_path_in, rpcs3_config_dir, trophy_conf_path_in, trophy_icon_path_in,
    tropusr_path_in,
};
use super::persistent::{ms_to_hours, parse_persistent_settings};
use super::tropusr::parse_tropusr;

/// Metadata for an RPCS3 game from the DB entry (no version selector:
/// RPCS3 ships as a single binary).
pub struct Rpcs3GameMeta {
    pub title: String,
    pub hidden: bool,
    pub logo_position: ira_models::LogoPosition,
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
        native_id: serial.clone(),
        kind: ira_models::GameKind::Ps3,
        platform_id: ira_models::Platform::parse(ira_models::GameKind::Ps3.as_str()),
        db_id,
        name: if meta.title.is_empty() {
            game.title.clone()
        } else {
            meta.title.clone()
        },
        hidden: meta.hidden,
        slug: serial.clone(),
        playtime,
        last_played,
        logo_position: meta.logo_position,
        logo_size: meta.logo_size,
        sort_title: meta.sort_title.clone(),
        game_path: game.game_path.to_string_lossy().into_owned(),
        sgdb_id: meta.sgdb_id.clone(),
        ..Default::default()
    };
    out.name_lower = out.name.to_lowercase();

    let ps3_data_dir = Path::new(save_dir).join("data").join("ps3").join(npwr_id);
    ensure_default_icon(&game.game_path, &ps3_data_dir);
    apply_icon_paths(&mut out, &game.game_path, &ps3_data_dir);

    out.achievements = load_ps3_trophies_in(npwr_id, &game.config_dir);
    out.total_count = out.achievements.len();
    out.earned_count = out.achievements.iter().filter(|a| a.earned).count();

    out
}

/// Copy ICON0.PNG into the data dir (converting to WebP) when no icon exists there yet.
fn ensure_default_icon(game_path: &Path, data_dir: &Path) {
    if ira_parser::find_image_file(data_dir, ira_models::AssetType::Icon.file_base()).is_some() {
        return;
    }
    let default_icon = game_path.join("ICON0.PNG");
    if !default_icon.is_file() {
        return;
    }
    if std::fs::create_dir_all(data_dir).is_err() {
        return;
    }
    let tmp_png = data_dir.join("icon.png");
    if std::fs::copy(&default_icon, &tmp_png).is_err() {
        return;
    }
    ira_parser::convert_to_lossless_webp(&tmp_png);
}

fn apply_icon_paths(out: &mut Game, game_path: &Path, image_dir: &Path) {
    // Fallback to the emulator's original ICON0.PNG if no icon in data dir.
    if ira_parser::find_image_file(image_dir, ira_models::AssetType::Icon.file_base()).is_none() {
        let default_icon = game_path.join("ICON0.PNG");
        if default_icon.is_file() {
            out.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }
    ira_parser::populate_image_paths(image_dir, out);
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
    let unlock_states = parse_tropusr(&tropusr_path_in(config_dir, npwr_id)).unwrap_or_else(|e| {
        eprintln!("Failed to parse TROPUSR.DAT for {npwr_id}: {e}");
        HashMap::new()
    });

    defs.iter()
        .map(|def| {
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

            MergedAchievement {
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
            }
        })
        .collect()
}
