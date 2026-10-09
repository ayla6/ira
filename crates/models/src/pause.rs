/// Pause-while-overlay-visible: three-level resolution
/// (game override → platform override → global default).
///
/// Every level is opt-in safe: absent overrides fall through, and the
/// global default is off unless the user enables it.
pub fn resolve_pause_game_on_overlay(
    game: Option<bool>,
    platform: Option<bool>,
    global: bool,
) -> bool {
    game.or(platform).unwrap_or(global)
}

#[cfg(test)]
mod tests {
    use super::resolve_pause_game_on_overlay;

    #[test]
    fn test_pause_resolves_game_over_platform_over_global() {
        // Every level set — game wins.
        assert!(resolve_pause_game_on_overlay(Some(true), Some(false), false));
        assert!(!resolve_pause_game_on_overlay(Some(false), Some(true), true));
    }

    #[test]
    fn test_pause_platform_wins_over_global() {
        assert!(resolve_pause_game_on_overlay(None, Some(true), false));
        assert!(!resolve_pause_game_on_overlay(None, Some(false), true));
    }

    #[test]
    fn test_pause_falls_back_to_global() {
        assert!(resolve_pause_game_on_overlay(None, None, true));
        assert!(!resolve_pause_game_on_overlay(None, None, false));
    }
}
