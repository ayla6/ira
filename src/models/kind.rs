// Kinds (how the game is managed/launched)
pub const LUTRIS: &str = "lutris";
pub const WINE: &str = "wine";
pub const LINUX: &str = "linux";
pub const PS4: &str = "ps4";
pub const SGDB: &str = "sgdb";

// Trophy sources (where achievements come from)
pub const GSE: &str = "gse";
pub const NGE: &str = "nge";

pub fn has_steam_enrichment(trophy_source: &str) -> bool {
    trophy_source == GSE || trophy_source == NGE
}

pub fn kind_display_name(kind: &str) -> &str {
    match kind {
        LUTRIS => "Lutris",
        WINE => "Windows",
        LINUX => "Linux",
        PS4 => "PS4",
        SGDB => "SteamGridDB",
        "other" => "Other",
        _ => "Other",
    }
}
