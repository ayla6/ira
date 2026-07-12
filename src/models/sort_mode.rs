use std::cmp::Ordering;

use super::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Alphabetical,
    Completion,
    HoursPlayed,
    LastPlayed,
    ReleaseDate,
    MetacriticScore,
    SteamReview,
}

impl SortMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SortMode::Alphabetical => "alphabetical",
            SortMode::Completion => "completion",
            SortMode::HoursPlayed => "hours_played",
            SortMode::LastPlayed => "last_played",
            SortMode::ReleaseDate => "release_date",
            SortMode::MetacriticScore => "metacritic_score",
            SortMode::SteamReview => "steam_review",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "completion" => SortMode::Completion,
            "hours_played" => SortMode::HoursPlayed,
            "last_played" => SortMode::LastPlayed,
            "release_date" => SortMode::ReleaseDate,
            "metacritic_score" => SortMode::MetacriticScore,
            "steam_review" => SortMode::SteamReview,
            _ => SortMode::Alphabetical,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            SortMode::Alphabetical => "Alphabetical",
            SortMode::Completion => "% Complete",
            SortMode::HoursPlayed => "Hours Played",
            SortMode::LastPlayed => "Last Played",
            SortMode::ReleaseDate => "Release Date",
            SortMode::MetacriticScore => "Metacritic Score",
            SortMode::SteamReview => "Steam Review",
        }
    }

    pub const ALL: &[SortMode] = &[
        SortMode::Alphabetical,
        SortMode::Completion,
        SortMode::HoursPlayed,
        SortMode::LastPlayed,
        SortMode::ReleaseDate,
        SortMode::MetacriticScore,
        SortMode::SteamReview,
    ];

    pub fn compare(&self, a: &Game, b: &Game) -> Ordering {
        match self {
            SortMode::Alphabetical => a.sort_key().to_lowercase().cmp(&b.sort_key().to_lowercase()),
            SortMode::Completion => {
                let pct_a = a.completion_pct();
                let pct_b = b.completion_pct();
                pct_b.partial_cmp(&pct_a).unwrap_or(Ordering::Equal)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
            SortMode::HoursPlayed => {
                b.playtime.partial_cmp(&a.playtime).unwrap_or(Ordering::Equal)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
            SortMode::LastPlayed => {
                b.lastplayed.cmp(&a.lastplayed)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
            SortMode::ReleaseDate => {
                sort_desc_unknowns_last(a.release_timestamp, b.release_timestamp)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
            SortMode::MetacriticScore => {
                sort_desc_unknowns_last(a.metacritic_score, b.metacritic_score)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
            SortMode::SteamReview => {
                sort_desc_unknowns_last(a.steam_review_score, b.steam_review_score)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
        }
    }
}

fn sort_desc_unknowns_last(a: i64, b: i64) -> Ordering {
    let a_unknown = a <= 0;
    let b_unknown = b <= 0;
    match (a_unknown, b_unknown) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => b.cmp(&a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game(name: &str) -> Game {
        Game { name: name.to_string(), ..Default::default() }
    }

    #[test]
    fn test_sort_mode_roundtrip() {
        for mode in SortMode::ALL {
            let s = mode.as_str();
            let back = SortMode::from_str(s);
            assert_eq!(*mode, back);
        }
    }

    #[test]
    fn test_sort_mode_unknown_defaults_alphabetical() {
        assert_eq!(SortMode::from_str("garbage"), SortMode::Alphabetical);
    }

    #[test]
    fn test_sort_alphabetical_case_insensitive() {
        let a = make_game("Half-Life");
        let b = make_game("half-life 2");
        assert_eq!(SortMode::Alphabetical.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_sort_completion_descending() {
        let mut a = make_game("Game A");
        a.earned_count = 5;
        a.total_count = 10;
        let mut b = make_game("Game B");
        b.earned_count = 8;
        b.total_count = 10;
        assert_eq!(SortMode::Completion.compare(&a, &b), Ordering::Greater);
    }

    #[test]
    fn test_sort_hours_played_descending() {
        let mut a = make_game("Game A");
        a.playtime = 5.0;
        let mut b = make_game("Game B");
        b.playtime = 10.0;
        assert_eq!(SortMode::HoursPlayed.compare(&a, &b), Ordering::Greater);
    }

    #[test]
    fn test_sort_unknowns_last() {
        assert_eq!(sort_desc_unknowns_last(0, 80), Ordering::Greater);
        assert_eq!(sort_desc_unknowns_last(80, 0), Ordering::Less);
        assert_eq!(sort_desc_unknowns_last(0, 0), Ordering::Equal);
        assert_eq!(sort_desc_unknowns_last(90, 80), Ordering::Less);
    }
}
