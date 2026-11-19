use crate::Game;

pub(super) fn markup(game: &Game, games: &[Game]) -> String {
    let name = glib::markup_escape_text(&game.name);
    let suffix = game.title_suffix(games);
    if suffix.is_empty() {
        return name.to_string();
    }
    format!(
        "{name} <span alpha=\"55%\">({})</span>",
        glib::markup_escape_text(&suffix)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markup_escapes_titles_and_unique_names() {
        let game = Game {
            db_id: 1,
            name: "<Game>".into(),
            ..Default::default()
        };
        assert_eq!(markup(&game, std::slice::from_ref(&game)), "&lt;Game&gt;");
    }
}
