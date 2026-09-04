use super::enrichment::enrich_game_async;
use super::helpers::confirm_dialog;
use super::state::SharedState;
use crate::AppMessage;
use crate::AppSender;
use crate::Game;
use crate::MergedAchievement;
use ira_api::SteamDataClient;
use ira_parser::set_achievement_earned;
use std::sync::Arc;

pub fn match_game_to_steam(
    state: &SharedState,
    db_id: i64,
    steam_app_id: String,
    game_name: String,
) {
    let (steam, sender, db, save_dir, ra_username, ra_web_api_key, cfg) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.ra_username.clone(),
            s.cfg.ra_web_api_key.clone(),
            s.cfg.clone(),
        )
    };
    std::thread::spawn(move || {
        if let Err(e) = ira_db::update_game_ids(
            &db,
            db_id,
            &steam_app_id,
            &steam_app_id,
            ira_models::TrophySource::Gse,
            &steam_app_id,
        ) {
            eprintln!("match_game_to_steam: update_game_ids failed: {}", e);
            return;
        }
        if let Err(e) = ira_db::set_manual_unmatch(&db, db_id, false) {
            eprintln!("match_game_to_steam: set_manual_unmatch failed: {}", e);
        }
        if let Err(e) = steam.generate_steam_settings(&steam_app_id) {
            eprintln!("match_game_to_steam: generate_steam_settings failed: {}", e);
        }
        match ira_db::find_by_db_id(&db, db_id) {
            Ok(Some(entry)) => match crate::game_loader::load_game(&entry, &save_dir) {
                Ok(mut game) => {
                    if game.name.is_empty() || game.name.starts_with("App ID:") {
                        game.set_name(&game_name);
                    }
                    let name = game.name.clone();
                    let _ = sender.send(AppMessage::NewGame(game));
                    enrich_game_async(crate::ui::enrichment::EnrichGameParams {
                        app_id: steam_app_id.clone(),
                        trophy_source: ira_models::TrophySource::Gse,
                        platform_id: steam_app_id.clone(),
                        db_id: entry.id,
                        title: name,
                        steam,
                        sender,
                        save_dir,
                        db,
                        ra_username,
                        ra_web_api_key,
                        cfg,
                        game: None,
                    });
                }
                Err(e) => eprintln!("match_game_to_steam: load_game failed: {}", e),
            },
            Ok(None) => eprintln!(
                "match_game_to_steam: find_by_db_id returned None for db_id={}",
                db_id
            ),
            Err(e) => eprintln!("match_game_to_steam: find_by_db_id error: {}", e),
        }
    });
}

pub fn match_game_to_sgdb(state: &SharedState, db_id: i64, sgdb_id: String) {
    let (steam, sender, db, save_dir, cfg) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.db.clone(),
            s.save_dir.clone(),
            s.cfg.clone(),
        )
    };
    std::thread::spawn(move || {
        if let Err(e) = ira_db::set_sgdb_id(&db, db_id, &sgdb_id) {
            eprintln!("match_game_to_sgdb: set_sgdb_id failed: {}", e);
            return;
        }
        if let Err(e) = ira_db::set_manual_unmatch(&db, db_id, false) {
            eprintln!("match_game_to_sgdb: set_manual_unmatch failed: {}", e);
        }
        let entry = ira_db::find_by_db_id(&db, db_id).ok().flatten();
        let (dir, game) = match &entry {
            Some(e) => (
                ira_parser::entry_data_dir(&save_dir, e),
                Some(sgdb_game_from_entry(e, &sgdb_id)),
            ),
            None => (ira_parser::sgdb_data_dir(&save_dir, &sgdb_id), None),
        };
        let switch_exe = cfg.console("switch").executable.clone();
        let (icon, hero, grid, logo, header, square) = match &game {
            Some(g) => super::fetch_images::ensure_game_assets(&steam, &dir, &cfg, g, &switch_exe),
            None => steam.ensure_sgdb_assets_in_dir(&dir, ira_api::types::SgdbId::Game(&sgdb_id), &[]),
        };
        let Some(mut game) = game else {
            return;
        };
        game.icon_path = icon;
        game.hero_image_path = hero;
        game.grid_path = grid;
        game.logo_path = logo;
        game.header_path = header;
        game.square_path = square;
        let _ = sender.send(AppMessage::NewGame(game));
    });
}

