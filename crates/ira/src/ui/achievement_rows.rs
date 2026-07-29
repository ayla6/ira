use gtk4::prelude::*;
use adw::prelude::*;
use crate::MergedAchievement;
use crate::strings as S;
use super::image_budget::ImageLoadBudget;
use super::state::SharedState;
use super::css::*;

pub const FIRST_BATCH: usize = 8;
pub const BATCH_SIZE: usize = 20;

pub fn build_global_tab(game: &crate::Game, global_vbox: &gtk4::Box, state: &SharedState, gen: u32) {
    let mut all_ach: Vec<MergedAchievement> = game.achievements.clone();
    all_ach.sort_by(|a, b| b.global_percent.partial_cmp(&a.global_percent).unwrap_or(std::cmp::Ordering::Equal));

    let global_group = adw::PreferencesGroup::new();
    global_group.set_title(S::GLOBAL_UNLOCK_RATES);
    global_group.set_margin_bottom(24);

    let mut budget = ImageLoadBudget::new(FIRST_BATCH);

    let first_n = FIRST_BATCH.min(all_ach.len());
    for ach in &all_ach[..first_n] {
        add_global_row(&global_group, ach, &mut budget);
    }
    global_vbox.append(&global_group);
    budget.flush(state, gen);

    if all_ach.len() > first_n {
        let remaining = all_ach[first_n..].to_vec();
        let group = global_group.clone();
        let state = state.clone();
        let mut i = 0;
        glib::idle_add_local(move || {
            if state.borrow().view_generation != gen {
                return glib::ControlFlow::Break;
            }
            let end = (i + BATCH_SIZE).min(remaining.len());
            let mut batch_budget = ImageLoadBudget::new(0);
            for ach in &remaining[i..end] {
                add_global_row(&group, ach, &mut batch_budget);
            }
            batch_budget.flush(&state, gen);
            i = end;
            if i >= remaining.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

pub fn add_global_row(group: &adw::PreferencesGroup, ach: &MergedAchievement, budget: &mut ImageLoadBudget) {
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
        if ach.trophy_type != '\0' {
            img.add_css_class(CSS_LOCKED_TROPHY);
        }
    }
    img
}

pub fn create_achievement_row(
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
    desc.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    desc.add_css_class(CSS_DIM_LABEL);
    desc.add_css_class(CSS_CAPTION);
    vbox.append(&desc);

    content.append(&vbox);

    if ach.earned {
        let time_label = gtk4::Label::new(Some(""));
        time_label.set_justify(gtk4::Justification::Right);
        time_label.set_valign(gtk4::Align::Start);
        time_label.add_css_class(CSS_DIM_LABEL);
        time_label.add_css_class(CSS_CAPTION);
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

pub fn create_global_stats_row(
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
    progress.add_css_class(CSS_GLOBAL_BAR);

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
        desc_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        desc_label.set_xalign(0.0);
        desc_label.add_css_class(CSS_DIM_LABEL);
        desc_label.add_css_class(CSS_CAPTION);
        vbox.append(&desc_label);
        content.append(&vbox);

        let pct_label = gtk4::Label::new(Some(&format!("{:.1}%", ach.global_percent)));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class(CSS_HEADING);
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
            if ach.trophy_type != '\0' {
                spoiler_img.add_css_class(CSS_LOCKED_TROPHY);
            }
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
        desc_spoiler.add_css_class(CSS_DIM_LABEL);
        desc_spoiler.add_css_class(CSS_CAPTION);
        vbox_spoiler.append(&desc_spoiler);
        spoiler_content.append(&vbox_spoiler);

        let pct_label = gtk4::Label::new(Some(&pct_str));
        pct_label.set_valign(gtk4::Align::Start);
        pct_label.add_css_class(CSS_HEADING);
        spoiler_content.append(&pct_label);

        content_stack.add_named(&spoiler_content, Some("spoiler"));

        let real_img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
        real_img.set_pixel_size(48);
        real_img.set_valign(gtk4::Align::Start);
        if !ach.icon_gray_path.is_empty() {
            budget.load(&real_img, &ach.icon_gray_path);
            if ach.trophy_type != '\0' {
                real_img.add_css_class(CSS_LOCKED_TROPHY);
            }
        }

        let real_content = make_content(real_img, &ach.display_name, &ach.description);
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
