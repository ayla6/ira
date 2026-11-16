use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 720,
            maximized: false,
        }
    }
}

impl WindowState {
    pub fn normal_size(self) -> (i32, i32) {
        (self.width.clamp(900, 8192), self.height.clamp(650, 8192))
    }
}

fn path() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_cache_home()
        .unwrap_or_else(|| std::env::temp_dir().join("ira-cache"))
        .join("ira/window-state.json")
}

pub fn load_window_state() -> Result<WindowState, String> {
    match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Read window state: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WindowState::default()),
        Err(e) => Err(format!("Read window state: {e}")),
    }
}

pub fn save_window_state(state: WindowState) -> Result<(), String> {
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_state_defaults_and_roundtrip() {
        assert_eq!(
            serde_json::from_str::<WindowState>("{}").unwrap(),
            WindowState::default()
        );
        let state = WindowState {
            width: 1400,
            height: 900,
            maximized: true,
        };
        let encoded = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<WindowState>(&encoded).unwrap(),
            state
        );
    }

    #[test]
    fn test_window_state_normal_size_bounds_invalid_dimensions() {
        let state = WindowState {
            width: -1,
            height: i32::MAX,
            maximized: false,
        };
        assert_eq!(state.normal_size(), (900, 8192));
        assert_eq!(WindowState::default().normal_size(), (1200, 720));
    }
}
