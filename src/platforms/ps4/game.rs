use std::path::Path;

use crate::models::{Game, MergedAchievement};
use crate::platforms::ps4::{
    trophy_dir, user_trophy_path, parse_trop_xml, parse_user_trophies,
    read_play_times, parse_playtime, ShadPS4Game,
};

/// Map PS4 trophy type to rarity string
#[allow(dead_code)]
fn trophy_type_rarity(ttype: char) -> &'static str {
    match ttype {
        'P' => "Platinum",
        'G' => "Gold",
        'S' => "Silver",
        'B' => "Bronze",
        _ => "Bronze",
    }
}

/// Load a shadPS4 game as a Game struct with achievements.
pub fn load_shadps4_game(
    shad: &ShadPS4Game,
    db_id: i64,
    title: &str,
    hidden: bool,
    logo_position: &str,
    logo_size: i32,
    sort_title: &str,
    sgdb_id: &str,
    shadps4_version: &str,
    last_played: i64,
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
        kind: "ps4".to_string(),
        platform_id: serial.clone(),
        db_id,
        name: if title.is_empty() {
            shad.title.clone()
        } else {
            title.to_string()
        },
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden,
        lutris_id: serial_to_lutris_id(serial),
        slug: serial.clone(),
        playtime,
        lastplayed: last_played,
        logo_position: logo_position.to_string(),
        logo_size,
        lutris_name: shad.title.clone(),
        manual_unmatch: false,
        sort_title: sort_title.to_string(),
        game_path: shad.game_path.to_string_lossy().into_owned(),
        sgdb_id: sgdb_id.to_string(),
        shadps4_version: shadps4_version.to_string(),
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
    let image_dir = if !sgdb_id.is_empty() {
        Path::new(save_dir).join("data").join("steamgriddb").join(sgdb_id)
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

    crate::parser::populate_image_paths(&image_dir, &mut game);

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
        };

        game.total_count += 1;
        if earned {
            game.earned_count += 1;
        }
        game.achievements.push(ach);
    }

    game
}

/// Generate a stable synthetic lutris_id from a CUSA ID.
/// Uses negative range to avoid collision with real Lutris IDs.
pub fn serial_to_lutris_id(serial: &str) -> i64 {
    let hash = serial.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    -((hash % 1_000_000) as i64 + 2_000_000)
}

/// Check if a lutris_id is a shadPS4 synthetic ID.
pub fn is_shadps4_id(lutris_id: i64) -> bool {
    lutris_id <= -2_000_000
}
