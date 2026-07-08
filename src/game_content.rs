use crate::db::GameEntry;
use crate::game_header::build_game_header;
use crate::parser::{load_game, Game, MergedAchievement};
use crate::state::{ImageLoadBudget, SharedState, EAGER_IMAGE_BUDGET, SAVE_DIR};
use crate::strings as S;
use adw::prelude::*;
use gtk4::glib;
use gtk4::pango;

pub fn display_game(game: &Game, state: &SharedState) {
    let content_box = state.borrow().content_box.clone();
    let content_scroll = state.borrow().content_scroll.clone();

    content_scroll.vadjustment().set_value(0.0);

    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    crate::images::clear_texture_cache();

    let fraction = if game.total_count > 0 {
        game.earned_count as f64 / game.total_count as f64
    } else {
        0.0
    };

    let content_width = content_scroll.allocated_width().max(600);
    content_box.append(&build_game_header(game, fraction, state, content_width));

    if game.app_id.is_empty() {
        let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
        box_.set_margin_top(32);
        box_.set_margin_bottom(32);
        box_.set_halign(gtk4::Align::Center);
        let label = gtk4::Label::new(Some(
            "This game isn't linked to a trophy source yet.\nUse \"Match unmatched games\" in the menu to find a match.",
        ));
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
        let view_stack = adw::ViewStack::new();

        let view_switcher = adw::ViewSwitcher::new();
        view_switcher.set_stack(Some(&view_stack));
        view_switcher.set_halign(gtk4::Align::Center);
        view_switcher.set_margin_top(12);
        view_switcher.set_margin_bottom(12);
        game_vbox.append(&view_switcher);

        let switcher_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        switcher_spacer.set_margin_bottom(12);
        game_vbox.append(&switcher_spacer);

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
            let entry = GameEntry {
                id: db_id_for_reload,
                kind: kind_for_reload.clone(),
                steam_id: app_id_for_reload.clone(),
                platform_id: platform_id_for_reload.clone(),
                title: String::new(),
                lutris_db_id: if lutris_id_for_reload != 0 {
                    Some(lutris_id_for_reload)
                } else {
                    None
                },
                sgdb_id: None,
                hidden: false,
                logo_position: String::new(),
                logo_size: 0,
                ignored: Some(0),
                manual_unmatch: Some(0),
            };
            if let Ok(updated) = load_game(&entry, SAVE_DIR) {
                crate::ui::apply_game_update(&state_for_reload, updated);
            }
        };

        let mut budget = ImageLoadBudget::new(EAGER_IMAGE_BUDGET);

        if !earned.is_empty() {
            let earned_group = adw::PreferencesGroup::new();
            earned_group.set_title(&format!("Earned  ·  {}", earned.len()));
            for ach in &earned {
                earned_group.add(&create_achievement_row(ach, None, &mut budget));
            }
            progress_vbox.append(&earned_group);
        }

        if !locked.is_empty() || !hidden.is_empty() {
            let locked_group = adw::PreferencesGroup::new();
            locked_group.set_title(&format!("Locked  ·  {}", locked.len() + hidden.len()));
            for ach in &locked {
                let ach_clone = (*ach).clone();
                let reload_clone = reload.clone();
                let kind_clone = game.kind.clone();
                let app_id_clone = game.app_id.clone();
                let platform_id_clone = game.platform_id.clone();
                let state_clone = state.clone();
                locked_group.add(&create_achievement_row(
                    ach,
                    Some(Box::new(move || {
                        crate::dialogs::confirm_mark_unlocked(
                            &state_clone,
                            &kind_clone,
                            &app_id_clone,
                            &platform_id_clone,
                            &ach_clone,
                            reload_clone.clone(),
                        );
                    })),
                    &mut budget,
                ));
            }
            if !hidden.is_empty() {
                let hidden_row = adw::ActionRow::new();
                hidden_row.set_title(&format!("... and {} hidden trophies", hidden.len()));
                hidden_row.set_subtitle("Earn them to reveal details");
                hidden_row.set_sensitive(false);
                locked_group.add(&hidden_row);
            }
            progress_vbox.append(&locked_group);
        }
        budget.flush();

        let global_built = std::cell::Cell::new(false);
        let app_id_for_global = game.app_id.clone();
        let state_for_global = state.clone();
        let global_vbox_weak = global_vbox.downgrade();
        view_stack.connect_notify_local(Some("visible-child-name"), move |stack, _| {
            if stack.visible_child_name() == Some("global".into()) && !global_built.get() {
                global_built.set(true);
                if let Some(global_vbox) = global_vbox_weak.upgrade() {
                    let s = state_for_global.borrow();
                    if let Some(game) = s.games.iter().find(|g| g.app_id == app_id_for_global) {
                        build_global_tab(game, &global_vbox);
                    }
                }
            }
        });

        let progress_page =
            view_stack.add_titled(&progress_vbox, Some("progress"), S::MY_PROGRESS);
        progress_page.set_icon_name(Some("user-home-symbolic"));

        let global_page =
            view_stack.add_titled(&global_vbox, Some("global"), S::GLOBAL_STATS);
        global_page.set_icon_name(Some("dialog-information-symbolic"));

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

fn build_global_tab(game: &Game, global_vbox: &gtk4::Box) {
    let mut all_ach: Vec<MergedAchievement> = game.achievements.clone();
    all_ach.sort_by(|a, b| {
        b.global_percent
            .partial_cmp(&a.global_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let global_group = adw::PreferencesGroup::new();
    global_group.set_title(S::GLOBAL_UNLOCK_RATES);
    global_group.set_margin_bottom(24);

    let mut budget = ImageLoadBudget::new(EAGER_IMAGE_BUDGET);

    let first_batch = 30.min(all_ach.len());
    for ach in &all_ach[..first_batch] {
        add_global_row(&global_group, ach, &mut budget);
    }
    global_vbox.append(&global_group);
    budget.flush();

    if all_ach.len() > first_batch {
        let remaining = all_ach[first_batch..].to_vec();
        let group = global_group.clone();
        let mut i = 0;
        glib::idle_add_local(move || {
            let end = (i + 20).min(remaining.len());
            let mut batch_budget = ImageLoadBudget::new(0);
            for ach in &remaining[i..end] {
                add_global_row(&group, ach, &mut batch_budget);
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

fn add_global_row(
    group: &adw::PreferencesGroup,
    ach: &MergedAchievement,
    budget: &mut ImageLoadBudget,
) {
    let (row, reveal) = create_global_stats_row(ach, budget);
    group.add(&row);
    if let Some(reveal) = reveal {
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            reveal();
        });
        row.add_controller(click);
    }
}

fn achievement_icon(ach: &MergedAchievement, budget: &mut ImageLoadBudget) -> gtk4::Image {
    let img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
    img.set_pixel_size(48);
    img.set_valign(gtk4::Align::Start);
    if ach.earned {
        if !ach.icon_path.is_empty() {
            budget.load(&img, &ach.icon_path);
        } else {
            img.set_icon_name(Some("starred-symbolic"));
        }
    } else if !ach.icon_gray_path.is_empty() {
        budget.load(&img, &ach.icon_gray_path);
    }
    img
}

fn create_achievement_row(
    ach: &MergedAchievement,
    on_mark_unlocked: Option<Box<dyn Fn()>>,
    budget: &mut ImageLoadBudget,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let img = achievement_icon(ach, budget);
    content.append(&img);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_hexpand(true);

    let title = gtk4::Label::new(Some(&ach.display_name));
    title.set_xalign(0.0);
    title.set_valign(gtk4::Align::Start);
    vbox.append(&title);

    let desc = gtk4::Label::new(Some(&ach.description));
    desc.set_xalign(0.0);
    desc.set_valign(gtk4::Align::Start);
    desc.set_wrap(true);
    desc.set_wrap_mode(pango::WrapMode::WordChar);
    desc.add_css_class("dim-label");
    desc.add_css_class("caption");
    vbox.append(&desc);

    content.append(&vbox);

    if ach.earned {
        let time_label = gtk4::Label::new(Some(""));
        time_label.set_justify(gtk4::Justification::Right);
        time_label.set_valign(gtk4::Align::Start);
        time_label.add_css_class("dim-label");
        time_label.add_css_class("caption");
        if ach.earned_time > 0 {
            let t = chrono::DateTime::from_timestamp(ach.earned_time, 0)
                .map(|dt| {
                    dt.format("%b %e, %Y @ %l:%M %p")
                        .to_string()
                        .replace("  ", " ")
                })
                .unwrap_or_else(|| "Unknown".to_string());
            time_label.set_text(&t);
        } else {
            time_label.set_text("Marked manually");
        }
        content.append(&time_label);
    }

    row.set_child(Some(&content));

    if let Some(on_mark) = on_mark_unlocked {
        let click = gtk4::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |_, _, _, _| {
            on_mark();
        });
        row.add_controller(click);
        row.set_tooltip_text(Some(S::RIGHT_CLICK_TO_MARK));
    }

    row
}

fn create_global_stats_row(
    ach: &MergedAchievement,
    budget: &mut ImageLoadBudget,
) -> (gtk4::ListBoxRow, Option<Box<dyn Fn() + 'static>>) {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let grid = gtk4::Grid::new();

    let progress = gtk4::ProgressBar::new();
    progress.set_fraction(ach.global_percent / 100.0);
    progress.set_valign(gtk4::Align::Fill);
    progress.set_halign(gtk4::Align::Fill);
    progress.set_hexpand(true);
    progress.set_vexpand(true);
    progress.set_opacity(0.18);
    progress.add_css_class("global-bar");

    let is_hidden_spoiler = ach.hidden && !ach.earned;

    let content_stack = gtk4::Stack::new();
    content_stack.set_hexpand(true);
    content_stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
    content_stack.set_transition_duration(350);

    let make_content = |img: gtk4::Image, title: &str, desc: &str| -> gtk4::Box {
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&img);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_valign(gtk4::Align::Start);
        vbox.set_hexpand(true);
        let title_label = gtk4::Label::new(Some(title));
        title_label.set_xalign(0.0);
        vbox.append(&title_label);
        let desc_label = gtk4::Label::new(Some(desc));
        desc_label.set_wrap(true);
        desc_label.set_wrap_mode(pango::WrapMode::WordChar);
        desc_label.set_xalign(0.0);
        desc_label.add_css_class("dim-label");
        desc_label.add_css_class("caption");
        vbox.append(&desc_label);
        content.append(&vbox);

        let pct_label =
            gtk4::Label::new(Some(&format!("{:.1}%", ach.global_percent)));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class("heading");
        content.append(&pct_label);
        content
    };

    let pct_str = format!("{:.1}%", ach.global_percent);

    let reveal = if is_hidden_spoiler {
        let spoiler_img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
        spoiler_img.set_pixel_size(48);
        spoiler_img.set_valign(gtk4::Align::Start);
        if !ach.icon_gray_path.is_empty() {
            budget.load(&spoiler_img, &ach.icon_gray_path);
        }

        let spoiler_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        spoiler_content.set_margin_top(8);
        spoiler_content.set_margin_bottom(8);
        spoiler_content.set_margin_start(12);
        spoiler_content.set_margin_end(12);
        spoiler_content.append(&spoiler_img);

        let vbox_spoiler = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox_spoiler.set_valign(gtk4::Align::Start);
        vbox_spoiler.set_hexpand(true);
        let title_spoiler = gtk4::Label::new(Some(S::HIDDEN_ACHIEVEMENT));
        title_spoiler.set_xalign(0.0);
        vbox_spoiler.append(&title_spoiler);
        let desc_spoiler = gtk4::Label::new(Some(S::CLICK_TO_REVEAL));
        desc_spoiler.set_xalign(0.0);
        desc_spoiler.add_css_class("dim-label");
        desc_spoiler.add_css_class("caption");
        vbox_spoiler.append(&desc_spoiler);
        spoiler_content.append(&vbox_spoiler);

        let pct_label = gtk4::Label::new(Some(&pct_str));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class("heading");
        spoiler_content.append(&pct_label);

        content_stack.add_named(&spoiler_content, Some("spoiler"));

        let real_img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
        real_img.set_pixel_size(48);
        real_img.set_valign(gtk4::Align::Start);
        if !ach.icon_gray_path.is_empty() {
            budget.load(&real_img, &ach.icon_gray_path);
        }

        let real_content =
            make_content(real_img, &ach.display_name, &ach.description);
        content_stack.add_named(&real_content, Some("real"));

        content_stack.set_visible_child_name("spoiler");
        row.set_selectable(true);
        row.set_activatable(true);

        grid.attach(&progress, 0, 0, 1, 1);
        grid.attach(&content_stack, 0, 0, 1, 1);
        row.set_child(Some(&grid));

        let stack_weak = content_stack.downgrade();
        let row_weak = row.downgrade();
        Some(Box::new(move || {
            let Some(row) = row_weak.upgrade() else { return };
            let Some(stack) = stack_weak.upgrade() else { return };
            stack.set_visible_child_name("real");
            row.set_activatable(false);
            row.set_selectable(false);
        }) as Box<dyn Fn()>)
    } else {
        let img = achievement_icon(ach, budget);

        let content = make_content(img, &ach.display_name, &ach.description);
        content_stack.add_named(&content, Some("real"));
        content_stack.set_visible_child_name("real");

        grid.attach(&progress, 0, 0, 1, 1);
        grid.attach(&content_stack, 0, 0, 1, 1);
        row.set_child(Some(&grid));

        None
    };

    (row, reveal)
}
