//! Library search box queries: bare words match accent-insensitively in any
//! order, "quoted phrases" match exactly with accents intact.

use crate::Game;
use ira_models::fold_accents;

/// A parsed search query.
///
/// Every bare word must appear in the name, sort title or platform —
/// case- and accent-insensitively, in any position. Every quoted phrase
/// must appear as a case-insensitive substring with accents preserved, so
/// `pokemon` finds "Pokémon" but `"pokémon"` only pins the accented form.
#[derive(Default)]
pub struct SearchQuery {
    phrases: Vec<String>,
    words: Vec<String>,
}

impl SearchQuery {
    pub fn parse(raw: &str) -> Self {
        let mut query = SearchQuery::default();
        let mut rest = raw;
        while let Some(open) = rest.find('"') {
            push_words(&mut query.words, &rest[..open]);
            let after = &rest[open + 1..];
            match after.find('"') {
                Some(close) => {
                    push_phrase(&mut query.phrases, &after[..close]);
                    rest = &after[close + 1..];
                }
                None => {
                    // Unterminated quote: the tail still matches verbatim.
                    push_phrase(&mut query.phrases, after);
                    rest = "";
                }
            }
        }
        push_words(&mut query.words, rest);
        query
    }

    /// True when nothing was parsed, i.e. the query matches every game.
    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty() && self.words.is_empty()
    }

    pub fn matches(&self, game: &Game) -> bool {
        if self.is_empty() {
            return true;
        }
        let sort_title = game.sort_title.to_lowercase();
        let platform = game.platform_id.to_lowercase();
        let haystacks = [game.name_lower.as_str(), sort_title.as_str(), platform.as_str()];

        if !self
            .phrases
            .iter()
            .all(|p| haystacks.iter().any(|h| h.contains(p)))
        {
            return false;
        }
        if self.words.is_empty() {
            return true;
        }
        let folded: Vec<String> = haystacks.iter().map(|h| fold_accents(h)).collect();
        self.words
            .iter()
            .all(|w| folded.iter().any(|h| h.contains(w)))
    }
}

fn push_words(words: &mut Vec<String>, text: &str) {
    words.extend(
        text.split_whitespace()
            .map(fold_accents)
            .filter(|w| !w.is_empty()),
    );
}

fn push_phrase(phrases: &mut Vec<String>, text: &str) {
    let phrase = text.trim().to_lowercase();
    if !phrase.is_empty() {
        phrases.push(phrase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str) -> Game {
        Game {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            ..Default::default()
        }
    }

    #[test]
    fn test_matches_words_ignore_accents_and_case() {
        let g = game("Pokémon Emerald");
        assert!(SearchQuery::parse("pokemon").matches(&g));
        assert!(SearchQuery::parse("pokémon").matches(&g));
        assert!(SearchQuery::parse("POKEMON emerald").matches(&g));
    }

    #[test]
    fn test_matches_words_in_any_order() {
        let g = game("Super Mario Kart");
        assert!(SearchQuery::parse("mario kart").matches(&g));
        assert!(SearchQuery::parse("kart super").matches(&g));
        assert!(!SearchQuery::parse("kart zelda").matches(&g));
    }

    #[test]
    fn test_quoted_phrase_requires_exact_accents() {
        let plain = game("Pokemon Emerald");
        let accented = game("Pokémon Emerald");
        let accented_query = SearchQuery::parse("\"pokémon\"");
        assert!(!accented_query.matches(&plain));
        assert!(accented_query.matches(&accented));
        let plain_query = SearchQuery::parse("\"pokemon\"");
        assert!(plain_query.matches(&plain));
        assert!(!plain_query.matches(&accented));
    }

    #[test]
    fn test_quoted_phrase_keeps_word_order() {
        let g = game("Super Mario Kart");
        assert!(SearchQuery::parse("\"mario kart\"").matches(&g));
        assert!(!SearchQuery::parse("\"kart mario\"").matches(&g));
    }

    #[test]
    fn test_mixed_words_and_phrases() {
        let g = game("Pokémon Emerald Version");
        assert!(SearchQuery::parse("emerald \"version\"").matches(&g));
        assert!(!SearchQuery::parse("ruby \"version\"").matches(&g));
    }

    #[test]
    fn test_unterminated_quote_matches_tail_verbatim() {
        let g = game("Pokémon Emerald");
        assert!(SearchQuery::parse("pokemon \"emer").matches(&g));
        assert!(!SearchQuery::parse("pokemon \"ruby").matches(&g));
    }

    #[test]
    fn test_empty_query_matches_everything() {
        let g = game("Anything");
        assert!(SearchQuery::parse("").matches(&g));
        assert!(SearchQuery::parse("   ").matches(&g));
        assert!(SearchQuery::parse("\"\"").matches(&g));
    }

    #[test]
    fn test_matches_sort_title_and_platform() {
        let g = Game {
            name: "Halo".to_string(),
            name_lower: "halo".to_string(),
            sort_title: "Halo 2".to_string(),
            platform_id: "xbox".to_string(),
            ..Default::default()
        };
        assert!(SearchQuery::parse("2").matches(&g));
        assert!(SearchQuery::parse("xbox").matches(&g));
        assert!(!SearchQuery::parse("zelda").matches(&g));
    }
}
