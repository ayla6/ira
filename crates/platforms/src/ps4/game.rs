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
        variant_id: None,
    };

    // Default icon: copy the game's sce_sys/icon0.png to data/ps4/{NPWR}/ and
    // convert to WebP. Only copies if no icon (webp/jpg) already exists in the data dir.
    let ps4_data_dir = Path::new(save_dir).join("data").join("ps4").join(npwr_id);
    if ira_parser::find_image_file(&ps4_data_dir, "icon").is_none() {
        let default_icon = shad.game_path.join("sce_sys").join("icon0.png");
        if default_icon.is_file() {
            let _ = std::fs::create_dir_all(&ps4_data_dir);
            let tmp_png = ps4_data_dir.join("icon.png");
            let _ = std::fs::copy(&default_icon, &tmp_png);
            ira_parser::convert_to_lossless_webp(&tmp_png);
        }
    }

    // Image paths — always use data/ps4/{NPWR_ID}/ to match game_data_dir/entry_data_dir
    let image_dir = ps4_data_dir.clone();

    // Fallback to the emulator's original sce_sys/icon0.png if no icon in data dir.
    if ira_parser::find_image_file(&image_dir, "icon").is_none() {
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
