use crate::AppMessage;
use crate::AppSender;
use crate::Game;
use crate::GameEntry;
use ira_api::SteamDataClient;
use ira_api::types::SteamCmdInfo;
use ira_db::DbConn;

use crate::game_loader::load_game;
use std::sync::Arc;
use std::sync::Mutex;

static RA_ENRICH_LOCK: Mutex<()> = Mutex::new(());

/// Phase reporter for [`EnrichGameParams::progress`]: how many phases are
/// done, how many the job has in total, and what it is doing now. Called
/// on the enriching thread as each phase begins; labels arrive already
/// translated.
pub type EnrichProgress = std::sync::Arc<dyn Fn(usize, usize, &str) + Send + Sync>;

pub struct EnrichGameParams {
    pub app_id: String,
    pub trophy_source: ira_models::TrophySource,
    pub platform_id: String,
    pub db_id: i64,
    pub title: String,
    pub steam: Arc<SteamDataClient>,
    pub sender: AppSender,
    pub save_dir: String,
    pub db: DbConn,
    pub ra_username: String,
    pub ra_web_api_key: String,
    pub cfg: ira_config::Config,
    /// If provided, use this game instead of calling load_game (skips achievement loading).
    /// Pass Some for background enrichment, None for on-demand loading.
    pub game: Option<Game>,
    /// Optional sidebar-strip reporter for jobs that outlive the dialog
    /// that started them, e.g. the auto-add wizard's asset download.
    pub progress: Option<EnrichProgress>,
}

pub fn enrich_game_async(params: EnrichGameParams) {
    std::thread::spawn(move || {
        enrich_game_blocking(params);
    });
}

/// Drives a strip's phase labels for one enrichment: achievements,
/// images, store metadata — reported as each begins.
struct Phases(Option<EnrichProgress>, usize);

impl Phases {
    fn enter(&self, done: usize, label: &str) {
        if let Some(report) = &self.0 {
            report(done, self.1, label);
        }
    }
}

