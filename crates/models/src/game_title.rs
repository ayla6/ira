use super::game::Game;
use chrono::Datelike;

/// Variants share their base game's identity and do not create title collisions.
impl Game {
    pub fn title_suffix(&self, games: &[Game]) -> String {
        let game = self;
        let peers: Vec<_> = games
            .iter()
            .filter(|other| {
                other.db_id != game.db_id && other.name.eq_ignore_ascii_case(&game.name)
            })
            .collect();
        if peers.is_empty() {
            return String::new();
        }
        let console = console_label(&game.platform_id);
        let same_console: Vec<_> = peers
            .iter()
            .filter(|other| other.platform_id == game.platform_id)
            .collect();
        if same_console.is_empty() {
            return console.to_string();
        }
        let year = year(game);
        if !year.is_empty() && same_console.iter().all(|other| year != self::year(other)) {
            if peers
                .iter()
                .any(|other| other.platform_id != game.platform_id)
            {
                return format!("{console}, {year}");
            }
            return year;
        }
        console.to_string()
    }
}

fn console_label(platform: &super::consoles::Platform) -> &str {
    use super::consoles::Platform;
    match platform {
        Platform::ThreeDS => "3DS",
        Platform::Console(console) => match console.id {
            "psx" => "PS1",
            "ps2" => "PS2",
            "ps3" => "PS3",
            "ps4" => "PS4",
            "psvita" => "PS Vita",
            "psp" => "PSP",
            "gb" => "GB",
            "gbc" => "GBC",
            "gba" => "GBA",
            "n64" => "N64",
            "n64dd" => "N64DD",
            "nds" => "DS",
            "gc" => "GC",
            "wii" => "Wii",
            "wiiu" => "Wii U",
            "switch" => "Switch",
            "virtualboy" => "VB",
            "dc" => "DC",
            "3do" => "3DO",
            "md" => "MD",
            "sms" => "SMS",
            "gg" => "GG",
            "ngp" => "NGP",
            "pce" => "PCE",
            "pcecd" => "PCE CD",
            "ws" => "WS",
            "wsc" => "WSC",
            _ => console.short_name(),
        },
        _ => platform.display_name(),
    }
}

fn year(game: &Game) -> String {
    (game.release_timestamp > 0)
        .then(|| chrono::DateTime::from_timestamp(game.release_timestamp, 0))
        .flatten()
        .map(|date| date.year().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_suffix_uses_console_for_cross_platform_duplicates() {
        let first = Game {
            db_id: 1,
            name: "Game".into(),
            platform_id: super::super::consoles::Platform::parse("psx"),
            ..Default::default()
        };
        let second = Game {
            db_id: 2,
            platform_id: super::super::consoles::Platform::parse("ps2"),
            ..first.clone()
        };
        assert_eq!(first.title_suffix(&[first.clone(), second]), "PS1");
    }

    #[test]
    fn test_suffix_distinguishes_years_and_ignores_variants() {
        let first = Game {
            db_id: 1,
            name: "Doom".into(),
            release_timestamp: 757382400,
            ..Default::default()
        };
        let second = Game {
            db_id: 2,
            name: "Doom".into(),
            release_timestamp: 1463097600,
            ..Default::default()
        };
        assert_eq!(first.title_suffix(&[first.clone(), second]), "1994");
        let variant = Game {
            variant_id: Some(3),
            ..first.clone()
        };
        assert!(first.title_suffix(&[first.clone(), variant]).is_empty());
    }
}
