use gtk4::prelude::*;
use adw::prelude::*;
use crate::GameEntry;
use crate::Game;
use crate::MergedAchievement;
use crate::parser::load_game;
use crate::strings as S;
use std::cell::Cell;

use super::state::{SharedState, SAVE_DIR};
use super::image_budget::ImageLoadBudget;
use super::play_button::play_button;
use super::message_handler::apply_game_update;
use super::achievement_rows::{create_achievement_row, build_global_tab};
use super::helpers::clear_children;

pub fn display_game(game: &Game, state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let content_scroll = state.borrow().content_scroll.clone();

    state.borrow_mut().view_generation += 1;
    let gen = state.borrow().view_generation;

    content_scroll.vadjustment().set_value(0.0);

    clear_children(&content_box);
    crate::images::clear_texture_cache();

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
        let is_ps4 = game.kind == "ps4";

        let view_stack = adw::ViewStack::new();

        if !is_ps4 {
            let view_switcher = adw::ViewSwitcher::new();
            view_switcher.set_stack(Some(&view_stack));
            view_switcher.set_halign(gtk4::Align::Center);
            view_switcher.set_margin_top(12);
            view_switcher.set_margin_bottom(12);
            game_vbox.append(&view_switcher);

            let switcher_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            switcher_spacer.set_margin_bottom(12);
            game_vbox.append(&switcher_spacer);
        }

        let progress_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        let global_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

        let mut earned: Vec<&MergedAchievement> = Vec::new();
        let mut locked: Vec<&MergedAchievement> = Vec::new();
        let mut hidden: Vec<&MergedAchievement> = Vec::new();
        for ach in &game.achievements {
            if ach.earned {
                earned.push(ach);
            } else if ach.hidden {
                hidden.push(ach);
            } else {
                locked.push(ach);
            }
        }

        earned.sort_by(|a, b| b.earned_time.cmp(&a.earned_time));
        locked.sort_by(|a, b| a.display_name.cmp(&b.display_name));

        let app_id_for_reload = game.app_id.clone();
        let kind_for_reload = game.kind.clone();
        let platform_id_for_reload = game.platform_id.clone();
        let db_id_for_reload = game.db_id;
        let lutris_id_for_reload = game.lutris_id;
        let state_for_reload = state.clone();
        let reload = move || {
            let entry = GameEntry::for_reload(db_id_for_reload, &kind_for_reload, &app_id_for_reload, &platform_id_for_reload, lutris_id_for_reload);
            if let Ok(updated) = load_game(&entry, SAVE_DIR) {
                apply_game_update(&state_for_reload, updated);
            }
        };

        let mut budget = ImageLoadBudget::new(18);
        const FIRST_BATCH: usize = 30;
        const BATCH_SIZE: usize = 20;

        if !earned.is_empty() {
            let earned_group = adw::PreferencesGroup::new();
            earned_group.set_title(&format!("Earned  ·  {}", earned.len()));

            let first_n = FIRST_BATCH.min(earned.len());
            for ach in &earned[..first_n] {
                earned_group.add(&create_achievement_row(ach, None, &mut budget));
            }
            progress_vbox.append(&earned_group);

            if earned.len() > first_n {
                let remaining: Vec<MergedAchievement> =
                    earned[first_n..].iter().map(|a| (*a).clone()).collect();
                let group = earned_group.clone();
                let state_gen = state.clone();
                let mut i = 0;
                glib::idle_add_local(move || {
                    if state_gen.borrow().view_generation != gen {
                        return glib::ControlFlow::Break;
                    }
                    let end = (i + BATCH_SIZE).min(remaining.len());
                    let mut batch_budget = ImageLoadBudget::new(0);
                    for ach in &remaining[i..end] {
                        group.add(&create_achievement_row(ach, None, &mut batch_budget));
                    }
                    batch_budget.flush();
                    i = end;
                    if i >= remaining.len() {
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });
            }
        }

        if !locked.is_empty() || !hidden.is_empty() {
            let locked_group = adw::PreferencesGroup::new();
            locked_group.set_title(&format!("Locked  ·  {}", locked.len() + hidden.len()));

            let first_n = FIRST_BATCH.min(locked.len());
            for ach in &locked[..first_n] {
                let ach_clone = (*ach).clone();
                let reload_clone = reload.clone();
                let kind_clone = game.kind.clone();
                let app_id_clone = game.app_id.clone();
                let platform_id_clone = game.platform_id.clone();
                let state_clone = state.clone();
                locked_group.add(&create_achievement_row(
                    ach,
                    Some(Box::new(move || {
                        super::matching::confirm_mark_unlocked(&state_clone, &kind_clone, &app_id_clone, &platform_id_clone, &ach_clone, reload_clone.clone());
                    })),
                    &mut budget,
                ));
            }

            let hidden_expander: Option<adw::ExpanderRow> = if !hidden.is_empty() {
                let expander = adw::ExpanderRow::new();
                expander.set_title(&format!("… and {} hidden trophies", hidden.len()));

                for ach in hidden.iter() {
                    let ach_clone = (*ach).clone();
                    let reload_inner = reload.clone();
                    let kind_inner = game.kind.clone();
                    let app_id_inner = game.app_id.clone();
                    let platform_id_inner = game.platform_id.clone();
                    let state_inner = state.clone();

                    let ach_row = adw::ActionRow::new();
                    ach_row.set_title(&ach.display_name);
                    ach_row.set_subtitle(&ach.description);
                    ach_row.set_activatable(true);

                    let img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
                    img.set_pixel_size(24);
                    img.set_valign(gtk4::Align::Center);
                    if ach.earned {
                        if !ach.icon_path.is_empty() {
                            crate::images::set_image(&img, &ach.icon_path);
                        }
                    } else if !ach.icon_gray_path.is_empty() {
                        crate::images::set_image(&img, &ach.icon_gray_path);
                    }
                    ach_row.add_prefix(&img);

                    let mclick = gtk4::GestureClick::new();
                    mclick.set_button(3);
                    mclick.connect_pressed(move |_, _, _, _| {
                        super::matching::confirm_mark_unlocked(&state_inner, &kind_inner, &app_id_inner, &platform_id_inner, &ach_clone, reload_inner.clone());
                    });
                    ach_row.add_controller(mclick);

                    expander.add_row(&ach_row);
                }

                Some(expander)
            } else {
                None
            };
            progress_vbox.append(&locked_group);

            if locked.len() > first_n {
                let remaining: Vec<MergedAchievement> =
                    locked[first_n..].iter().map(|a| (*a).clone()).collect();
                let group = locked_group.clone();
                let reload = reload.clone();
                let kind = game.kind.clone();
                let app_id = game.app_id.clone();
                let platform_id = game.platform_id.clone();
                let state = state.clone();
                let mut expander = hidden_expander.clone();
                let mut i = 0;
                glib::idle_add_local(move || {
                    if state.borrow().view_generation != gen {
                        return glib::ControlFlow::Break;
                    }
                    let end = (i + BATCH_SIZE).min(remaining.len());
                    let mut batch_budget = ImageLoadBudget::new(0);
                    for ach in &remaining[i..end] {
                        let ach_clone = ach.clone();
                        let reload_clone = reload.clone();
                        let kind_clone = kind.clone();
                        let app_id_clone = app_id.clone();
                        let platform_id_clone = platform_id.clone();
                        let state_clone = state.clone();
                        group.add(&create_achievement_row(
                            ach,
                            Some(Box::new(move || {
                                super::matching::confirm_mark_unlocked(&state_clone, &kind_clone, &app_id_clone, &platform_id_clone, &ach_clone, reload_clone.clone());
                            })),
                            &mut batch_budget,
                        ));
                    }
                    batch_budget.flush();
                    i = end;
                    if i >= remaining.len() {
                        if let Some(exp) = expander.take() {
                            group.add(&exp);
                        }
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });
            } else if let Some(exp) = hidden_expander {
                locked_group.add(&exp);
            }
        }
        budget.flush();

        let progress_page = view_stack.add_titled(&progress_vbox, Some("progress"), S::MY_PROGRESS);
        progress_page.set_icon_name(Some("user-home-symbolic"));

        if !is_ps4 {
            let global_built = Cell::new(false);
            let app_id_for_global = game.app_id.clone();
            let state_for_global = state.clone();
            let gen_for_global = gen;
            let global_vbox_weak = global_vbox.downgrade();
            view_stack.connect_notify_local(Some("visible-child-name"), move |stack, _| {
                if stack.visible_child_name() == Some("global".into()) && !global_built.get() {
                    global_built.set(true);
                    if let Some(global_vbox) = global_vbox_weak.upgrade() {
                        let s = state_for_global.borrow();
                        if s.view_generation == gen_for_global {
                            if let Some(game) = s.games.iter().find(|g| g.app_id == app_id_for_global) {
                                build_global_tab(game, &global_vbox, &state_for_global, gen_for_global);
                            }
                        }
                    }
                }
            });

            let global_page = view_stack.add_titled(&global_vbox, Some("global"), S::GLOBAL_STATS);
            global_page.set_icon_name(Some("dialog-information-symbolic"));
        }

        view_stack.set_vhomogeneous(false);
        view_stack.set_margin_bottom(32);

        game_vbox.append(&view_stack);
    }

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(860);
    clamp.set_tightening_threshold(860);
    clamp.set_margin_start(16);
    clamp.set_margin_end(16);
    clamp.set_child(Some(&game_vbox));

    content_box.append(&clamp);
}

fn build_game_header(game: &Game, fraction: f64, state: &SharedState, content_width: i32) -> gtk4::Widget {
    let title_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-1");
        row.append(&title_label);
        row
    };

    let stats_row = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        row.set_valign(gtk4::Align::Center);
        row.append(&play_button(state, game.lutris_id));
        row.append(&stat_label("Last played", &format_lastplayed(game.lastplayed)));
        row.append(&stat_label("Play time", &format_playtime(game.playtime)));
        if game.total_count > 0 {
            let tbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            tbox.set_valign(gtk4::Align::Center);
            let cap = gtk4::Label::new(Some("Trophies"));
            cap.set_xalign(0.0);
            cap.add_css_class("dim-label");
            cap.add_css_class("caption");
            tbox.append(&cap);
            let trow = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            trow.set_valign(gtk4::Align::Center);
            let val = gtk4::Label::new(Some(&format!("{}/{}", game.earned_count, game.total_count)));
            val.set_xalign(0.0);
            val.set_valign(gtk4::Align::Center);
            val.add_css_class("heading");
            trow.append(&val);
            let prog = gtk4::ProgressBar::new();
            prog.set_fraction(fraction);
            prog.set_valign(gtk4::Align::Center);
            prog.set_size_request(120, -1);
            trow.append(&prog);
            tbox.append(&trow);
            row.append(&tbox);
        }
        row
    };

    let has_hero = !game.hero_image_path.is_empty();
    if !has_hero {
        let header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        header.set_margin_top(24);
        header.set_margin_bottom(8);
        header.set_margin_start(24);
        header.set_margin_end(24);
        header.append(&title_row);
        header.append(&stats_row);
        return header.upcast();
    }

    let overlay = gtk4::Overlay::new();
    overlay.set_vexpand(false);
    overlay.set_hexpand(true);
    overlay.set_height_request(((content_width as f64) / 3.1).max(150.0) as i32);

    let hero = gtk4::Picture::new();
    if let Some(t) = crate::images::texture_for(&game.hero_image_path) {
        hero.set_paintable(Some(&t));
    }
    hero.set_halign(gtk4::Align::Fill);
    hero.set_valign(gtk4::Align::Fill);
    hero.set_hexpand(true);
    hero.set_content_fit(gtk4::ContentFit::Cover);
    overlay.set_child(Some(&hero));

    if !game.logo_path.is_empty() {
        let logo_pct = game.logo_size.clamp(5, 100);
        let logo_pos = game.logo_position.clone();

        if let Ok(pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path) {
            let pb_w = pixbuf.width() as f64;
            let pb_h = pixbuf.height() as f64;

            let logo_area = gtk4::DrawingArea::new();
            logo_area.set_halign(gtk4::Align::Fill);
            logo_area.set_valign(gtk4::Align::Fill);
            logo_area.set_hexpand(true);
            logo_area.set_vexpand(true);

            logo_area.set_draw_func(move |_area, cr, area_w, area_h| {
                let w = area_w as f64;
                let h = area_h as f64;
                if w <= 0.0 || h <= 0.0 {
                    return;
                }

                let (sw, sh) = logo_scaled_dims(w, h, pb_w, pb_h, logo_pct);
                let lw = sw as f64;
                let lh = sh as f64;

                let (halign, valign) = logo_position_align(&logo_pos);

                let x = match halign {
                    gtk4::Align::Start => 24.0,
                    gtk4::Align::Center => (w - lw) / 2.0,
                    gtk4::Align::End => w - lw - 24.0,
                    _ => 24.0,
                };
                let y = match valign {
                    gtk4::Align::Start => 24.0,
                    gtk4::Align::Center => (h - lh) / 2.0,
                    gtk4::Align::End => h - lh - 24.0,
                    _ => h - lh - 24.0,
                };

                let _ = cr.save();
                cr.translate(x, y);
                cr.scale(lw / pb_w, lh / pb_h);
                cr.set_source_pixbuf(&pixbuf, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            });

            overlay.add_overlay(&logo_area);
        }
    }

    {
        let overlay_weak = overlay.downgrade();
        let size_monitor = gtk4::DrawingArea::new();
        size_monitor.set_halign(gtk4::Align::Fill);
        size_monitor.set_valign(gtk4::Align::Fill);
        size_monitor.set_hexpand(true);
        size_monitor.set_vexpand(true);
        size_monitor.set_draw_func(move |_area, _cr, w, _h| {
            if w > 0 {
                if let Some(overlay) = overlay_weak.upgrade() {
                    let target = ((w as f64) / 3.1).max(150.0) as i32;
                    if overlay.height_request() != target {
                        overlay.set_height_request(target);
                    }
                }
            }
        });
        overlay.add_overlay(&size_monitor);
    }

    let stats_container = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    stats_container.set_margin_start(24);
    stats_container.set_margin_end(24);
    stats_container.set_margin_top(12);
    stats_container.set_margin_bottom(12);
    stats_container.append(&stats_row);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&overlay);
    outer.append(&stats_container);
    outer.upcast()
}

