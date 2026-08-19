use crate::Game;
use gtk4::prelude::*;

use super::achievement_view::build_achievements_view;
use super::css::*;
use super::game_header::build_game_header;
use super::helpers::clear_children;
use super::state::SharedState;
pub fn display_game(game: &Game, state: &SharedState) {
    let _span = tracing::info_span!("display_game", db_id = game.db_id).entered();
    let (content_box, content_scroll, grid_header, is_same_game, gen) = {
        let s = state.borrow();
        let is_same = s.displayed_db_id == game.db_id;
        let gen = s.view_generation + 1;
        (
            s.content_box.clone(),
            s.content_scroll.clone(),
            s.grid_header.clone(),
            is_same,
            gen,
        )
    };
    {
        let mut s = state.borrow_mut();
        s.displayed_db_id = game.db_id;
        s.displayed_variant_id = game.variant_id;
        s.displayed_content_dirty = false;
        s.view_generation = gen;
    }

    clear_children(&grid_header);
    content_scroll.set_child(Some(&content_box));
    if !is_same_game {
        content_scroll.vadjustment().set_value(0.0);
    }

    clear_children(&content_box);

    let fraction = if game.total_count > 0 {
        game.earned_count as f64 / game.total_count as f64
    } else {
        0.0
    };

    let content_width = content_scroll.width().max(600);

    let header_widget = build_game_header(game, fraction, state, content_width);
    content_box.append(&header_widget);

    if game.app_id.is_empty() {
        let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        box_.set_margin_top(32);
        box_.set_margin_bottom(32);
        box_.set_halign(gtk4::Align::Center);
        let label = gtk4::Label::new(Some(&crate::tr!(
            "This game isn't linked to a trophy source yet.\nUse \"Match unmatched games\" in the menu to find a match."
        )));
        label.add_css_class(CSS_DIM_LABEL);
        label.set_wrap(true);
        label.set_justify(gtk4::Justification::Center);
        box_.append(&label);
        content_box.append(&box_);
        return;
    }

    let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spacer.set_margin_top(12);
    content_box.append(&spacer);

    let game_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    game_vbox.set_margin_start(16);
    game_vbox.set_margin_end(16);

    let has_achievements = !game.achievements.is_empty();

    if has_achievements {
        game_vbox.append(&build_achievements_view(game, state, gen));
    }

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(860);
    clamp.set_tightening_threshold(860);
    clamp.set_margin_start(16);
    clamp.set_margin_end(16);
    clamp.set_child(Some(&game_vbox));

    content_box.append(&clamp);
}

pub fn display_game_cached(game: &Game, state: &SharedState) {
    let (content_scroll, content_box, reuse) = {
        let s = state.borrow();
        (
            s.content_scroll.clone(),
            s.content_box.clone(),
            !s.content_unloaded
                && s.displayed_db_id == game.db_id
                && s.displayed_variant_id == game.variant_id
                && !s.displayed_content_dirty
                && content_has_children(&s.content_box),
        )
    };
    if reuse {
        content_scroll.set_child(Some(&content_box));
    } else {
        display_game(game, state);
    }
}

fn content_has_children(content_box: &gtk4::Box) -> bool {
    content_box.first_child().is_some()
}

pub fn format_playtime(hours: f64) -> String {
    let seconds = (hours * 3600.0).round() as i64;
    super::helpers::format_duration(seconds)
}

pub(crate) fn format_last_played(ts: i64) -> String {
    if ts == 0 {
        return crate::tr!("Never");
    }
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%b %-d").to_string())
        .unwrap_or_else(|| crate::tr!("Never"))
}

/// Scale a logo to fit inside a box that is `logo_pct`% of the hero width and
/// height, preserving aspect ratio. Returns the rendered size (min 32px).
pub(crate) fn logo_scaled_dims(
    hero_w: f64,
    hero_h: f64,
    src_w: f64,
    src_h: f64,
    logo_pct: i32,
) -> (f64, f64) {
    let pct = logo_pct as f64 / 100.0;
    let max_h = hero_h * pct;
    let max_w = hero_w * pct;
    let scale = (max_w / src_w).min(max_h / src_h);
    let w = (src_w * scale).max(32.0);
    let h = (src_h * scale).max(32.0);
    (w, h)
}

pub(crate) fn logo_position_align(pos: &str) -> (gtk4::Align, gtk4::Align) {
    match ira_models::LogoPosition::from_string(pos) {
        ira_models::LogoPosition::BottomCenter => (gtk4::Align::Center, gtk4::Align::End),
        ira_models::LogoPosition::BottomRight => (gtk4::Align::End, gtk4::Align::End),
        ira_models::LogoPosition::CenterLeft => (gtk4::Align::Start, gtk4::Align::Center),
        ira_models::LogoPosition::Center => (gtk4::Align::Center, gtk4::Align::Center),
        ira_models::LogoPosition::CenterRight => (gtk4::Align::End, gtk4::Align::Center),
        ira_models::LogoPosition::TopLeft => (gtk4::Align::Start, gtk4::Align::Start),
        ira_models::LogoPosition::TopCenter => (gtk4::Align::Center, gtk4::Align::Start),
        ira_models::LogoPosition::TopRight => (gtk4::Align::End, gtk4::Align::Start),
        _ => (gtk4::Align::Start, gtk4::Align::End),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logo_scaled_dims_wide_logo_at_half_percent() {
        // 3.1:1 hero, 3.4:1 logo, pct=50 → width binds, logo is 50% of hero width.
        let (lw, _lh) = logo_scaled_dims(1920.0, 620.0, 1024.0, 300.0, 50);
        assert!((lw - 960.0).abs() < 1.0);
    }

    #[test]
    fn test_logo_scaled_dims_tall_logo_bounded_by_height() {
        // Square logo on a wide hero → height binds, logo is 50% of hero height.
        let (_lw, lh) = logo_scaled_dims(1920.0, 620.0, 300.0, 300.0, 50);
        assert!((lh - 310.0).abs() < 1.0);
    }

    #[test]
    fn test_logo_scaled_dims_never_below_min_size() {
        let (lw, lh) = logo_scaled_dims(1920.0, 620.0, 2000.0, 40.0, 5);
        assert!(lw >= 32.0);
        assert!(lh >= 32.0);
    }
}
