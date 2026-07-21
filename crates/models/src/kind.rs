use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameKind {
    Wine,
    Linux,
    #[default]
    Other,
    Ps4,
    Ps3,
    Steam,
    Retro,
}

impl GameKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GameKind::Wine => "wine",
            GameKind::Linux => "linux",
            GameKind::Ps4 => "ps4",
            GameKind::Ps3 => "ps3",
            GameKind::Steam => "steam",
            GameKind::Retro => "retro",
            GameKind::Other => "other",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "wine" => GameKind::Wine,
            "linux" => GameKind::Linux,
            "ps4" => GameKind::Ps4,
            "ps3" => GameKind::Ps3,
            "steam" => GameKind::Steam,
            "retro" => GameKind::Retro,
            _ => GameKind::Other,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            GameKind::Wine => "Windows",
            GameKind::Linux => "Linux",
            GameKind::Ps4 => "PS4",
            GameKind::Ps3 => "PS3",
            GameKind::Steam => "Steam",
            GameKind::Retro => "Retro",
            GameKind::Other => "Other",
        }
    }

    /// PS4 and PS3 games — both use the NPCommId (NPWR) trophy system
    /// with TROP.XML / TROPCONF.SFM definitions and per-trophy icons.
    pub fn is_trophy_console(self) -> bool {
        matches!(self, GameKind::Ps4 | GameKind::Ps3)
    }
}

impl std::fmt::Display for GameKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for GameKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GameKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(GameKind::from_string(&s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrophySource {
    #[default]
    Empty,
    Gse,
    Nge,
    SteamNative,
    Ra,
}

impl TrophySource {
    pub fn as_str(self) -> &'static str {
        match self {
            TrophySource::Empty => "",
            TrophySource::Gse => "gse",
            TrophySource::Nge => "nge",
            TrophySource::SteamNative => "steam",
            TrophySource::Ra => "ra",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "gse" => TrophySource::Gse,
            "nge" => TrophySource::Nge,
            "steam" => TrophySource::SteamNative,
            "ra" => TrophySource::Ra,
            _ => TrophySource::Empty,
        }
    }

    pub fn has_steam_enrichment(self) -> bool {
        matches!(self, TrophySource::Gse | TrophySource::Nge | TrophySource::SteamNative)
    }
}

impl std::fmt::Display for TrophySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TrophySource {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TrophySource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(TrophySource::from_string(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_kind_roundtrip() {
        for kind in [GameKind::Wine, GameKind::Linux, GameKind::Ps4, GameKind::Ps3, GameKind::Steam, GameKind::Retro, GameKind::Other] {
            let s = kind.as_str();
            let back = GameKind::from_string(s);
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn test_game_kind_unknown() {
        assert_eq!(GameKind::from_string("garbage"), GameKind::Other);
    }

    #[test]
    fn test_game_kind_default() {
        assert_eq!(GameKind::default(), GameKind::Other);
    }

    #[test]
    fn test_trophy_source_roundtrip() {
        for ts in [TrophySource::Gse, TrophySource::Nge, TrophySource::SteamNative, TrophySource::Ra, TrophySource::Empty] {
            let s = ts.as_str();
            let back = TrophySource::from_string(s);
            assert_eq!(ts, back);
        }
    }

    #[test]
    fn test_trophy_source_unknown() {
        assert_eq!(TrophySource::from_string("garbage"), TrophySource::Empty);
    }

    #[test]
    fn test_has_steam_enrichment_gse() {
        assert!(TrophySource::Gse.has_steam_enrichment());
    }

    #[test]
    fn test_has_steam_enrichment_nge() {
        assert!(TrophySource::Nge.has_steam_enrichment());
    }

    #[test]
    fn test_has_steam_enrichment_steam() {
        assert!(TrophySource::SteamNative.has_steam_enrichment());
    }

    #[test]
    fn test_has_steam_enrichment_ra() {
        assert!(!TrophySource::Ra.has_steam_enrichment());
    }

    #[test]
    fn test_has_steam_enrichment_empty() {
        assert!(!TrophySource::Empty.has_steam_enrichment());
    }

    #[test]
    fn test_kind_display_name_all() {
        assert_eq!(GameKind::Wine.display_name(), "Windows");
        assert_eq!(GameKind::Linux.display_name(), "Linux");
        assert_eq!(GameKind::Ps4.display_name(), "PS4");
        assert_eq!(GameKind::Ps3.display_name(), "PS3");
        assert_eq!(GameKind::Steam.display_name(), "Steam");
        assert_eq!(GameKind::Retro.display_name(), "Retro");
        assert_eq!(GameKind::Other.display_name(), "Other");
    }

    #[test]
    fn test_is_trophy_console() {
        assert!(GameKind::Ps4.is_trophy_console());
        assert!(GameKind::Ps3.is_trophy_console());
        assert!(!GameKind::Steam.is_trophy_console());
        assert!(!GameKind::Retro.is_trophy_console());
        assert!(!GameKind::Wine.is_trophy_console());
        assert!(!GameKind::Other.is_trophy_console());
    }
}
