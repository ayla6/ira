use gtk4::prelude::*;
use adw::prelude::*;
use crate::GameEntry;
use crate::Game;
use crate::MergedAchievement;
use crate::game_loader::load_game;
use crate::strings as S;
use std::cell::Cell;

use super::state::SharedState;
use super::image_budget::ImageLoadBudget;
use super::achievement_rows::{create_achievement_row, build_global_tab};

pub(super) fn build_achievements_view(game: &Game, state: &SharedState, gen: u32) -> gtk4::Widget {
    let _span = tracing::info_span!("build_achievements_view", db_id = game.db_id).entered();
    let is_ps4 = game.kind == ira_models::GameKind::Ps4;

    let view_stack = adw::ViewStack::new();
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    if !is_ps4 {
        let view_switcher = adw::ViewSwitcher::new();
        view_switcher.set_stack(Some(&view_stack));
        view_switcher.set_halign(gtk4::Align::Center);
        view_switcher.set_margin_top(12);
        view_switcher.set_margin_bottom(12);
        outer.append(&view_switcher);
        view_stack.set_margin_top(12);
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
    locked.sort_by(|a, b| {
        trophy_rank(a.trophy_type).cmp(&trophy_rank(b.trophy_type))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });

    fn trophy_rank(t: char) -> u8 {
        match t { 'B' => 0, 'S' => 1, 'G' => 2, 'P' => 3, _ => 4 }
    }

    let app_id_for_reload = game.app_id.clone();
    let kind_for_reload = game.kind;
    let trophy_source_for_reload = game.trophy_source;
    let platform_id_for_reload = game.platform_id.clone();
    let db_id_for_reload = game.db_id;
    let is_retro_or_ps4 = kind_for_reload == ira_models::GameKind::Ps4 || kind_for_reload == ira_models::GameKind::Retro;
    let can_mark_unlocked = matches!(kind_for_reload, ira_models::GameKind::Wine | ira_models::GameKind::Linux);

    let sender = state.borrow().sender.clone();
    let save_dir = state.borrow().save_dir.clone();
    let reload = move || {
        let (steam_id, game_id): (&str, &str) = if is_retro_or_ps4 { ("", &app_id_for_reload) } else { (&app_id_for_reload, "") };
        let entry = GameEntry::for_reload(db_id_for_reload, kind_for_reload, trophy_source_for_reload, steam_id, game_id, &platform_id_for_reload);
        let sender = sender.clone();
        let save_dir = save_dir.clone();
        std::thread::spawn(move || {
            if let Ok(updated) = load_game(&entry, &save_dir) {
                let _ = sender.send(crate::AppMessage::EnrichedGame(updated));
            }
        });
    };

    const FIRST_BATCH: usize = 8;
    const BATCH_SIZE: usize = 20;
    let mut budget = ImageLoadBudget::new(8);

    if !earned.is_empty() {
        let earned_group = adw::PreferencesGroup::new();
        earned_group.set_title(&format!("Earned  ·  {}", earned.len()));
        progress_vbox.append(&earned_group);

        let first_n = FIRST_BATCH.min(earned.len());
        for ach in &earned[..first_n] {
            earned_group.add(&create_achievement_row(ach, None, &mut budget));
        }

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
                batch_budget.flush(&state_gen, gen);
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
        progress_vbox.append(&locked_group);

        let first_n = FIRST_BATCH.min(locked.len());
        for ach in &locked[..first_n] {
            let on_mark = if can_mark_unlocked {
                let ach_clone = (*ach).clone();
                let reload_clone = reload.clone();
                let trophy_source_clone = game.trophy_source;
                let app_id_clone = game.app_id.clone();
                let platform_id_clone = game.platform_id.clone();
                let state_clone = state.clone();
                Some(Box::new(move || {
                    super::matching::confirm_mark_unlocked(&state_clone, trophy_source_clone, &app_id_clone, &platform_id_clone, &ach_clone, reload_clone.clone());
                }) as Box<dyn Fn()>)
            } else {
                None
            };
            locked_group.add(&create_achievement_row(ach, on_mark, &mut budget));
        }

        let hidden_expander: Option<adw::ExpanderRow> = if !hidden.is_empty() {
            let expander = adw::ExpanderRow::new();
            expander.set_title(&format!("… and {} hidden trophies", hidden.len()));

            for ach in hidden.iter() {
                let ach_row = adw::ActionRow::new();
                ach_row.set_title(&super::helpers::esc(&ach.display_name));
                ach_row.set_subtitle(&super::helpers::esc(&ach.description));
                ach_row.set_activatable(true);

                let img = gtk4::Image::from_icon_name("changes-prevent-symbolic");
                img.set_pixel_size(24);
                img.set_valign(gtk4::Align::Center);
                if ach.earned {
                    if !ach.icon_path.is_empty() {
                        ira_images::set_image(&img, &ach.icon_path);
                    }
                } else if !ach.icon_gray_path.is_empty() {
                    ira_images::set_image(&img, &ach.icon_gray_path);
                }
                ach_row.add_prefix(&img);

                if can_mark_unlocked {
                    let ach_clone = (*ach).clone();
                    let reload_inner = reload.clone();
                    let trophy_source_inner = game.trophy_source;
                    let app_id_inner = game.app_id.clone();
                    let platform_id_inner = game.platform_id.clone();
                    let state_inner = state.clone();
                    let mclick = gtk4::GestureClick::new();
                    mclick.set_button(3);
                    mclick.connect_pressed(move |_, _, _, _| {
                        super::matching::confirm_mark_unlocked(&state_inner, trophy_source_inner, &app_id_inner, &platform_id_inner, &ach_clone, reload_inner.clone());
                    });
                    ach_row.add_controller(mclick);
                }

                expander.add_row(&ach_row);
            }

            Some(expander)
        } else {
            None
        };

        if locked.len() > first_n {
            let remaining: Vec<MergedAchievement> =
                locked[first_n..].iter().map(|a| (*a).clone()).collect();
            let group = locked_group.clone();
            let reload = reload.clone();
            let trophy_source = game.trophy_source;
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
                    let on_mark = if can_mark_unlocked {
                        let ach_clone = ach.clone();
                        let reload_clone = reload.clone();
                        let trophy_source_clone = trophy_source;
                        let app_id_clone = app_id.clone();
                        let platform_id_clone = platform_id.clone();
                        let state_clone = state.clone();
                        Some(Box::new(move || {
                            super::matching::confirm_mark_unlocked(&state_clone, trophy_source_clone, &app_id_clone, &platform_id_clone, &ach_clone, reload_clone.clone());
                        }) as Box<dyn Fn()>)
                    } else {
                        None
                    };
                    group.add(&create_achievement_row(ach, on_mark, &mut batch_budget));
                }
                batch_budget.flush(&state, gen);
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
    budget.flush(state, gen);

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
    outer.append(&view_stack);
    outer.upcast()
}
