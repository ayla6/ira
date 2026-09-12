//! Library search box queries: bare words match loosely — accents, word
//! order and punctuation don't matter. "Quoted phrases" stay contiguous
//! and accent-sensitive, but punctuation doesn't matter there either.

use crate::Game;
use ira_models::{normalize_name, normalize_phrase};

/// A parsed search query.
///
/// Every bare word must appear in the name, sort title or platform —
/// case-, accent- and punctuation-insensitively, in any position. Every
/// quoted phrase must appear contiguously; accents count inside quotes,
/// punctuation still doesn't. Between letters, a separator run is
/// equivalent to nothing or to any other separator run, so lets, let's,
/// let`s and let s all find "Let's Go".
#[derive(Default)]
pub struct SearchQuery {
    words: Vec<String>,
    phrases: Vec<Phrase>,
}

/// A quoted phrase in both comparison forms: separators as spaces and
/// separators removed — so `"let s go"` and `"let's go"` find the same
/// titles.
#[derive(Default)]
struct Phrase {
    spaced: String,
    joined: String,
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
        self.words.is_empty() && self.phrases.is_empty()
    }

    pub fn matches(&self, game: &Game) -> bool {
        if self.is_empty() {
            return true;
        }
        let fields = [&game.name, &game.sort_title, &game.platform_id];
        if !self.words.is_empty() {
            let spaced: Vec<String> = fields.iter().map(|f| normalize_name(f)).collect();
            let joined: Vec<String> = spaced.iter().map(|h| joined_form(h)).collect();
            if !self
                .words
                .iter()
                .all(|w| contains_any(w, &spaced) || contains_any(w, &joined))
            {
                return false;
            }
        }
        if self.phrases.is_empty() {
            return true;
        }
        let spaced: Vec<String> = fields.iter().map(|f| normalize_phrase(f)).collect();
        let joined: Vec<String> = spaced.iter().map(|h| joined_form(h)).collect();
        self.phrases.iter().all(|p| {
            contains_any(&p.spaced, &spaced) || contains_any(&p.joined, &joined)
        })
    }
}

/// normalize_* output separates tokens with single spaces; dropping them
/// gives the "separator = nothing" comparison form.
fn joined_form(spaced: &str) -> String {
    spaced.replace(' ', "")
}

fn contains_any(needle: &str, haystacks: &[String]) -> bool {
    haystacks.iter().any(|h| h.contains(needle))
}

fn push_words(words: &mut Vec<String>, text: &str) {
    words.extend(
        text.split_whitespace()
            .map(|w| joined_form(&normalize_name(w)))
            .filter(|w| !w.is_empty()),
    );
}

fn push_phrase(phrases: &mut Vec<Phrase>, text: &str) {
    let spaced = normalize_phrase(text.trim());
    if spaced.is_empty() {
        return;
    }
    let joined = joined_form(&spaced);
    phrases.push(Phrase { spaced, joined });
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
    fn test_matches_ignores_punctuation() {
        let g = game("Dr. Mario World");
        assert!(SearchQuery::parse("dr mario").matches(&g));
        let g = game("Final Fantasy VII");
        assert!(SearchQuery::parse("final, fantasy!!").matches(&g));
    }

    #[test]
    fn test_apostrophe_variants_all_find_lets_go() {
        let accented = game("Pokémon: Let's Go, Eevee!");
        for query in ["lets", "let's", "let`s", "letʼs", "let s"] {
            assert!(
                SearchQuery::parse(query).matches(&accented),
                "query {query:?} should find {:?}",
                accented.name
            );
        }
        // The title side may use a different variant than the query.
        let plain = game("Pokemon Lets Go Eevee");
        assert!(SearchQuery::parse("let's go").matches(&plain));
        let spaced_title = game("Pokemon Let s Go Eevee");
        assert!(SearchQuery::parse("let's go").matches(&spaced_title));
    }

    #[test]
    fn test_matches_ampersand_as_and() {
        let g = game("Mario & Luigi: Superstar Saga");
        assert!(SearchQuery::parse("mario and luigi").matches(&g));
        assert!(SearchQuery::parse("mario luigi superstar").matches(&g));
    }

    #[test]
    fn test_matches_ignores_bracketed_tags() {
        let g = game("Emerald (USA) (Rev 1)");
        assert!(SearchQuery::parse("emerald").matches(&g));
        assert!(!SearchQuery::parse("usa").matches(&g));
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
    fn test_quoted_phrase_ignores_punctuation() {
        let accented = game("Pokémon: Let's Go, Eevee!");
        assert!(SearchQuery::parse("\"pokémon: lets go\"").matches(&accented));
        // Separator run in the quote vs apostrophe in the title.
        assert!(SearchQuery::parse("\"let s go\"").matches(&accented));
        let spaced_title = game("Pokemon: Let s Go Eevee");
        assert!(SearchQuery::parse("\"let's go\"").matches(&spaced_title));
        let plain = game("Pokemon: Lets Go, Eevee!");
        assert!(!SearchQuery::parse("\"pokémon: lets go\"").matches(&plain));
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
        assert!(SearchQuery::parse("!!!").matches(&g));
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
