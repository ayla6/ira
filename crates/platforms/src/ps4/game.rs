use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ps4::{parse_playtime, read_play_times_from_path, ShadPS4Game};
use crate::sony::{parse_trop_xml, parse_user_trophies, TrophyDef};
use ira_models::{Game, MergedAchievement};

pub struct ShadPS4GameMeta {
    pub title: String,
    pub hidden: bool,
    pub logo_position: ira_models::LogoPosition,
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

    let defs = parse_trop_xml(&trophy_defs_path(&shad.user_dir, npwr_id));
    let unlock_states = parse_user_trophies(&user_trophy_xml_path(&shad.user_dir, npwr_id));
    let icons_dir = shad.user_dir.join("trophy").join(npwr_id).join("Icons");

    let play_times = read_play_times_from_path(&shad.user_dir.join("play_time.txt"));
    let playtime = play_times
        .get(serial)
        .map(|s| parse_playtime(s))
        .unwrap_or(0.0);

    let mut game = base_game(shad, db_id, meta, playtime);
    let ps4_data_dir = Path::new(save_dir).join("data").join("ps4").join(npwr_id);
    ensure_default_icon(&shad.game_path, &ps4_data_dir);
    apply_icon_paths(&mut game, &shad.game_path, &ps4_data_dir);
    game.achievements = build_achievements(&defs, &unlock_states, &icons_dir);
    game.total_count = game.achievements.len();
    game.earned_count = game.achievements.iter().filter(|a| a.earned).count();

    game
}

fn trophy_defs_path(user_dir: &Path, npwr_id: &str) -> PathBuf {
    user_dir
        .join("trophy")
        .join(npwr_id)
        .join("Xml")
        .join("TROP.XML")
}

fn user_trophy_xml_path(user_dir: &Path, npwr_id: &str) -> PathBuf {
    user_dir
        .join("home")
        .join("1000")
        .join("trophy")
        .join(format!("{npwr_id}.xml"))
}

fn base_game(shad: &ShadPS4Game, db_id: i64, meta: &ShadPS4GameMeta, playtime: f64) -> Game {
    let mut game = Game {
        app_id: shad.npwr_id.clone(),
        native_id: shad.serial.clone(),
        kind: ira_models::GameKind::Ps4,
        platform_id: ira_models::Platform::parse(ira_models::GameKind::Ps4.as_str()),
        db_id,
        name: if meta.title.is_empty() {
            shad.title.clone()
        } else {
            meta.title.clone()
        },
        hidden: meta.hidden,
        slug: shad.serial.clone(),
        playtime,
        last_played: meta.last_played,
        logo_position: meta.logo_position,
        logo_size: meta.logo_size,
        sort_title: meta.sort_title.clone(),
        game_path: shad.game_path.to_string_lossy().into_owned(),
        sgdb_id: meta.sgdb_id.clone(),
        shadps4_version: meta.shadps4_version.clone(),
        ..Default::default()
    };
    game.name_lower = game.name.to_lowercase();
    game
}

/// Copy sce_sys/icon0.png into the data dir (converting to WebP) when no
/// icon exists there yet.
fn ensure_default_icon(game_path: &Path, data_dir: &Path) {
    if ira_parser::find_image_file(data_dir, ira_models::AssetType::Icon.file_base()).is_some() {
        return;
    }
    let default_icon = game_path.join("sce_sys").join("icon0.png");
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

fn apply_icon_paths(game: &mut Game, game_path: &Path, image_dir: &Path) {
    // Fallback to the emulator's original sce_sys/icon0.png if no icon in data dir.
    if ira_parser::find_image_file(image_dir, ira_models::AssetType::Icon.file_base()).is_none() {
        let default_icon = game_path.join("sce_sys").join("icon0.png");
        if default_icon.is_file() {
            game.icon_path = default_icon.to_string_lossy().into_owned();
        }
    }
    ira_parser::populate_image_paths(image_dir, game);
}

/// Load PS4 trophy achievements from TROP.XML (definitions) + the user
/// trophy XML (unlock state), mirroring `load_ps3_trophies`. The detail
/// view only has the DB row — not the discovery-time user dir — so this
/// resolves the standard (non-flatpak) shadPS4 user dir itself.
/// Returns an empty vector if the NPWR ID is empty or shadPS4 has no
/// trophy data for it (e.g. never launched there).
pub fn load_ps4_trophies(npwr_id: &str) -> Vec<MergedAchievement> {
    load_ps4_trophies_in(npwr_id, &super::paths::shadps4_user_dir())
}

pub fn load_ps4_trophies_in(npwr_id: &str, user_dir: &Path) -> Vec<MergedAchievement> {
    if npwr_id.is_empty() {
        return Vec::new();
    }
    let defs_path = trophy_defs_path(user_dir, npwr_id);
    if !defs_path.is_file() {
        return Vec::new();
    }
    let defs = parse_trop_xml(&defs_path);
    let unlock_states = parse_user_trophies(&user_trophy_xml_path(user_dir, npwr_id));
    build_achievements(
        &defs,
        &unlock_states,
        &user_dir.join("trophy").join(npwr_id).join("Icons"),
    )
}

fn build_achievements(
    defs: &[TrophyDef],
    unlock_states: &HashMap<String, (bool, i64)>,
    icons_dir: &Path,
) -> Vec<MergedAchievement> {
    defs.iter()
        .map(|def| {
            let (earned, timestamp) = unlock_states.get(&def.id).cloned().unwrap_or((false, 0));
            // Icon path: TROP000.PNG, TROP001.PNG, etc.
            let icon_name = format!("TROP{:03}.PNG", def.id.parse::<u32>().unwrap_or(0));
            let icon_path = icons_dir.join(&icon_name);
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
                icon_gray_path: icon_str, // PS4 doesn't have gray icons — reuse same
                global_percent: 0.0,
                trophy_type: def.ttype,
            }
        })
        .collect()
}
