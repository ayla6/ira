use std::path::{Path, PathBuf};

pub fn ra_data_dir(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ra")
}

pub fn console_games_path(save_dir: &str, console_id: u32) -> PathBuf {
    ra_data_dir(save_dir).join(format!("console_{}.json", console_id))
}

pub fn game_dir(save_dir: &str, game_id: &str) -> PathBuf {
    ra_data_dir(save_dir).join(game_id)
}

pub fn web_progress_path(save_dir: &str, game_id: &str) -> PathBuf {
    game_dir(save_dir, game_id).join("web_progress.json")
}

pub fn achievements_dir(save_dir: &str, game_id: &str) -> PathBuf {
    game_dir(save_dir, game_id).join("achievements")
}

pub fn badge_path(save_dir: &str, game_id: &str, badge_name: &str) -> PathBuf {
    achievements_dir(save_dir, game_id).join(format!("{}.webp", badge_name))
}

pub fn badge_locked_path(save_dir: &str, game_id: &str, badge_name: &str) -> PathBuf {
    achievements_dir(save_dir, game_id).join(format!("{}_lock.webp", badge_name))
}