pub fn enrich_game_blocking(params: EnrichGameParams) {
    let EnrichGameParams {
        app_id,
        trophy_source,
        platform_id,
        db_id,
        title,
        steam,
        sender,
        save_dir,
        db,
        ra_username,
        ra_web_api_key,
        cfg,
        game,
        progress,
    } = params;
    let phases = Phases(progress, 3);
    let _s = tracing::info_span!("enrich_game", app_id = %app_id, db_id = db_id).entered();

    let mut game = if trophy_source == ira_models::TrophySource::Ra {
        // RA is the first link of the default icon chain; games it leaves
        // icon-less fall through to native and SteamGridDB below.
        let _guard = RA_ENRICH_LOCK.lock().unwrap();
        match enrich_ra(EnrichRaParams {
            app_id: &app_id,
            trophy_source: trophy_source.as_str(),
            platform_id: &platform_id,
            db_id,
            title: &title,
            save_dir: &save_dir,
            ra_username: &ra_username,
            ra_web_api_key: &ra_web_api_key,
            db: &db,
        }) {
            Some(g) => g,
            None => return,
        }
    } else {
        let entry = ira_db::find_by_db_id(&db, db_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                let mut e = GameEntry::for_reload(
                    db_id,
                    ira_models::GameKind::Other,
                    trophy_source,
                    &app_id,
                    "",
                    &platform_id,
                );
                e.title = title.clone();
                e
            });

        if trophy_source.has_steam_enrichment() {
            let meta_path =
                ira_parser::achievements_dir(&save_dir, &app_id).join("achievements.json");
            if !meta_path.exists() {
                phases.enter(0, &crate::tr!("Fetching achievements…"));
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!("Could not generate achievements for {}: {}", app_id, e);
                    if let Some(parent) = meta_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&meta_path, "[]");
                }
            }
        }

        let game_provided = game.is_some();
        let mut game = match game {
            Some(g) => g,
            None => match load_game(&entry, &save_dir) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("Failed reloading {}: {}", app_id, e);
                    return;
                }
            },
        };

        if !game_provided && trophy_source.has_steam_enrichment() {
            let has_missing_icons = game.achievements.iter().any(|a| a.icon_path.is_empty());
            if has_missing_icons {
                let _s = tracing::info_span!("enrich_redownload_icons", app_id = %app_id).entered();
                if let Err(e) = steam.generate_steam_settings(&app_id) {
                    eprintln!(
                        "Could not re-download achievement icons for {}: {}",
                        app_id, e
                    );
                } else if let Ok(reloaded) = load_game(&entry, &save_dir) {
                    game.achievements = reloaded.achievements;
                }
            }
        }
        game
    };

    if trophy_source.has_steam_enrichment() {
        phases.enter(1, &crate::tr!("Downloading images…"));
        enrich_steam_assets(&mut game, &steam, &save_dir, &app_id);

        if let Err(e) = ira_db::store_game_metadata(
            &db,
            game.db_id,
            &game.release_date,
            game.release_timestamp,
            game.metacritic_score,
            game.steam_review_score,
            game.steam_review_count,
        ) {
            eprintln!("Failed to store game metadata: {}", e);
        }
    }

    ensure_default_icon(&mut game, &steam, &cfg, &save_dir);

    // Art still missing after Steam and the native icon chain: SteamGridDB
    // fills the gaps — through the game's SGDB match when there is one,
    // otherwise through SGDB's steam endpoints by the app id — so a
    // freshly added game never sits imageless. Only missing files
    // download; the manual "fetch missing images" pass reuses them.
    if steam.has_sgdb_key()
        && (game.icon_path.is_empty()
            || game.hero_image_path.is_empty()
            || game.grid_path.is_empty()
            || game.header_path.is_empty()
            || game.logo_path.is_empty())
    {
        let switch_exe = cfg.console("switch").executable.clone();
        let dir = ira_parser::game_data_dir(&save_dir, &game);
        let (icon, hero, grid, logo, header, square) =
            super::fetch_images::ensure_game_assets(&steam, &dir, &cfg, &game, &switch_exe);
        if game.icon_path.is_empty() {
            game.icon_path = icon;
        }
        if game.hero_image_path.is_empty() {
            game.hero_image_path = hero;
        }
        if game.grid_path.is_empty() {
            game.grid_path = grid;
        }
        if game.logo_path.is_empty() {
            game.logo_path = logo;
        }
        if game.header_path.is_empty() {
            game.header_path = header;
        }
        if game.square_path.is_empty() {
            game.square_path = square;
        }
    }

    // PC games: even with no ScreenScraper match, the store synopsis and
    // the company cache's ids can fill the metadata record.
    if game.kind.is_pc() {
        if let Ok(app_id) = game.platform_id.parse::<u32>() {
            phases.enter(2, &crate::tr!("Fetching store metadata…"));
            garnish_pc_ss_metadata(&db, &steam, game.db_id, app_id);
        }
    }

    let _ = sender.send(AppMessage::EnrichedGame(game));
}