fn stat_label(caption: &str, value: &str) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_size_request(110, -1);
    let cap = gtk4::Label::new(Some(caption));
    cap.set_xalign(0.0);
    cap.add_css_class("dim-label");
    cap.add_css_class("caption");
    vbox.append(&cap);
    let val = gtk4::Label::new(Some(value));
    val.set_xalign(0.0);
    val.add_css_class("heading");
    vbox.append(&val);
    vbox
}

pub fn format_playtime(hours: f64) -> String {
    let total = (hours * 60.0).round() as u64;
    let h = total / 60;
    let m = total % 60;
    match (h, m) {
        (0, 0) => "0min".to_string(),
        (0, m) => format!("{}min", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h{:02}min", h, m),
    }
}

fn format_lastplayed(ts: i64) -> String {
    if ts == 0 {
        return "Never".to_string();
    }
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%b %-d").to_string())
        .unwrap_or_else(|| "Never".to_string())
}

pub(crate) fn logo_scaled_dims(hero_w: f64, hero_h: f64, src_w: f64, src_h: f64, logo_pct: i32) -> (i32, i32) {
    let max_h = hero_h * (logo_pct as f64 / 100.0);
    let max_w = hero_w * (logo_pct as f64 / 200.0);
    let scale = (max_w / src_w).min(max_h / src_h);
    let w = (src_w * scale).max(32.0) as i32;
    let h = (src_h * scale).max(32.0) as i32;
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
