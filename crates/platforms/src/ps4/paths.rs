use std::path::PathBuf;

pub const SHADPS4_FLATPAK_ID: &str = "net.shadps4.shadPS4";

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// shadPS4 user data directory.
/// Linux: ~/.local/share/shadPS4/
pub fn shadps4_user_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .map(|p| p.join("shadPS4"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("shadPS4")
        })
}

pub fn shadps4_user_dir_for(executable: &str) -> PathBuf {
    let Some(app_id) = executable.strip_prefix("flatpak:") else {
        return shadps4_user_dir();
    };
    home_dir()
        .join(".var")
        .join("app")
        .join(app_id)
        .join("data")
        .join("shadPS4")
}

/// Path to play_time.txt
pub fn play_time_path() -> PathBuf {
    shadps4_user_dir().join("play_time.txt")
}

pub fn play_time_path_for(executable: &str) -> PathBuf {
    shadps4_user_dir_for(executable).join("play_time.txt")
}

/// Path to the trophy directory
pub fn trophy_dir(npwr_id: &str) -> PathBuf {
    shadps4_user_dir().join("trophy").join(npwr_id)
}

pub fn trophy_dir_for(executable: &str, npwr_id: &str) -> PathBuf {
    shadps4_user_dir_for(executable)
        .join("trophy")
        .join(npwr_id)
}

/// Path to the user trophy unlock-state XML
pub fn user_trophy_path(npwr_id: &str) -> PathBuf {
    user_trophy_path_for("", npwr_id)
}

pub fn user_trophy_path_for(executable: &str, npwr_id: &str) -> PathBuf {
    shadps4_user_dir_for(executable)
        .join("home")
        .join("1000")
        .join("trophy")
        .join(format!("{}.xml", npwr_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatpak_user_root() {
        let path = shadps4_user_dir_for("flatpak:net.shadps4.shadPS4");
        assert!(path
            .to_string_lossy()
            .ends_with(".var/app/net.shadps4.shadPS4/data/shadPS4"));
    }
}
