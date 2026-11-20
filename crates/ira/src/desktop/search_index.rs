use ira_models::{AssetType, GameEntry, GameKind};
use std::path::Path;

#[derive(Clone, Debug)]
pub(crate) struct SearchItem {
    pub id: i64,
    pub name: String,
    pub sort_title: String,
    pub suffix: String,
    pub platform: String,
    pub icon: Option<String>,
    pub last_played: i64,
}

pub(crate) fn load_index() -> Result<Vec<SearchItem>, String> {
    let config = ira_config::read_preferences()?;
    let path = Path::new(&config.save_dir).join("ira.db");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = ira_db::open_read_only(&path)?;
    let entries = ira_db::load_all_games_unordered(&db)?;
    let games: Vec<_> = entries.iter().map(|entry| entry.to_game()).collect();
    Ok(entries
        .into_iter()
        .filter(|entry| installed(entry, &config))
        .map(|entry| {
            let dir = ira_parser::entry_data_dir(&config.save_dir, &entry);
            let base = AssetType::Icon.file_base();
            let icon = ira_parser::find_image_file(&dir, &format!("{base}_small"))
                .or_else(|| ira_parser::find_image_file(&dir, base))
                .map(|path| path.to_string_lossy().into_owned());
            SearchItem {
                id: entry.id,
                suffix: entry.to_game().title_suffix(&games),
                sort_title: entry.sort_title,
                name: entry.title,
                platform: format!(
                    "{} {}",
                    entry.platform_id.id(),
                    entry.platform_id.display_name()
                ),
                icon,
                last_played: entry.last_played,
            }
        })
        .collect())
}

fn installed(entry: &GameEntry, config: &ira_config::Config) -> bool {
    if entry.hidden || entry.vanished || entry.title.trim().is_empty() {
        return false;
    }
    match entry.kind {
        GameKind::Retro | GameKind::Switch => {
            !entry.rom_path.is_empty()
                && config
                    .resolve_rom_path(entry.platform_id.id(), &entry.rom_path)
                    .is_some_and(|path| path.exists())
        }
        GameKind::Ps3 | GameKind::Ps4 | GameKind::PsVita | GameKind::WiiU | GameKind::ThreeDS => {
            !entry.rom_path.is_empty() && Path::new(&entry.rom_path).exists()
        }
        _ => entry.game_folder.is_empty() || Path::new(&entry.game_folder).exists(),
    }
}

pub(crate) fn matching_ids(items: &[SearchItem], terms: &[String]) -> Vec<String> {
    let query = ira_models::normalize_name(&terms.join(" "));
    let terms: Vec<_> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<_> = items
        .iter()
        .filter_map(|item| {
            let name = ira_models::normalize_name(&item.name);
            let searchable = format!(
                "{name} {} {}",
                ira_models::normalize_name(&item.sort_title),
                ira_models::normalize_name(&item.platform)
            );
            terms
                .iter()
                .all(|term| searchable.contains(term))
                .then(|| (std::cmp::Reverse(item.last_played), name, item.id))
        })
        .collect();
    matches.sort();
    matches
        .into_iter()
        .take(24)
        .map(|(_, _, id)| id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, name: &str) -> SearchItem {
        SearchItem {
            id,
            name: name.into(),
            sort_title: String::new(),
            suffix: String::new(),
            platform: "nds".into(),
            icon: None,
            last_played: 0,
        }
    }

    #[test]
    fn test_matching_ids_searches_sort_title() {
        let mut game = item(1, "The Legend");
        game.sort_title = "Zelda".into();
        assert_eq!(matching_ids(&[game], &["zelda".into()]), vec!["1"]);
    }

    #[test]
    fn test_matching_ids_normalizes_names_and_matches_all_terms() {
        let games = vec![
            item(1, "Pokémon Black"),
            item(2, "Pokémon White"),
            item(3, "Black Mesa"),
        ];
        assert_eq!(
            matching_ids(&games, &["POKEMON".into(), "black".into()]),
            vec!["1"]
        );
        assert_eq!(
            matching_ids(&games, &["white".into(), "nds".into()]),
            vec!["2"]
        );
        assert!(matching_ids(&games, &[]).is_empty());
        assert!(matching_ids(&games, &["[]".into()]).is_empty());
        assert!(matching_ids(&games, &["missing".into()]).is_empty());
    }

    #[test]
    fn test_matching_ids_orders_by_last_played_then_name_and_bounds_results() {
        let mut games = vec![item(1, "Super Game"), item(2, "Game Plus"), item(3, "Game")];
        games[0].last_played = 10;
        assert_eq!(matching_ids(&games, &["game".into()]), vec!["1", "3", "2"]);
        games.extend((4..100).map(|id| item(id, "Game Extra")));
        assert_eq!(matching_ids(&games, &["game".into()]).len(), 24);
    }

    #[test]
    fn test_installed_rejects_hidden_vanished_and_missing_roms() {
        let config = ira_config::Config::default();
        let mut entry = GameEntry::from_game(&ira_models::Game::default());
        entry.title = "A game".into();
        assert!(installed(&entry, &config));
        entry.hidden = true;
        assert!(!installed(&entry, &config));
        entry.hidden = false;
        entry.vanished = true;
        assert!(!installed(&entry, &config));
        entry.vanished = false;
        entry.kind = GameKind::Retro;
        assert!(!installed(&entry, &config));
        entry.rom_path = "/nonexistent/ira-rom.nds".into();
        assert!(!installed(&entry, &config));
    }
}
