use crate::Game;
use gtk4::prelude::*;

use super::achievement_view::build_achievements_view;
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

/// The Steam client library hero reserves a fixed inset around the logo area
/// (26px left/right, 16px top/bottom in its `BoxSizerContainer`), and the
/// logo percentage sizes the logo relative to that inset region — never the
/// raw hero — so a 100% logo always fits inside the hero.
pub(crate) const LOGO_MARGIN_X: f64 = 26.0;
pub(crate) const LOGO_MARGIN_Y: f64 = 16.0;

/// Size of the margin-inset logo region inside a hero of the given size.
fn logo_region_size(hero_w: f64, hero_h: f64) -> (f64, f64) {
    (
        (hero_w - 2.0 * LOGO_MARGIN_X).max(1.0),
        (hero_h - 2.0 * LOGO_MARGIN_Y).max(1.0),
    )
}

/// Scale a logo to fit inside a box that is `logo_pct`% of the hero's
/// margin-inset logo region, preserving aspect ratio. Returns the rendered
/// size.
pub(crate) fn logo_scaled_dims(
    hero_w: f64,
    hero_h: f64,
    src_w: f64,
    src_h: f64,
    logo_pct: i32,
) -> (f64, f64) {
    let pct = logo_pct as f64 / 100.0;
    let (region_w, region_h) = logo_region_size(hero_w, hero_h);
    let max_h = region_h * pct;
    let max_w = region_w * pct;
    let scale = (max_w / src_w).min(max_h / src_h);
    (src_w * scale, src_h * scale)
}

/// Top-left corner and size of the drawn logo: the scaled logo sits flush
/// against the pinned edge of the margin-inset region, like the Steam
/// client's per-pin `object-position` anchoring.
pub(crate) fn logo_rect(
    hero_w: f64,
    hero_h: f64,
    src_w: f64,
    src_h: f64,
    logo_pct: i32,
    halign: gtk4::Align,
    valign: gtk4::Align,
) -> (f64, f64, f64, f64) {
    let (lw, lh) = logo_scaled_dims(hero_w, hero_h, src_w, src_h, logo_pct);
    let (region_w, region_h) = logo_region_size(hero_w, hero_h);
    let x = match halign {
        gtk4::Align::Start => LOGO_MARGIN_X,
        gtk4::Align::End => hero_w - LOGO_MARGIN_X - lw,
        _ => LOGO_MARGIN_X + (region_w - lw) / 2.0,
    };
    let y = match valign {
        gtk4::Align::Start => LOGO_MARGIN_Y,
        gtk4::Align::End => hero_h - LOGO_MARGIN_Y - lh,
        _ => LOGO_MARGIN_Y + (region_h - lh) / 2.0,
    };
    (x, y, lw, lh)
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
    // 1920×620 hero → region 1868×588; 1024×300 logo at 50% → box 934×294,
    // width binds, logo is half the region wide.
    let (lw, lh) = logo_scaled_dims(1920.0, 620.0, 1024.0, 300.0, 50);
    assert!((lw - 934.0).abs() < 1.0);
    assert!((lh - 273.6).abs() < 0.1);
}

#[test]
fn test_logo_scaled_dims_tall_logo_bounded_by_height() {
    // Square logo on a wide hero → height binds, logo is half the region
    // height (588 / 2 = 294).
    let (lw, lh) = logo_scaled_dims(1920.0, 620.0, 300.0, 300.0, 50);
    assert!((lw - 294.0).abs() < 1.0);
    assert!((lh - 294.0).abs() < 1.0);
}

#[test]
fn test_logo_scaled_dims_full_percent_fills_region_exactly() {
    // 100% must fill the inset region, never the raw hero — this is the
    // overflow regression: the logo must not eat into the margins.
    let (lw, lh) = logo_scaled_dims(1920.0, 620.0, 4000.0, 1200.0, 100);
    assert!(lw <= 1920.0 - 2.0 * LOGO_MARGIN_X + 0.001);
    assert!(lh <= 620.0 - 2.0 * LOGO_MARGIN_Y + 0.001);
}

#[test]
fn test_logo_scaled_dims_stays_inside_region_at_min_percent() {
    // Extreme aspect at 5%: both axes stay inside the percentage box.
    let region_w = 1920.0 - 2.0 * LOGO_MARGIN_X;
    let region_h = 620.0 - 2.0 * LOGO_MARGIN_Y;
    let (lw, lh) = logo_scaled_dims(1920.0, 620.0, 2000.0, 40.0, 5);
    assert!(lw <= region_w * 0.05 + 0.001);
    assert!(lh <= region_h * 0.05 + 0.001);
}

#[test]
fn test_logo_rect_bottom_left_flush_with_inset_region() {
    // Default position at 100%: flush against the inset region's bottom-left
    // corner, never crossing into the margins.
    let (x, y, lw, lh) = logo_rect(
        1920.0,
        620.0,
        1024.0,
        300.0,
        100,
        gtk4::Align::Start,
        gtk4::Align::End,
    );
    assert!((x - LOGO_MARGIN_X).abs() < 0.001);
    assert!((x + lw - (1920.0 - LOGO_MARGIN_X)).abs() < 0.001);
    assert!((y + lh - (620.0 - LOGO_MARGIN_Y)).abs() < 0.001);
}

#[test]
fn test_logo_rect_center_stays_centered() {
    let (x, y, lw, lh) = logo_rect(
        1920.0,
        620.0,
        300.0,
        300.0,
        50,
        gtk4::Align::Center,
        gtk4::Align::Center,
    );
    assert!((x + lw / 2.0 - 960.0).abs() < 1.0);
    assert!((y + lh / 2.0 - 310.0).abs() < 1.0);
}
}