/// The Steam-driven enrichment block, extracted from `enrich_game_blocking`.
fn enrich_steam_assets(
    game: &mut Game,
    steam: &Arc<SteamDataClient>,
    save_dir: &str,
    app_id: &str,
) {
    let _s = tracing::info_span!("enrich_steam_assets", app_id = %app_id).entered();
    if game.name.starts_with("App ID:") || !ira_parser::data_dir(save_dir, app_id).join("appdetails.json").exists() {
        if let Some(details) = steam.fetch_app_details(app_id) {
            if !details.name.is_empty() && game.name.starts_with("App ID:") {
                game.set_name(&details.name);
            }
        }
    }

    let has_local_icon = !game.icon_path.is_empty();
    if game.icon_path.is_empty() || game.hero_image_path.is_empty() {
        let (icon_path, hero_path) = steam.ensure_assets(app_id, has_local_icon);
        if game.icon_path.is_empty() && !icon_path.is_empty() {
            game.icon_path = icon_path;
        }
        if game.hero_image_path.is_empty() && !hero_path.is_empty() {
            game.hero_image_path = hero_path;
        }
    }

    if game.icon_path.is_empty() {
        if let Some(png) = fetch_steam_game_icon(app_id, save_dir, steam) {
            game.icon_path = png;
        }
    }

    if game.grid_path.is_empty() || game.header_path.is_empty() || game.logo_path.is_empty() {
        let (grid_path, header_path, logo_path) = steam.ensure_grids(app_id);
        if game.grid_path.is_empty() && !grid_path.is_empty() {
            game.grid_path = grid_path;
        }
        if game.header_path.is_empty() && !header_path.is_empty() {
            game.header_path = header_path;
        }
        if game.logo_path.is_empty() && !logo_path.is_empty() {
            game.logo_path = logo_path;
        }
    }

    if !game.achievements.is_empty() && game.achievements.iter().any(|a| a.global_percent == 0.0)
    {
        if let Some(pcts) = steam.fetch_global_achievements(app_id) {
            for a in &mut game.achievements {
                if let Some(&pct) = pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }
    }

    if game.release_date.is_empty() || game.metacritic_score < 0 || game.steam_review_score < 0 {
        if let Some(cmd) = steam.fetch_steamcmd_info(app_id) {
            if game.release_timestamp == 0 && cmd.release_timestamp > 0 {
                game.release_timestamp = cmd.release_timestamp;
            }
            if game.metacritic_score < 0 && cmd.metacritic_score >= 0 {
                game.metacritic_score = cmd.metacritic_score;
            }
            if game.steam_review_score < 0 && cmd.review_score >= 0 {
                game.steam_review_score = cmd.review_score;
                game.steam_review_count = cmd.review_percentage;
            }
            if game.name.starts_with("App ID:") && !cmd.name.is_empty() {
                game.set_name(&cmd.name);
            }
        }
    }

    if game.steam_review_score < 0 {
        if let Some(review) = steam.fetch_steam_reviews(app_id) {
            game.steam_review_score = review.review_score as i64;
            game.steam_review_count = review.total_reviews as i64;
        }
    }
}

/// Default icon chain for games that end enrichment without any icon:
/// RetroAchievements already ran for matched games, so fall back to the
/// native ROM/emulator icon and finally a SteamGridDB icon picked by name.
fn ensure_default_icon(
    game: &mut Game,
    steam: &Arc<SteamDataClient>,
    cfg: &ira_config::Config,
    save_dir: &str,
) {
    if !game.icon_path.is_empty() {
        return;
    }
    let (azahar_exe, cemu_exe, switch_exe) = (
        cfg.azahar_executable.clone(),
        cfg.cemu_executable.clone(),
        cfg.console("switch").executable.clone(),
    );
    if let Some(bytes) = super::image_manager_helpers::native_icon_bytes(
        game, cfg, &azahar_exe, &cemu_exe, &switch_exe,
    ) {
        if super::image_manager_helpers::write_native_icon_to_disk(
            save_dir,
            game,
            &bytes,
            ira_models::AssetType::Icon,
        ) {
            if let Some(path) =
                ira_parser::find_image_file(&ira_parser::game_data_dir(save_dir, game), ira_models::AssetType::Icon.file_base())
            {
                game.icon_path = path.to_string_lossy().into_owned();
            }
        }
        return;
    }

    let sgdb_matchable = game.sgdb_id.is_empty()
        && !game.manual_unmatch
        && !game.name.is_empty()
        && (game.app_id.is_empty()
            || game.kind == ira_models::GameKind::Retro
            || game.kind.is_console_emulator());
    if !sgdb_matchable || !steam.has_sgdb_key() {
        return;
    }
    let results = steam.search_sgdb(&game.name);
    let Some((sgdb_id, _)) = super::helpers::matching_sgdb_result(&results, &game.name) else {
        return;
    };
    let dir = ira_parser::game_data_dir(save_dir, game);
    let icon = steam.force_download_sgdb(&dir, ira_api::types::SgdbId::Game(&sgdb_id), ira_models::AssetType::Icon);
    if !icon.is_empty() {
        game.icon_path = icon;
    }
}

/// Fetch the game icon from Steam's local clienticon cache or CDN.
/// 1. Look up clienticon hash from cached steamcmd.net data, fall back to appinfo.vdf
/// 2. Try local steam/games/<hash>.ico
/// 3. Fall back to Steam CDN download
/// 4. Convert ICO to WebP and save to the game's data directory
fn fetch_steam_game_icon(
    app_id: &str,
    save_dir: &str,
    steam: &std::sync::Arc<ira_api::SteamDataClient>,
) -> Option<String> {
    let _s = tracing::info_span!("fetch_steam_game_icon", app_id = %app_id).entered();
    let clienticon = steam
        .clienticon_hash(app_id)
        .or_else(|| {
            let app_id_num: u32 = app_id.parse().ok()?;
            ira_platforms::steam::get_clienticon(app_id_num)
        })?;
    if clienticon.is_empty() {
        return None;
    }

    let dest_webp = ira_parser::data_dir(save_dir, app_id).join("icon.webp");
    if dest_webp.is_file() {
        return Some(dest_webp.to_string_lossy().into_owned());
    }

    let _ = std::fs::create_dir_all(dest_webp.parent()?);

    let ico_in_games = ira_platforms::steam::steam_install_dir().map(|d| {
        d.join("steam")
            .join("games")
            .join(format!("{}.ico", clienticon))
    });

    let ico_bytes = if let Some(ref path) = ico_in_games {
        if path.is_file() {
            std::fs::read(path).ok()
        } else {
            None
        }
    } else {
        None
    };

    let ico_bytes = match ico_bytes {
        Some(b) => b,
        None => {
            let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", app_id, clienticon);
            match steam.download_bytes(&url) {
                Ok(b) => b,
                Err(_) => return None,
            }
        }
    };

    let webp = ira_parser::convert_bytes_to_lossless_webp(&ico_bytes)?;
    std::fs::write(&dest_webp, &webp).ok()?;
    Some(dest_webp.to_string_lossy().into_owned())
}

struct EnrichRaParams<'a> {
    app_id: &'a str,
    trophy_source: &'a str,
    platform_id: &'a str,
    db_id: i64,
    title: &'a str,
    save_dir: &'a str,
    ra_username: &'a str,
    ra_web_api_key: &'a str,
    db: &'a DbConn,
}

