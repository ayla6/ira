use std::path::{Path, PathBuf};

pub fn ra_data_dir(save_dir: &str) -> PathBuf {
    Path::new(save_dir).join("data").join("ra")
}

pub fn console_games_path(save_dir: &str, console_id: u32) -> PathBuf {
    ra_data_dir(save_dir).join(format!("console_{}.json", console_id))
}

pub fn game_data_path(save_dir: &str, game_id: &str) -> PathBuf {
    ra_data_dir(save_dir).join(format!("game_{}.json", game_id))
}

pub fn unlocks_path(save_dir: &str, game_id: &str) -> PathBuf {
    ra_data_dir(save_dir).join(format!("unlocks_{}.json", game_id))
}

pub fn badges_dir(save_dir: &str) -> PathBuf {
    ra_data_dir(save_dir).join("badges")
}

pub fn badge_path(save_dir: &str, badge_name: &str) -> PathBuf {
    badges_dir(save_dir).join(format!("{}.png", badge_name))
}

pub fn badge_locked_path(save_dir: &str, badge_name: &str) -> PathBuf {
    badges_dir(save_dir).join(format!("{}_lock.png", badge_name))
}

pub fn game_icon_path(save_dir: &str, game_id: &str) -> PathBuf {
    ra_data_dir(save_dir).join(format!("game_{}_icon.png", game_id))
}
