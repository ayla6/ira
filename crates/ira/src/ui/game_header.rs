use crate::Game;
use gtk4::prelude::*;

use super::css::*;
use super::game_display::{
    format_last_played, format_playtime, logo_position_align, logo_scaled_dims,
};
use super::play_button::play_button;
use super::state::SharedState;

pub(super) fn build_game_header(
    game: &Game,
    fraction: f64,
    state: &SharedState,
    content_width: i32,
) -> gtk4::Widget {
    let _span = tracing::info_span!("build_game_header", db_id = game.db_id).entered();
    let title_row = build_header_title(game);
    let stats_row = build_header_stats(game, fraction, state);
    let wine_enabled = check_wine_enabled(game, state);
    let wine_btn = build_wine_tools_button(state, game.db_id, wine_enabled);
    let settings_btn = build_settings_button(state, game.db_id);

    let has_hero = !game.hero_image_path.is_empty();
    let has_logo = !game.logo_path.is_empty();
    let has_grid = !game.grid_path.is_empty();

    if !has_hero && !has_logo && !has_grid {
        let header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        header.set_margin_top(24);
        header.set_margin_bottom(8);
        header.set_margin_start(24);
        header.set_margin_end(24);
        header.append(&title_row);
        let stats_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        stats_wrapper.append(&stats_row);
        let btn_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        if let Some(ref wb) = wine_btn {
            btn_group.append(wb);
        }
        btn_group.append(&settings_btn);
        stats_wrapper.append(&btn_group);
        header.append(&stats_wrapper);
        return header.upcast();
    }

    let hero_path = if has_hero {
        game.hero_image_path.clone()
    } else {
        String::new()
    };

    let overlay = build_hero_overlay(game, content_width, &hero_path, has_hero, has_logo);
    let stats_container = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    stats_container.set_margin_start(24);
    stats_container.set_margin_end(24);
    stats_container.set_margin_top(12);
    stats_container.set_margin_bottom(12);
    stats_container.append(&stats_row);
    let btn_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    if let Some(ref wb) = wine_btn {
        btn_group.append(wb);
    }
    btn_group.append(&settings_btn);
    stats_container.append(&btn_group);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&overlay);
    outer.append(&stats_container);
    outer.upcast()
}

fn build_header_title(game: &Game) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    let title_label = gtk4::Label::new(Some(&game.name));
    title_label.set_xalign(0.0);
    title_label.add_css_class(CSS_TITLE_1);
    row.append(&title_label);
    row
}

fn build_header_stats(game: &Game, fraction: f64, state: &SharedState) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    row.set_valign(gtk4::Align::Center);
    row.set_hexpand(true);
    row.append(&play_button(state, game.db_id, game.variant_id));
    let history_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    history_box.set_valign(gtk4::Align::Center);
    history_box.append(&stat_label(
        "Last played",
        &format_last_played(game.last_played),
    ));
    let pt_box = stat_label("Play time", &format_playtime(game.playtime));
    history_box.append(&pt_box);
    history_box.add_css_class(CSS_CLICKABLE_STAT);
    history_box.set_tooltip_text(Some("View play history"));
    let sc = state.clone();
    let db_id = game.db_id;
    let variant_id = game.variant_id;
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        super::play_history::show_play_history_dialog(&sc, db_id, variant_id);
    });
    history_box.add_controller(click);
    row.append(&history_box);
    if game.total_count > 0 {
        let tbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        tbox.set_valign(gtk4::Align::Center);
        let cap = gtk4::Label::new(Some("Trophies"));
        cap.set_xalign(0.0);
        cap.add_css_class(CSS_DIM_LABEL);
        cap.add_css_class(CSS_CAPTION);
        tbox.append(&cap);
        let trow = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        trow.set_valign(gtk4::Align::Center);
        let val = gtk4::Label::new(Some(&format!("{}/{}", game.earned_count, game.total_count)));
        val.set_xalign(0.0);
        val.set_valign(gtk4::Align::Center);
        val.add_css_class(CSS_HEADING);
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
}

fn check_wine_enabled(game: &Game, state: &SharedState) -> bool {
    if game.kind != ira_models::GameKind::Wine {
        return false;
    }
    let s = state.borrow();
    let config = ira_db::get_game_config(&s.db, game.db_id).ok().flatten();
    let app_default = s.cfg.default_wine_config.clone();
    let (_, mut wine, profile_id) = config.unwrap_or_default();
    if let Some(pid) = profile_id {
        if let Ok(Some(profile)) = ira_db::get_profile(&s.db, pid) {
            wine.version = profile.wine_version;
            wine.custom_wine_path = profile.custom_wine_path;
            wine.prefix = profile.prefix;
            wine.arch = profile.arch;
            wine.umu_enabled = profile.umu_enabled;
        }
    }
    wine = wine.merge_with_default(&app_default);
    wine.enabled
}

