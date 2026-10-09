use std::path::{Path, PathBuf};

use ira_models::GameKind;

pub const GALAXY_ID: &str = "100000000000000000";

/// `<save_dir>/data/<group>/<key>` — the one shape every per-game data
/// directory shares. `<group>` is a [`GameKind`] folder name
/// (so renames in the model can't silently orphan save paths) or `"ira"`
/// for local games.
fn data_dir_in(save_dir: &str, group: &str, key: impl std::fmt::Display) -> PathBuf {
    Path::new(save_dir)
        .join("data")
        .join(group)
        .join(key.to_string())
}

pub fn data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::Steam.as_str(), app_id)
}

pub fn ps4_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::Ps4.as_str(), app_id)
}

pub fn ps3_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::Ps3.as_str(), app_id)
}

pub fn vita_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::PsVita.as_str(), app_id)
}

pub fn wiiu_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::WiiU.as_str(), app_id)
}

pub fn three_ds_data_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir_in(save_dir, GameKind::ThreeDS.as_str(), app_id)
}

pub fn switch_data_dir(save_dir: &str, db_id: i64) -> PathBuf {
    data_dir_in(save_dir, GameKind::Switch.as_str(), db_id)
}

pub fn retro_data_dir(save_dir: &str, db_id: i64) -> PathBuf {
    data_dir_in(save_dir, GameKind::Retro.as_str(), db_id)
}

fn data_dir_for(
    save_dir: &str,
    kind: GameKind,
    id: &str,
    db_id: i64,
    enriched: bool,
) -> PathBuf {
    match kind {
        GameKind::Retro => retro_data_dir(save_dir, db_id),
        GameKind::Switch => switch_data_dir(save_dir, db_id),
        GameKind::Ps4 => ps4_data_dir(save_dir, id),
        GameKind::Ps3 => ps3_data_dir(save_dir, id),
        GameKind::PsVita => vita_data_dir(save_dir, id),
        GameKind::WiiU => wiiu_data_dir(save_dir, id),
        GameKind::ThreeDS => three_ds_data_dir(save_dir, id),
        _ if enriched => data_dir(save_dir, id),
        _ if !id.is_empty() => data_dir(save_dir, id),
        // No store id at all (manually added PC games): key the
        // directory on the unique row id, so games never share one.
        _ => local_data_dir(save_dir, db_id),
    }
}

/// The per-game data directory for games with no id of their own,
/// keyed on Ira's row id.
pub fn local_data_dir(save_dir: &str, db_id: i64) -> PathBuf {
    data_dir_in(save_dir, "ira", db_id)
}

pub fn game_data_dir(save_dir: &str, game: &ira_models::Game) -> PathBuf {
    data_dir_for(
        save_dir,
        game.kind,
        &game.app_id,
        game.db_id,
        game.trophy_source.has_steam_enrichment(),
    )
}

pub fn entry_data_dir(save_dir: &str, entry: &ira_models::GameEntry) -> PathBuf {
    let id = if !entry.steam_id.is_empty() {
        &entry.steam_id
    } else {
        entry.external_id()
    };
    data_dir_for(
        save_dir,
        entry.kind,
        id,
        entry.id,
        entry.trophy_source.has_steam_enrichment(),
    )
}

pub fn achievements_dir(save_dir: &str, app_id: &str) -> PathBuf {
    data_dir(save_dir, app_id).join("achievements")
}

/// Where a game's unlock state lives: Steam-native achievements read
/// from the client, everything else from a JSON status map. GOG games
/// key on their product id, Goldberg games on the emulator app id —
/// both are the platform-native id.
pub fn unlock_status_path(
    save_dir: &str,
    trophy_source: ira_models::TrophySource,
    app_id: &str,
    native_id: &str,
) -> PathBuf {
    match trophy_source {
        ira_models::TrophySource::Nge => Path::new(save_dir)
            .join("emulator_saves")
            .join("nge")
            .join(GALAXY_ID)
            .join(native_id)
            .join("achievements.json"),
        _ => Path::new(save_dir)
            .join("emulator_saves")
            .join("gbe")
            .join(app_id)
            .join("achievements.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_console_data_dirs() {
        assert_eq!(
            vita_data_dir("/save", "PCSF00001"),
            PathBuf::from("/save/data/psvita/PCSF00001")
        );
        assert_eq!(
            wiiu_data_dir("/save", "0005000010101d00"),
            PathBuf::from("/save/data/wiiu/0005000010101d00")
        );
        assert_eq!(
            switch_data_dir("/save", 42),
            PathBuf::from("/save/data/switch/42")
        );
    }
}
