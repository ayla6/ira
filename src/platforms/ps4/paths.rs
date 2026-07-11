use std::path::PathBuf;

/// shadPS4 user data directory.
/// Linux: ~/.local/share/shadPS4/
pub fn shadps4_user_dir() -> PathBuf {
    xdg_data_dir_fallback("shadPS4")
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

fn xdg_data_dir_fallback(app: &str) -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let p = PathBuf::from(xdg).join(app);
        if p.exists() {
            return p;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local").join("share").join(app)
    } else {
        PathBuf::from(".").join(".local").join("share").join(app)
    }
}