/// Loads the game and pulls its RA achievements and icon. Returns `None`
/// (sending nothing) when the game cannot be loaded or RA is not
/// configured; the caller owns announcing the result.
fn enrich_ra(params: EnrichRaParams) -> Option<Game> {
    let EnrichRaParams {
        app_id,
        trophy_source,
        platform_id,
        db_id,
        title,
        save_dir,
        ra_username,
        ra_web_api_key,
        db,
    } = params;
    let _s = tracing::info_span!("enrich_ra", app_id = %app_id, db_id = db_id).entered();
    if ra_username.is_empty() || ra_web_api_key.is_empty() {
        eprintln!(
            "RA: skipping enrichment for {} — username_len={} web_api_key_len={}",
            app_id,
            ra_username.len(),
            ra_web_api_key.len()
        );
        return None;
    }

    let entry = ira_db::find_by_db_id(db, db_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            GameEntry::for_reload(
                db_id,
                ira_models::GameKind::Retro,
                ira_models::TrophySource::from_string(trophy_source),
                "",
                app_id,
                platform_id,
            )
        });
    let mut game = match load_game(&entry, save_dir) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("RA: failed to load game {}: {}", app_id, e);
            return None;
        }
    };
    if !title.is_empty() {
        game.set_name(title);
    }

    ira_platforms::retroachievements::enrich_ra_game(
        &mut game,
        save_dir,
        ra_username,
        ra_web_api_key,
    );

    Some(game)
}

