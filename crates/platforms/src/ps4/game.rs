use std::path::Path;

use ira_models::{Game, MergedAchievement};
use crate::ps4::{
    trophy_dir, user_trophy_path, parse_trop_xml, parse_user_trophies,
    read_play_times, parse_playtime, ShadPS4Game,
};

pub struct ShadPS4GameMeta {
    pub title: String,
    pub hidden: bool,
    pub logo_position: String,
    pub logo_size: i32,
    pub sort_title: String,
    pub sgdb_id: String,
    pub shadps4_version: String,
    pub last_played: i64,
}

/// Load a shadPS4 game as a Game struct with achievements.
pub fn load_shadps4_game(
    shad: &ShadPS4Game,
    db_id: i64,
    meta: &ShadPS4GameMeta,
    save_dir: &str,
) -> Game {
    let npwr_id = &shad.npwr_id;
    let serial = &shad.serial;

    // Trophy definitions
    let trop_xml = trophy_dir(npwr_id).join("Xml").join("TROP.XML");
    let defs = parse_trop_xml(&trop_xml);

    // User unlock state
    let user_xml = user_trophy_path(npwr_id);
    let unlock_states = parse_user_trophies(&user_xml);

    // Trophy icons dir
    let icons_dir = trophy_dir(npwr_id).join("Icons");

    // Playtime
    let play_times = read_play_times();
    let playtime_str = play_times.get(serial).cloned().unwrap_or_default();
    let playtime = parse_playtime(&playtime_str);

    // Build game
    let mut game = Game {
        app_id: npwr_id.clone(),
        kind: ira_models::GameKind::Ps4,
        trophy_source: ira_models::TrophySource::Empty,
        platform_id: serial.clone(),
        db_id,
        name: if meta.title.is_empty() {
            shad.title.clone()
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
        last_played: meta.last_played,
        logo_position: meta.logo_position.clone(),
        logo_size: meta.logo_size,
        manual_unmatch: false,
        sort_title: meta.sort_title.clone(),
        game_path: shad.game_path.to_string_lossy().into_owned(),
        sgdb_id: meta.sgdb_id.clone(),
        shadps4_version: meta.shadps4_version.clone(),
        release_date: String::new(),
        release_timestamp: 0,
        metacritic_score: -1,
        steam_review_score: -1,
        steam_review_count: 0,
        ra_core: String::new(),
        emulator_override: String::new(),
        rom_path: String::new(),
    };

    // PS4 default icon: always copy from the game's sce_sys/icon0.png to data/ps4/{NPWR}/icon.png
    let ps4_data_dir = Path::new(save_dir).join("data").join("ps4").join(npwr_id);
    let ps4_icon = ps4_data_dir.join("icon.png");
    if !ps4_icon.is_file() {
        let default_icon = shad.game_path.join("sce_sys").join("icon0.png");
        if default_icon.is_file() {
            let _ = std::fs::create_dir_all(&ps4_data_dir);
            let _ = std::fs::copy(&default_icon, &ps4_icon);
        }
    }

    // Image paths — use SGDB dir if sgdb_id is set, otherwise data/ps4/{NPWR_ID}/
    let image_dir = if !meta.sgdb_id.is_empty() {
        Path::new(save_dir).join("data").join("steamgriddb").join(&meta.sgdb_id)
    } else {
        ps4_data_dir.clone()
    };

    let icon_png = image_dir.join("icon.png");
    if icon_png.is_file() {
        game.icon_path = icon_png.to_string_lossy().into_owned();
    } else if ps4_icon.is_file() {
        game.icon_path = ps4_icon.to_string_lossy().into_owned();
    } else {
        // Fallback: game's sce_sys/icon0.png
        let default_icon = shad.game_path.join("sce_sys").join("icon0.png");
        if default_icon.is_file() {
            game.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }

    ira_parser::populate_image_paths(&image_dir, &mut game);

    // Build achievements
    for def in &defs {
        let (earned, timestamp) = unlock_states
            .get(&def.id)
            .cloned()
            .unwrap_or((false, 0));

        // Icon path: TROP000.PNG, TROP001.PNG, etc.
        let icon_name = format!("TROP{:03}.PNG", def.id.parse::<u32>().unwrap_or(0));
        let icon_path = icons_dir.join(&icon_name);
        let icon_str = if icon_path.is_file() {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        let ach = MergedAchievement {
            name: format!("trophy_{}", def.id),
            display_name: def.name.clone(),
            description: def.detail.clone(),
            hidden: def.hidden,
            earned,
            earned_time: timestamp,
            icon_path: icon_str.clone(),
            icon_gray_path: icon_str, // PS4 doesn't have gray icons — reuse same
            global_percent: 0.0,
            trophy_type: def.ttype,
        };

        game.total_count += 1;
        if earned {
            game.earned_count += 1;
        }
        game.achievements.push(ach);
    }

    game
}