fn build_wine_tools_button(
    state: &SharedState,
    db_id: i64,
    wine_enabled: bool,
) -> Option<gtk4::Widget> {
    if !wine_enabled {
        return None;
    }

    let menu = gio::Menu::new();
    menu.append(Some("Winetricks"), Some("wine.winetricks"));
    menu.append(Some("Wine task manager"), Some("wine.taskmgr"));
    menu.append(Some("Wine control panel"), Some("wine.control"));
    menu.append(Some("Wine registry"), Some("wine.regedit"));
    menu.append(Some("Wine configuration"), Some("wine.winecfg"));
    menu.append(Some("Open Wine console"), Some("wine.console"));
    menu.append(Some("Open Bash terminal"), Some("wine.bash"));
    menu.append(Some("Run EXE inside Wine prefix"), Some("wine.run_exe"));

    let btn = gtk4::MenuButton::new();
    btn.set_icon_name("wine-glasses-symbolic");
    btn.add_css_class(CSS_FLAT);
    btn.set_valign(gtk4::Align::Center);
    btn.set_tooltip_text(Some("Wine tools"));
    btn.set_menu_model(Some(&menu));

    let actions = gio::SimpleActionGroup::new();

    add_wine_winetricks_action(&actions, state, db_id);
    add_wine_cmd_action(&actions, state, db_id, "taskmgr", "taskmgr");
    add_wine_cmd_action(&actions, state, db_id, "control", "control");
    add_wine_cmd_action(&actions, state, db_id, "regedit", "regedit");
    add_wine_cmd_action(&actions, state, db_id, "winecfg", "winecfg");
    add_wine_cmd_action(&actions, state, db_id, "console", "wineconsole");
    add_wine_bash_action(&actions, state, db_id);
    add_wine_run_exe_action(&actions, state, db_id);

    btn.insert_action_group("wine", Some(&actions));
    Some(btn.upcast())
}

fn add_wine_cmd_action(
    actions: &gio::SimpleActionGroup,
    state: &SharedState,
    db_id: i64,
    action_name: &'static str,
    cmd_arg: &'static str,
) {
    let st = state.clone();
    let action = gio::SimpleAction::new(action_name, None);
    action.connect_activate(move |_, _| {
        let (wine_exe, prefix, env) = get_wine_cmd_env(&st, db_id);
        if let Some(exe) = wine_exe {
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg(cmd_arg).env("WINEPREFIX", &prefix);
            for (k, v) in &env {
                cmd.env(k, v);
            }
            let _ = cmd.spawn();
        }
    });
    actions.add_action(&action);
}

fn add_wine_winetricks_action(actions: &gio::SimpleActionGroup, state: &SharedState, db_id: i64) {
    let st = state.clone();
    let action = gio::SimpleAction::new("winetricks", None);
    action.connect_activate(move |_, _| {
        let (wine_exe, prefix, env) = get_wine_cmd_env(&st, db_id);
        if let Some(exe) = wine_exe {
            let mut cmd = std::process::Command::new("winetricks");
            cmd.arg("--gui");
            cmd.env("WINEPREFIX", &prefix);
            if let Some(dir) = std::path::Path::new(&exe).parent() {
                let path = std::env::var("PATH").unwrap_or_default();
                cmd.env("PATH", format!("{}:{}", dir.display(), path));
            }
            for (k, v) in &env {
                cmd.env(k, v);
            }
            let _ = cmd.spawn();
        }
    });
    actions.add_action(&action);
}

fn add_wine_bash_action(actions: &gio::SimpleActionGroup, state: &SharedState, db_id: i64) {
    let st = state.clone();
    let action = gio::SimpleAction::new("bash", None);
    action.connect_activate(move |_, _| {
        let (wine_exe, prefix, env) = get_wine_cmd_env(&st, db_id);
        let mut full_env = env;
        full_env.push(("WINEPREFIX".to_string(), prefix));
        if let Some(exe) = wine_exe {
            if let Some(dir) = std::path::Path::new(&exe).parent() {
                let path = std::env::var("PATH").unwrap_or_default();
                full_env.push(("PATH".to_string(), format!("{}:{}", dir.display(), path)));
            }
        }
        super::helpers::spawn_terminal(&full_env);
    });
    actions.add_action(&action);
}

fn add_wine_run_exe_action(actions: &gio::SimpleActionGroup, state: &SharedState, db_id: i64) {
    let st = state.clone();
    let action = gio::SimpleAction::new("run_exe", None);
    action.connect_activate(move |_, _| {
        let (wine_exe, prefix, env) = get_wine_cmd_env(&st, db_id);
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select EXE to run in Wine prefix");
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.exe");
        filter.add_pattern("*.msi");
        dialog.set_default_filter(Some(&filter));
        dialog.open(
            None::<&adw::ApplicationWindow>,
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        if let Some(ref exe) = wine_exe {
                            let mut cmd = std::process::Command::new(exe);
                            cmd.arg(&path).env("WINEPREFIX", &prefix);
                            for (k, v) in &env {
                                cmd.env(k, v);
                            }
                            let _ = cmd.spawn();
                        }
                    }
                }
            },
        );
    });
    actions.add_action(&action);
}