/// Fill empty developer/publisher lists for the Steam-side names: the
/// company cache first — proper ScreenScraper ids — and a local company
/// under a negative id when ScreenScraper has no entry for the studio.
pub(super) fn fill_steam_companies(
    db: &ira_db::DbConn,
    info: &SteamCmdInfo,
    developers: &mut Vec<ira_models::ScraperEntity>,
    publishers: &mut Vec<ira_models::ScraperEntity>,
) {
    let fields = [
        (info.developer.trim(), developers),
        (info.publisher.trim(), publishers),
    ];
    for (field, target) in fields {
        for name in field.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if let Some(entity) = ira_db::steam_company_entity(db, name) {
                target.push(entity);
            }
        }
    }
}

/// Fill a PC game's ScreenScraper-metadata gaps without a match: the
/// store page's synopsis when none is stored, and the company cache's
/// ids when Steam's companies are known there. Runs for freshly added
/// games, enriched games, and after a matcher miss alike.
pub(super) fn garnish_pc_ss_metadata(
    db: &ira_db::DbConn,
    steam: &SteamDataClient,
    db_id: i64,
    app_id: u32,
) {
    let existing = ira_db::scraper_metadata_for_game(db, db_id).ok().flatten();
    let needs_companies = existing
        .as_ref()
        .is_none_or(|meta| meta.developers.is_empty() && meta.publishers.is_empty());
    let needs_synopsis = existing.as_ref().is_none_or(|meta| meta.synopses.is_empty());
    let needs_ratings = existing.as_ref().is_none_or(|meta| meta.classifications.is_empty());
    if !needs_companies && !needs_synopsis && !needs_ratings {
        return;
    }
    let Some(info) = steam.fetch_steamcmd_info(&app_id.to_string()) else {
        return;
    };
    let mut meta = existing.unwrap_or_default();
    let mut companies = std::mem::take(&mut meta.developers);
    let mut publishers = std::mem::take(&mut meta.publishers);
    if needs_companies {
        fill_steam_companies(db, &info, &mut companies, &mut publishers);
    }
    meta.developers = companies;
    meta.publishers = publishers;
    if needs_synopsis || needs_ratings {
        if let Some(extras) = steam.fetch_store_extras(&app_id.to_string()) {
            if needs_synopsis && !extras.synopsis.is_empty() {
                meta.synopses = vec![("en".to_string(), extras.synopsis.clone())];
            }
            if needs_ratings {
                meta.classifications = extras
                    .ratings
                    .iter()
                    .map(|(board, value)| ira_models::ScraperClassification {
                        kind: board.clone(),
                        value: value.clone(),
                    })
                    .collect();
            }
        }
    }
    if let Err(e) = ira_db::store_scraper_metadata(db, db_id, &meta) {
        eprintln!("Could not store PC metadata garnish for {db_id}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{EnrichProgress, Phases};

    #[test]
    fn test_phases_reports_done_total_and_label() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let reporter: EnrichProgress = {
            let calls = calls.clone();
            std::sync::Arc::new(move |done, total, label| {
                calls.lock().unwrap().push((done, total, label.to_string()));
            })
        };
        let phases = Phases(Some(reporter), 3);
        phases.enter(0, "achievements");
        phases.enter(2, "metadata");
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                (0, 3, "achievements".to_string()),
                (2, 3, "metadata".to_string()),
            ]
        );
    }

    #[test]
    fn test_phases_without_reporter_is_silent() {
        let phases = Phases(None, 3);
        phases.enter(0, "achievements");
    }
}
