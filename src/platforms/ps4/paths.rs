use std::path::PathBuf;

/// shadPS4 user data directory.
/// Linux: ~/.local/share/shadPS4/
pub fn shadps4_user_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local").join("share").join("shadPS4")
}

/// Path to play_time.txt
pub fn play_time_path() -> PathBuf {
    shadps4_user_dir().join("play_time.txt")
}

/// Path to the trophy directory
pub fn trophy_dir(npwr_id: &str) -> PathBuf {
    shadps4_user_dir().join("trophy").join(npwr_id)
}

/// Path to the user trophy unlock-state XML
pub fn user_trophy_path(npwr_id: &str) -> PathBuf {
    shadps4_user_dir()
        .join("home")
        .join("1000")
        .join("trophy")
        .join(format!("{}.xml", npwr_id))
}
