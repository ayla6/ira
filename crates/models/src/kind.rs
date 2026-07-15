// Kinds (how the game is managed/launched)
pub const LUTRIS: &str = "lutris";
pub const WINE: &str = "wine";
pub const LINUX: &str = "linux";
pub const PS4: &str = "ps4";
pub const SGDB: &str = "sgdb";
pub const STEAM: &str = "steam";
pub const RETRO: &str = "retro";

// Trophy sources (where achievements come from)
pub const GSE: &str = "gse";
pub const NGE: &str = "nge";
pub const STEAM_NATIVE: &str = "steam";
pub const RA: &str = "ra";

pub fn has_steam_enrichment(trophy_source: &str) -> bool {
    trophy_source == GSE || trophy_source == NGE || trophy_source == STEAM_NATIVE
}

pub fn kind_display_name(kind: &str) -> &str {
    match kind {
        LUTRIS => "Lutris",
        WINE => "Windows",
        LINUX => "Linux",
        PS4 => "PS4",
        SGDB => "SteamGridDB",
        STEAM => "Steam",
        RETRO => "Retro",
        "other" => "Other",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_steam_enrichment_gse() {
        assert!(has_steam_enrichment(GSE));
    }

    #[test]
    fn test_has_steam_enrichment_nge() {
        assert!(has_steam_enrichment(NGE));
    }

    #[test]
    fn test_has_steam_enrichment_steam() {
        assert!(has_steam_enrichment(STEAM_NATIVE));
    }

    #[test]
    fn test_has_steam_enrichment_ra() {
        assert!(!has_steam_enrichment(RA));
    }

    #[test]
    fn test_has_steam_enrichment_empty() {
        assert!(!has_steam_enrichment(""));
    }

    #[test]
    fn test_has_steam_enrichment_unknown() {
        assert!(!has_steam_enrichment("unknown"));
    }

    #[test]
    fn test_kind_display_name_lutris() {
        assert_eq!(kind_display_name(LUTRIS), "Lutris");
    }

    #[test]
    fn test_kind_display_name_wine() {
        assert_eq!(kind_display_name(WINE), "Windows");
    }

    #[test]
    fn test_kind_display_name_linux() {
        assert_eq!(kind_display_name(LINUX), "Linux");
    }

    #[test]
    fn test_kind_display_name_ps4() {
        assert_eq!(kind_display_name(PS4), "PS4");
    }

    #[test]
    fn test_kind_display_name_sgdb() {
        assert_eq!(kind_display_name(SGDB), "SteamGridDB");
    }

    #[test]
    fn test_kind_display_name_steam() {
        assert_eq!(kind_display_name(STEAM), "Steam");
    }

    #[test]
    fn test_kind_display_name_retro() {
        assert_eq!(kind_display_name(RETRO), "Retro");
    }

    #[test]
    fn test_kind_display_name_other() {
        assert_eq!(kind_display_name("other"), "Other");
    }

    #[test]
    fn test_kind_display_name_unknown() {
        assert_eq!(kind_display_name("unknown"), "Other");
    }
}
