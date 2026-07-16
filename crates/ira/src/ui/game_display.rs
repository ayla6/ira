use gtk4::prelude::*;
use crate::Game;

use super::state::SharedState;
use super::helpers::clear_children;
use super::game_header::build_game_header;
use super::achievement_view::build_achievements_view;

pub fn display_game(game: &Game, state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let content_scroll = state.borrow().content_scroll.clone();
    let grid_header = state.borrow().grid_header.clone();

    state.borrow_mut().view_generation += 1;
    let gen = state.borrow().view_generation;

    clear_children(&grid_header);
    content_scroll.set_child(Some(&content_box));
    content_scroll.vadjustment().set_value(0.0);

    clear_children(&content_box);

    let fraction = if game.total_count > 0 {
        game.earned_count as f64 / game.total_count as f64
    } else {
        0.0
    };

    let content_width = content_scroll.width().max(600);
    content_box.append(&build_game_header(game, fraction, state, content_width));

    if game.app_id.is_empty() {
        let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        box_.set_margin_top(32);
        box_.set_margin_bottom(32);
        box_.set_halign(gtk4::Align::Center);
        let label = gtk4::Label::new(Some("This game isn't linked to a trophy source yet.\nUse \"Match unmatched games\" in the menu to find a match."));
        label.add_css_class("dim-label");
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

pub fn format_playtime(hours: f64) -> String {
    let seconds = (hours * 3600.0).round() as i64;
    super::helpers::format_duration(seconds)
}

pub(crate) fn format_last_played(ts: i64) -> String {
    if ts == 0 {
        return "Never".to_string();
    }
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%b %-d").to_string())
        .unwrap_or_else(|| "Never".to_string())
}

pub(crate) fn logo_scaled_dims(hero_w: f64, hero_h: f64, src_w: f64, src_h: f64, logo_pct: i32) -> (f64, f64) {
    let max_h = hero_h * (logo_pct as f64 / 100.0);
    let max_w = hero_w * (logo_pct as f64 / 200.0);
    let scale = (max_w / src_w).min(max_h / src_h);
    let w = (src_w * scale).max(32.0);
    let h = (src_h * scale).max(32.0);
    (w, h)
}

pub(crate) fn logo_position_align(pos: &str) -> (gtk4::Align, gtk4::Align) {
    match pos {
        "bottom-center" => (gtk4::Align::Center, gtk4::Align::End),
        "bottom-right" => (gtk4::Align::End, gtk4::Align::End),
        "center-left" => (gtk4::Align::Start, gtk4::Align::Center),
        "center" => (gtk4::Align::Center, gtk4::Align::Center),
        "center-right" => (gtk4::Align::End, gtk4::Align::Center),
        "top-left" => (gtk4::Align::Start, gtk4::Align::Start),
        "top-center" => (gtk4::Align::Center, gtk4::Align::Start),
        "top-right" => (gtk4::Align::End, gtk4::Align::Start),
        _ => (gtk4::Align::Start, gtk4::Align::End),
    }
}