/// The display `Game` for a freshly matched SGDB entry, before any asset
/// paths are filled in. `game_path` carries the ROM path so the native
/// icon import can find console ROMs.
fn sgdb_game_from_entry(entry: &ira_models::GameEntry, sgdb_id: &str) -> Game {
    Game {
        app_id: String::new(),
        kind: entry.kind,
        trophy_source: entry.trophy_source,
        platform_id: entry.platform_id.clone(),
        db_id: entry.id,
        name: entry.title.clone(),
        name_lower: entry.title.to_lowercase(),
        icon_path: String::new(),
        hero_image_path: String::new(),
        grid_path: String::new(),
        header_path: String::new(),
        logo_path: String::new(),
        square_path: String::new(),
        achievements: Vec::new(),
        earned_count: 0,
        total_count: 0,
        hidden: entry.hidden,
        slug: String::new(),
        playtime: 0.0,
        last_played: entry.last_played,
        logo_position: entry.logo_position.clone(),
        logo_size: entry.logo_size,
        manual_unmatch: entry.manual_unmatch,
        sort_title: entry.sort_title.clone(),
        game_path: entry.rom_path.clone(),
        sgdb_id: sgdb_id.to_string(),
        shadps4_version: entry.shadps4_version.clone(),
        release_date: entry.release_date.clone(),
        release_timestamp: entry.release_timestamp,
        metacritic_score: entry.metacritic_score,
        steam_review_score: entry.steam_review_score,
        steam_review_count: entry.steam_review_count,
        ra_core: entry.ra_core.clone(),
        emulator_override: entry.emulator_override.clone(),
        rom_path: entry.rom_path.clone(),
        game_folder: entry.game_folder.clone(),
        variant_id: None,
    }
}

/// Persist accepting an SGDB match on the DB side: store the SGDB id and,
/// when `clear_manual_unmatch` is set, drop any prior manual-unmatch flag.
/// Failures are logged, never propagated — callers keep going either way.
pub(crate) fn persist_sgdb_match(
    db: &ira_db::DbConn,
    db_id: i64,
    sgdb_id: &str,
    clear_manual_unmatch: bool,
) {
    if let Err(e) = ira_db::set_sgdb_id(db, db_id, sgdb_id) {
        eprintln!("Failed to set SGDB ID: {}", e);
    }
    if clear_manual_unmatch {
        if let Err(e) = ira_db::set_manual_unmatch(db, db_id, false) {
            eprintln!("Failed to clear manual unmatch: {}", e);
        }
    }
}

/// Worker-thread tail shared by the SGDB-match flows: resolve the asset
/// directory (the matched game's own kind-aware directory when known, else
/// the bare SGDB-id pool), download the asset set into it (console games
/// keep their square native), and announce the resulting paths to the UI.
/// Purely blocking work — call from a spawned thread; each caller keeps its
/// own tracing span and any stagger sleep.
pub(crate) fn fetch_and_report_sgdb_assets(
    steam: &Arc<SteamDataClient>,
    sender: &AppSender,
    save_dir: &str,
    cfg: &ira_config::Config,
    game_for_dir: Option<&Game>,
    db_id: i64,
    sgdb_id: String,
) {
    let dir = match game_for_dir {
        Some(g) => ira_parser::game_data_dir(save_dir, g),
        None => ira_parser::sgdb_data_dir(save_dir, &sgdb_id),
    };
    let (icon, hero, grid, logo, header, square) = match game_for_dir {
        Some(g) => {
            let switch_exe = cfg.console("switch").executable.clone();
            super::fetch_images::ensure_game_assets(steam, &dir, cfg, g, &switch_exe)
        }
        None => steam.ensure_sgdb_assets_in_dir(&dir, ira_api::types::SgdbId::Game(&sgdb_id), &[]),
    };
    let _ = sender.send(AppMessage::SgdbAssetsDownloaded {
        db_id,
        sgdb_id,
        icon,
        hero,
        grid,
        logo,
        header,
        square,
    });
}

pub fn confirm_mark_unlocked(
    state: &SharedState,
    trophy_source: ira_models::TrophySource,
    app_id: &str,
    platform_id: &str,
    ach: &MergedAchievement,
    reload: impl Fn() + 'static,
) {
    let window = state.borrow().window.clone();
    let ach_name = ach.name.clone();
    let app_id = app_id.to_string();
    let platform_id = platform_id.to_string();
    let save_dir = state.borrow().save_dir.clone();
    confirm_dialog(
        &window,
        &crate::tr!("Mark as already unlocked?"),
        &crate::tr!(
            "This will mark \u{201c}{}\u{201d} as earned without a real unlock time. Use this only if you already unlocked it previously (e.g. before using this tool)."
        )
        .replacen("{}", &ach.display_name, 1),
        &crate::tr!("Mark as unlocked"),
        adw::ResponseAppearance::Destructive,
        move || {
            if let Err(e) = set_achievement_earned(
                &save_dir,
                trophy_source,
                &app_id,
                &platform_id,
                &ach_name,
                true,
            ) {
                eprintln!("Failed to mark achievement as unlocked: {}", e);
                return;
            }
            reload();
        },
    );
}
