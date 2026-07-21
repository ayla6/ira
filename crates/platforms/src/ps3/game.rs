use std::path::Path;

use ira_models::{Game, MergedAchievement};
use crate::ps4::parse_trop_xml;

use super::discovery::Rpcs3Game;
use super::paths::{trophy_conf_path, tropusr_path, trophy_icon_path, persistent_settings_path};
use super::tropusr::parse_tropusr;
use super::persistent::{parse_persistent_settings, ms_to_hours};

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
pub fn load_rpcs3_game(
    game: &Rpcs3Game,
    db_id: i64,
    meta: &Rpcs3GameMeta,
    save_dir: &str,
) -> Game {
    let npwr_id = &game.npwr_id;
    let serial = &game.serial;

    // Trophy definitions from TROPCONF.SFM — same XML schema as PS4's TROP.XML,
    // so we reuse parse_trop_xml directly.
    let defs = if npwr_id.is_empty() {
        Vec::new()
    } else {
        parse_trop_xml(&trophy_conf_path(npwr_id))
    };

    // Unlock state from TROPUSR.DAT (binary).
    let unlock_states = if npwr_id.is_empty() {
        Default::default()
    } else {
        parse_tropusr(&tropusr_path(npwr_id)).unwrap_or_default()
    };

    // Playtime + last-played from persistent_settings.dat.
    let persistent = parse_persistent_settings(&persistent_settings_path());
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
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
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
        variant_id: None,
    };

    // Default icon: copy the game's ICON0.PNG to data/ps3/{NPWR}/icon.png.
    let ps3_data_dir = Path::new(save_dir).join("data").join("ps3").join(npwr_id);
    let ps3_icon = ps3_data_dir.join("icon.png");
    if !ps3_icon.is_file() {
        let default_icon = game.game_path.join("ICON0.PNG");
        if default_icon.is_file() {
            let _ = std::fs::create_dir_all(&ps3_data_dir);
            let _ = std::fs::copy(&default_icon, &ps3_icon);
        }
    }

    // Image paths — use SGDB dir if sgdb_id is set, otherwise data/ps3/{NPWR_ID}/.
    let image_dir = if !meta.sgdb_id.is_empty() {
        Path::new(save_dir).join("data").join("steamgriddb").join(&meta.sgdb_id)
    } else {
        ps3_data_dir.clone()
    };

    let icon_png = image_dir.join("icon.png");
    if icon_png.is_file() {
        out.icon_path = icon_png.to_string_lossy().into_owned();
    } else if ps3_icon.is_file() {
        out.icon_path = ps3_icon.to_string_lossy().into_owned();
    } else {
        let default_icon = game.game_path.join("ICON0.PNG");
        if default_icon.is_file() {
            out.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }

    ira_parser::populate_image_paths(&image_dir, &mut out);

    // Build achievements from TROPCONF.SFM definitions + TROPUSR.DAT unlock state.
    for def in &defs {
        // TROPCONF.SFM trophy IDs are zero-padded strings ("000", "001", ...).
        // TROPUSR.DAT uses numeric u32 IDs. Parse the string to u32 for lookup.
        let trophy_num: u32 = def.id.parse().unwrap_or(0);
        let (earned, timestamp) = unlock_states
            .get(&trophy_num)
            .cloned()
            .unwrap_or((false, 0));

        let icon_path = trophy_icon_path(npwr_id, trophy_num);
        let icon_str = if icon_path.is_file() {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        out.achievements.push(MergedAchievement {
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

        out.total_count += 1;
        if earned {
            out.earned_count += 1;
        }
    }

    out
}
