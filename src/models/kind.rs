// Kinds (how the game is managed/launched)
pub const LUTRIS: &str = "lutris";
pub const WINE: &str = "wine";
pub const LINUX: &str = "linux";
pub const PS4: &str = "ps4";
pub const SGDB: &str = "sgdb";
pub const STEAM: &str = "steam";

// Trophy sources (where achievements come from)
pub const GSE: &str = "gse";
pub const NGE: &str = "nge";
pub const STEAM_NATIVE: &str = "steam";

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
        "other" => "Other",
        _ => "Other",
    }
}