fn build_settings_button(state: &SharedState, db_id: i64) -> gtk4::Widget {
    let menu = gio::Menu::new();
    menu.append(Some("View log"), Some("game.view_log"));

    let btn = adw::SplitButton::new();
    btn.set_icon_name("cogged-wheel-big-symbolic");
    btn.add_css_class(CSS_FLAT);
    btn.set_valign(gtk4::Align::Center);
    btn.set_tooltip_text(Some("Settings"));
    btn.set_menu_model(Some(&menu));

    let st = state.clone();
    btn.connect_clicked(move |_| {
        super::edit_game_dialog::show_edit_game_dialog(&st, db_id);
    });

    let actions = gio::SimpleActionGroup::new();
    let st_log = state.clone();
    let log_action = gio::SimpleAction::new("view_log", None);
    log_action.connect_activate(move |_, _| {
        super::log_viewer::show_log_dialog(&st_log, db_id);
    });
    actions.add_action(&log_action);
    btn.insert_action_group("game", Some(&actions));

    btn.upcast::<gtk4::Widget>()
}

fn build_hero_overlay(
    game: &Game,
    content_width: i32,
    hero_path: &str,
    has_hero: bool,
    has_logo: bool,
) -> gtk4::Overlay {
    let overlay = gtk4::Overlay::new();
    overlay.set_vexpand(false);
    overlay.set_hexpand(true);
    overlay.set_height_request(((content_width as f64) / 3.1).max(150.0) as i32);

    let hero = gtk4::Picture::new();
    if !hero_path.is_empty() {
        if let Some(t) = ira_images::texture_for(hero_path) {
            hero.set_paintable(Some(&t));
        }
    }
    hero.set_halign(gtk4::Align::Fill);
    hero.set_valign(gtk4::Align::Fill);
    hero.set_hexpand(true);
    hero.set_content_fit(gtk4::ContentFit::Cover);
    if !has_hero {
        hero.add_css_class("hero-fallback-bg");
    }
    overlay.set_child(Some(&hero));

    if has_logo && !game.logo_path.is_empty() {
        let logo_pct = game.logo_size.clamp(5, 100);
        let logo_pos = game.logo_position.clone();

        if let Some(pixbuf) = ira_images::pixbuf_for(&game.logo_path) {
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

                let (lw, lh) = logo_scaled_dims(w, h, pb_w, pb_h, logo_pct);

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
    } else if !has_logo {
        let title_label = gtk4::Label::new(Some(&game.name));
        title_label.set_xalign(0.0);
        title_label.set_valign(gtk4::Align::Center);
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_margin_start(24);
        title_label.set_margin_end(24);
        title_label.add_css_class("hero-title-overlay");
        overlay.add_overlay(&title_label);
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

    overlay
}

fn stat_label(caption: &str, value: &str) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_valign(gtk4::Align::Center);
    vbox.set_size_request(110, -1);
    let cap = gtk4::Label::new(Some(caption));
    cap.set_xalign(0.0);
    cap.add_css_class(CSS_DIM_LABEL);
    cap.add_css_class(CSS_CAPTION);
    vbox.append(&cap);
    let val = gtk4::Label::new(Some(value));
    val.set_xalign(0.0);
    val.add_css_class(CSS_HEADING);
    vbox.append(&val);
    vbox
}

fn get_wine_cmd_env(
    state: &SharedState,
    db_id: i64,
) -> (Option<String>, String, Vec<(String, String)>) {
    let s = state.borrow();
    let config = ira_db::get_game_config(&s.db, db_id).ok().flatten();
    let app_default = s.cfg.default_wine_config.clone();
    let (_, mut wine, profile_id) = config.unwrap_or_default();
    if let Some(pid) = profile_id {
        if let Ok(Some(profile)) = ira_db::get_profile(&s.db, pid) {
            wine.version = profile.wine_version;
            wine.custom_wine_path = profile.custom_wine_path;
            wine.prefix = profile.prefix;
            wine.arch = profile.arch;
            wine.umu_enabled = profile.umu_enabled;
        }
    }
    wine = wine.merge_with_default(&app_default);
    let prefix = ira_launcher::wine_launch::wine_prefix(&wine);
    let exe =
        ira_launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path).ok();
    let env = ira_launcher::wine_launch::build_wine_env(&wine, exe.as_deref().unwrap_or(""));
    (exe, prefix, env)
}
