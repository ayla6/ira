use crate::db::{DbConn, GameEntry};
use crate::game_content::display_game;
use crate::parser::{load_game, set_achievement_earned, Game, MergedAchievement};
use crate::sidebar::rebuild_sidebar;
use crate::state::{select_row_silently, SharedState, SAVE_DIR};
use crate::steam::SteamClient;
use crate::strings as S;
use crate::watcher::AchievementWatcher;
use crate::AppMessage;
use adw::prelude::*;
use gtk4::glib;

use std::sync::mpsc::Sender;
use std::sync::Arc;

fn open_folder(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn confirm_dialog(
    parent: &adw::ApplicationWindow,
    title: &str,
    body: &str,
    confirm_label: &str,
    appearance: adw::ResponseAppearance,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance("confirm", appearance);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "confirm" {
            on_confirm();
        }
    });
    dialog.present(Some(parent));
}

pub fn show_settings_dialog(state: &SharedState) {
    let parent = state.borrow().window.clone();
    let cfg = state.borrow().cfg.clone();
    let steam = state.borrow().steam.clone();
    let dialog = adw::Window::new();
    dialog.set_title(Some(S::SETTINGS));
    dialog.set_default_size(450, 360);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(&parent));

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let group = adw::PreferencesGroup::new();
    group.set_title(S::API_KEYS);
    group.set_margin_top(16);
    group.set_margin_bottom(16);
    group.set_margin_start(16);
    group.set_margin_end(16);

    let steam_entry = adw::EntryRow::new();
    steam_entry.set_title(S::STEAM_WEB_API_KEY);
    steam_entry.set_text(&cfg.steam_api_key);
    steam_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&steam_entry);

    let sgdb_entry = adw::EntryRow::new();
    sgdb_entry.set_title(S::STEAMGRIDDB_KEY);
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    sgdb_entry.set_input_purpose(gtk4::InputPurpose::Password);
    group.add(&sgdb_entry);

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title(S::LIVE_UPDATES);
    notif_group.set_margin_top(16);
    notif_group.set_margin_start(16);
    notif_group.set_margin_end(16);

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title(S::NOTIFY_ON_UNLOCKS);
    notif_row.set_subtitle(S::NOTIFY_SUBTITLE);
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title(S::CLOSE_TO_BG_TITLE);
    bg_row.set_subtitle(S::CLOSE_TO_BG_SUBTITLE);
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");
    save_btn.set_margin_top(8);
    save_btn.set_margin_bottom(16);
    save_btn.set_margin_start(16);
    save_btn.set_margin_end(16);

    let state_clone = state.clone();
    let dialog_clone = dialog.clone();
    let steam_clone = steam.clone();
    save_btn.connect_clicked(move |_| {
        let mut s = state_clone.borrow_mut();
        s.cfg.steam_api_key = steam_entry.text().to_string();
        s.cfg.steam_griddb_api_key = sgdb_entry.text().to_string();
        s.cfg.notifications_enabled = notif_row.is_active();
        s.cfg.close_to_background = bg_row.is_active();

        steam_clone.update_keys(&s.cfg.steam_api_key, &s.cfg.steam_griddb_api_key);

        let cfg = s.cfg.clone();
        drop(s);

        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        dialog_clone.destroy();
    });

    box_.append(&group);
    box_.append(&notif_group);
    box_.append(&save_btn);
    toolbar_view.set_content(Some(&box_));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}

pub fn show_game_context_menu(state: &SharedState, game: &Game, row: &gtk4::ListBoxRow) {
    let current_hidden = state
        .borrow()
        .games
        .iter()
        .find(|g| g.lutris_id == game.lutris_id)
        .map(|g| g.hidden)
        .unwrap_or(game.hidden);

    let menu = gio::Menu::new();
    menu.append(Some(S::EDIT_GAME_SETTINGS), Some("game.edit"));
    let folders_menu = gio::Menu::new();
    if game.kind == "steam" || game.kind == "sgdb" {
        let subdir = if game.kind == "sgdb" { "steamgriddb" } else { "steam" };
        folders_menu.append(Some("Image data"), Some("game.open_images"));
    }
    if game.kind == "steam" {
        folders_menu.append(Some("Achievement status"), Some("game.open_steam_status"));
    } else if game.kind == "gog" {
        folders_menu.append(Some("Achievement status"), Some("game.open_gog_status"));
    }
    if folders_menu.n_items() > 0 {
        menu.append_submenu(Some("Open folder"), &folders_menu);
    }
    menu.append(Some(if current_hidden { S::UNHIDE_GAME } else { S::HIDE_GAME }), Some("game.hide"));
    menu.append(Some(S::REMOVE_GAME), Some("game.remove"));
    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_halign(gtk4::Align::Start);

    let state_clone = state.clone();
    let game_clone = game.clone();
    let actions = gio::SimpleActionGroup::new();

    let edit_action = gio::SimpleAction::new("edit", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    edit_action.connect_activate(move |_, _| {
        show_game_settings_dialog(&sc, &gc);
    });
    actions.add_action(&edit_action);

    let hide_action = gio::SimpleAction::new("hide", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    let row_clone = row.clone();
    hide_action.connect_activate(move |_, _| {
        let new_hidden = !current_hidden;
        let lutris_id = gc.lutris_id;
        {
            let s = sc.borrow();
            if let Some(g) = s.games.iter().find(|g| g.lutris_id == lutris_id) {
                if g.db_id != 0 {
                    if let Err(e) = crate::db::set_game_hidden(&s.db, g.db_id, new_hidden) {
                        eprintln!("Failed to set hidden: {}", e);
                    }
                } else if lutris_id != 0 {
                    if let Err(e) = crate::db::set_lutris_hidden(&s.db, lutris_id, new_hidden) {
                        eprintln!("Failed to set lutris hidden: {}", e);
                    }
                }
            }
        }
        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.hidden = new_hidden;
        }
        let scroll = sc.borrow().sidebar_scroll.clone();
        let saved_scroll = scroll.vadjustment().value();
        if new_hidden {
            row_clone.add_css_class("hidden-game");
        } else {
            row_clone.remove_css_class("hidden-game");
        }
        let show_hidden = sc.borrow().cfg.show_hidden_games;
        row_clone.set_visible(!new_hidden || show_hidden);
        let adj = scroll.vadjustment();
        let max = (adj.upper() - adj.page_size()).max(0.0);
        adj.set_value(saved_scroll.min(max));
    });
    actions.add_action(&hide_action);

    let remove_action = gio::SimpleAction::new("remove", None);
    let sc = state_clone.clone();
    let gc = game_clone.clone();
    remove_action.connect_activate(move |_, _| {
        let window = sc.borrow().window.clone();
        let sc2 = sc.clone();
        let app_id = gc.app_id.clone();
        let db_id = gc.db_id;
        confirm_dialog(
            &window,
            S::REMOVE_GAME_QUESTION,
            &format!("Remove \u{201C}{}\u{201D} from the database? This won't delete any save files.", gc.name),
            S::REMOVE_GAME,
            adw::ResponseAppearance::Destructive,
            move || {
                if let Err(e) = crate::db::remove_game(&sc2.borrow().db, db_id) {
                    eprintln!("Failed to remove game from DB: {}", e);
                }
                let _ = sc2.borrow().sender.send(AppMessage::GameRemoved { app_id: app_id.clone() });
            },
        );
    });
    actions.add_action(&remove_action);

    if game_clone.kind == "steam" || game_clone.kind == "sgdb" {
        let open_images = gio::SimpleAction::new("open_images", None);
        let gc = game_clone.clone();
        open_images.connect_activate(move |_, _| {
            let subdir = if gc.kind == "sgdb" { "steamgriddb" } else { "steam" };
            let path = format!("{}/data/{}/{}", SAVE_DIR, subdir, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_images);
    }

    if game_clone.kind == "steam" {
        let open_status = gio::SimpleAction::new("open_steam_status", None);
        let gc = game_clone.clone();
        open_status.connect_activate(move |_, _| {
            let path = format!("{}/steam/{}", SAVE_DIR, gc.app_id);
            open_folder(&path);
        });
        actions.add_action(&open_status);
    }

    if game_clone.kind == "gog" {
        let open_gog = gio::SimpleAction::new("open_gog_status", None);
        let gc = game_clone.clone();
        open_gog.connect_activate(move |_, _| {
            let path = format!("{}/gog/{}/{}", SAVE_DIR, crate::parser::GALAXY_ID, gc.platform_id);
            open_folder(&path);
        });
        actions.add_action(&open_gog);
    }

    row.insert_action_group("game", Some(&actions));

    popover.set_parent(row);
    popover.popup();
}

fn show_game_settings_dialog(state: &SharedState, game: &Game) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_default_width(500);
    win.set_default_height(500);
    win.set_transient_for(Some(&parent));
    win.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&game.name))));
    outer.append(&header_bar);

    let notebook = gtk4::Notebook::new();

    let settings_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    settings_page.set_margin_start(16);
    settings_page.set_margin_end(16);
    settings_page.set_margin_top(16);
    settings_page.set_margin_bottom(16);

    let title_entry = gtk4::Entry::new();
    title_entry.set_placeholder_text(Some(S::GAME_TITLE));
    title_entry.set_text(&game.name);
    let title_label = gtk4::Label::new(Some(S::TITLE));
    title_label.set_halign(gtk4::Align::Start);
    settings_page.append(&title_label);
    settings_page.append(&title_entry);

    if game.lutris_id != 0 && !game.app_id.is_empty() {
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        settings_page.append(&sep);

        let unmatch_btn = gtk4::Button::with_label("Unmatch game");
        unmatch_btn.add_css_class("destructive-action");
        unmatch_btn.set_halign(gtk4::Align::Start);
        let sc = state.clone();
        let gc = game.clone();
        let win_clone = win.clone();
        unmatch_btn.connect_clicked(move |_| {
            let parent = sc.borrow().window.clone();
            let sc2 = sc.clone();
            let lutris_id = gc.lutris_id;
            let name = gc.name.clone();
            let win2 = win_clone.clone();
            confirm_dialog(
                &parent,
                "Unmatch game?",
                &format!("Unmatch \u{201C}{}\u{201D} from its trophy source? The game will remain in the list but without trophies.", name),
                "Unmatch",
                adw::ResponseAppearance::Destructive,
                move || {
                    if let Err(e) = crate::db::unmatch_game(&sc2.borrow().db, lutris_id) {
                        eprintln!("Failed to unmatch game: {}", e);
                    }
                    if let Some(g) = sc2.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                        g.app_id.clear();
                        g.kind.clear();
                        g.platform_id.clear();
                        g.achievements.clear();
                        g.earned_count = 0;
                        g.total_count = 0;
                        g.icon_path.clear();
                        g.hero_image_path.clear();
                        g.grid_path.clear();
                        g.header_path.clear();
                        g.logo_path.clear();
                        g.manual_unmatch = true;
                        if !g.lutris_name.is_empty() {
                            g.name = g.lutris_name.clone();
                        }
                    }
                    rebuild_sidebar(&sc2);
                    win2.close();
                },
            );
        });
        settings_page.append(&unmatch_btn);
    }

    let settings_label = gtk4::Label::new(Some("Settings"));
    notebook.append_page(&settings_page, Some(&settings_label));

    let logo_positions = ["bottom-left", "bottom-center", "bottom-right", "center-left", "center", "center-right", "top-left", "top-center", "top-right"];
    let logo_controls = if !game.logo_path.is_empty() {
        let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        logo_page.set_margin_start(16);
        logo_page.set_margin_end(16);
        logo_page.set_margin_top(16);
        logo_page.set_margin_bottom(16);

        let pos_store = gtk4::StringList::new(&logo_positions);
        let pos_combo = gtk4::DropDown::new(Some(pos_store), None::<&gtk4::PropertyExpression>);
        let current_idx = logo_positions.iter().position(|&p| p == game.logo_position).unwrap_or(0);
        pos_combo.set_selected(current_idx as u32);
        let pos_row = adw::ActionRow::new();
        pos_row.set_title("Position");
        pos_row.add_suffix(&pos_combo);
        logo_page.append(&pos_row);

        let size_pct = game.logo_size.clamp(5, 100);
        let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 5.0, 10.0, 0.0);
        let size_spin = gtk4::SpinButton::new(Some(&size_adj), 1.0, 0);
        size_spin.set_numeric(true);
        let size_row = adw::ActionRow::new();
        size_row.set_title("Size (% of hero)");
        size_row.add_suffix(&size_spin);
        logo_page.append(&size_row);

        let logo_label = gtk4::Label::new(Some("Logo"));
        notebook.append_page(&logo_page, Some(&logo_label));
        Some((pos_combo, size_adj))
    } else {
        None
    };

    if !game.app_id.is_empty() {
        let images_page = build_image_manager_content(state, game, &win);
        let images_label = gtk4::Label::new(Some("Images"));
        notebook.append_page(&images_page, Some(&images_label));
    }

    outer.append(&notebook);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(S::CANCEL);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");
    let state_clone = state.clone();
    let db_id = game.db_id;
    let lutris_id = game.lutris_id;
    let app_id = game.app_id.clone();
    let title_entry_c = title_entry.clone();
    let logo_controls_c = logo_controls.clone();
    let win_s = win.clone();
    save_btn.connect_clicked(move |_| {
        let title = title_entry_c.text().to_string();
        if let Err(e) = crate::db::update_game_title(&state_clone.borrow().db, db_id, &title) {
            eprintln!("Failed to update game: {}", e);
        }

        if let Some((ref pos_combo, ref size_adj)) = logo_controls_c {
            let pos = logo_positions[pos_combo.selected() as usize].to_string();
            let size = size_adj.value() as i32;
            if db_id != 0 {
                if let Err(e) = crate::db::set_logo_settings(&state_clone.borrow().db, db_id, &pos, size) {
                    eprintln!("Failed to update logo settings: {}", e);
                }
            }
            if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
                g.logo_position = pos;
                g.logo_size = size;
            }
        }

        if let Some(g) = state_clone.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lutris_id) {
            g.name = title.clone();
        }
        if !app_id.is_empty() {
            state_clone.borrow().game_names.lock().unwrap().insert(app_id.clone(), title);
        }
        rebuild_sidebar(&state_clone);

        let game_list = state_clone.borrow().game_list.clone();
        if let Some(idx) = state_clone.borrow().games.iter().position(|g| g.lutris_id == lutris_id) {
            let row = game_list.row_at_index((idx + 1) as i32);
            select_row_silently(&state_clone, row.as_ref());
        }

        let selected = state_clone.borrow().selected_id.clone();
        if selected == lutris_id.to_string() {
            if let Some(g) = state_clone.borrow().games.iter().find(|g| g.lutris_id == lutris_id).cloned() {
                display_game(&g, &state_clone);
            }
        }

        win_s.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    outer.append(&btn_row);

    win.set_content(Some(&outer));
    win.present();
}

fn build_image_manager_content(state: &SharedState, game: &Game, parent_win: &adw::Window) -> gtk4::Box {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let is_steam = game.kind == "steam" || game.kind == "gog";
    let id = game.app_id.clone();
    let data_subdir = if game.kind == "sgdb" { "steamgriddb" } else { "steam" };

    for (label, file, asset, thumb_w, thumb_h) in [
        ("Icon", "icon.png", "icon", 48, 48),
        ("Hero", "library_hero.jpg", "hero", 96, 32),
        ("Grid", "library_600x900.jpg", "grid", 32, 48),
        ("Header", "header.jpg", "header", 96, 45),
        ("Logo", "logo.png", "logo", 96, 32),
    ] {
        let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.add_css_class("heading");
        section.append(&lbl);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_hexpand(true);

        let img_path = format!("{}/data/{}/{}/{}", SAVE_DIR, data_subdir, id, file);
        let preview = gtk4::Picture::for_filename(&img_path);
        preview.set_content_fit(gtk4::ContentFit::ScaleDown);
        preview.set_size_request(thumb_w, thumb_h);
        let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        preview_wrapper.set_size_request(thumb_w, thumb_h);
        if std::path::Path::new(&img_path).exists() {
            preview_wrapper.append(&preview);
        } else {
            let ph = gtk4::Label::new(Some("\u{2014}"));
            ph.add_css_class("dim-label");
            preview_wrapper.append(&ph);
        }
        row.append(&preview_wrapper);

        let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        btns.set_hexpand(true);
        btns.set_halign(gtk4::Align::End);

        let refresh_images: std::rc::Rc<dyn Fn()> = std::rc::Rc::new({
            let preview_wrapper = preview_wrapper.clone();
            let img_path = img_path.clone();
            let state_clone = state.clone();
            let game_clone = game.clone();
            move || {
                while let Some(child) = preview_wrapper.first_child() {
                    preview_wrapper.remove(&child);
                }
                if std::path::Path::new(&img_path).exists() {
                    let p = gtk4::Picture::for_filename(&img_path);
                    p.set_content_fit(gtk4::ContentFit::ScaleDown);
                    preview_wrapper.append(&p);
                } else {
                    let ph = gtk4::Label::new(Some("\u{2014}"));
                    ph.add_css_class("dim-label");
                    preview_wrapper.append(&ph);
                }
                let s = state_clone.borrow();
                if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&s.db, game_clone.lutris_id) {
                    drop(s);
                    if let Ok(updated) = crate::parser::load_game(&entry, SAVE_DIR) {
                        crate::ui::apply_game_update(&state_clone, updated);
                    }
                }
            }
        });

        let browse_btn = gtk4::Button::with_label("Browse\u{2026}");
        let dest_path = img_path.clone();
        let state_browse = state.clone();
        let refresh = refresh_images.clone();
        browse_btn.connect_clicked(move |_| {
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            filter.add_mime_type("image/x-icon");
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select image");
            dialog.set_default_filter(Some(&filter));
            let dest = dest_path.clone();
            let refresh_c = refresh.clone();
            dialog.open(Some(&state_browse.borrow().window), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let is_ico = path.extension().and_then(|e| e.to_str()) == Some("ico");
                        if is_ico {
                            let ico_dest = std::path::Path::new(&dest).with_extension("ico");
                            if std::fs::copy(&path, &ico_dest).is_ok() {
                                if let Ok(png) = crate::parser::convert_ico_to_png(&ico_dest) {
                                    let _ = std::fs::remove_file(&ico_dest);
                                }
                            }
                        } else if let Err(e) = std::fs::copy(&path, &dest) {
                            eprintln!("Failed to copy image: {}", e);
                        }
                        refresh_c();
                    }
                }
            });
        });
        btns.append(&browse_btn);

        if is_steam && asset != "icon" {
            let btn = gtk4::Button::with_label("Steam");
            let steam = state.borrow().steam.clone();
            let id_c = id.clone();
            let asset_c = asset.to_string();
            let refresh = refresh_images.clone();
            btn.connect_clicked(move |_| {
                let _ = steam.force_download_steam(&id_c, &asset_c);
                refresh();
            });
            btns.append(&btn);
        }

        let btn = gtk4::Button::with_label("SGDB\u{2026}");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let asset_c = asset.to_string();
        let is_steam_c = is_steam;
        let parent = parent_win.clone();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            show_sgdb_picker(&steam, &id_c, &asset_c, is_steam_c, &parent, refresh.clone());
        });
        btns.append(&btn);

        row.append(&btns);
        section.append(&row);
        content.append(&section);
    }

    let note = gtk4::Label::new(Some("Re-select the game after changing images to see updates."));
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    content.append(&note);

    content
}

fn show_sgdb_picker(steam: &Arc<crate::steam::SteamClient>, id: &str, asset: &str, is_steam_id: bool, parent: &adw::Window, on_done: std::rc::Rc<dyn Fn()>) {
    let picker = adw::Window::new();
    picker.set_default_width(500);
    picker.set_default_height(450);
    picker.set_transient_for(Some(parent));
    picker.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&format!("Pick {}", asset)))));
    outer.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(8);
    list.set_margin_bottom(8);

    let loading = gtk4::Label::new(Some("Loading\u{2026}"));
    loading.add_css_class("dim-label");
    list.append(&loading);

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    picker.set_content(Some(&outer));
    picker.present();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<crate::steam::SgdbAsset>>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = steam.clone();
    let id_c = id.to_string();
    let asset_c = asset.to_string();
    std::thread::spawn(move || {
        let results = steam_c.list_sgdb_assets(&id_c, &asset_c, is_steam_id);
        let _ = tx.send(results);
    });

    let list_clone = list.clone();
    let steam_clone = steam.clone();
    let id_clone = id.to_string();
    let asset_clone = asset.to_string();
    let picker_clone = picker.clone();
    let on_done = on_done.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(assets) = rx.borrow_mut().try_recv() {
            while let Some(child) = list_clone.first_child() {
                list_clone.remove(&child);
            }

            if assets.is_empty() {
                let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
                none.add_css_class("dim-label");
                list_clone.append(&none);
                return glib::ControlFlow::Break;
            }

            for a in assets {
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                row.set_margin_top(4);
                row.set_margin_bottom(4);

                let thumb_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                let thumb_pic = gtk4::Picture::new();
                thumb_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
                thumb_pic.set_size_request(48, 48);
                thumb_box.append(&thumb_pic);
                row.append(&thumb_box);

                let url_clone = a.url.clone();
                let thumb_pic_clone = thumb_pic.clone();
                let steam_thumb = steam_clone.clone();
                let thumb_dir = format!("{}/data/.thumbnails", SAVE_DIR);
                let _ = std::fs::create_dir_all(&thumb_dir);
                let thumb_name = format!("{}/{}", thumb_dir, url_clone.rsplit('/').next().unwrap_or("thumb"));
                let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<String>>();
                let rx_thumb = std::cell::RefCell::new(rx_thumb);
                std::thread::spawn(move || {
                    let final_path = if std::path::Path::new(&thumb_name).exists() {
                        Some(thumb_name.clone())
                    } else if steam_thumb.download_file(&url_clone, std::path::Path::new(&thumb_name)).is_ok() {
                        if std::path::Path::new(&thumb_name).extension().and_then(|e| e.to_str()) == Some("ico") {
                            if let Ok(img) = image::open(&thumb_name) {
                                let png_path = std::path::Path::new(&thumb_name).with_extension("png");
                                if img.save(&png_path).is_ok() {
                                    let _ = std::fs::remove_file(&thumb_name);
                                    Some(png_path.to_string_lossy().into_owned())
                                } else {
                                    Some(thumb_name.clone())
                                }
                            } else {
                                Some(thumb_name.clone())
                            }
                        } else {
                            Some(thumb_name.clone())
                        }
                    } else {
                        None
                    };
                    let _ = tx_thumb.send(final_path);
                });
                let tp = thumb_pic_clone.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    if let Ok(path) = rx_thumb.borrow_mut().try_recv() {
                        if let Some(p) = path {
                            tp.set_filename(Some(&p));
                        }
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });

                let mut info = if a.width > 0 && a.height > 0 {
                    format!("{}\u{d7}{}", a.width, a.height)
                } else {
                    String::new()
                };
                if !a.style.is_empty() {
                    if !info.is_empty() {
                        info = format!("{} \u{b7} {}", info, a.style);
                    } else {
                        info = a.style.clone();
                    }
                }
                if !a.author.is_empty() {
                    if !info.is_empty() {
                        info = format!("{} \u{b7} by {}", info, a.author);
                    } else {
                        info = format!("by {}", a.author);
                    }
                }
                let label = gtk4::Label::new(Some(&info));
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                row.append(&label);

                let dl_btn = gtk4::Button::with_label("Download");
                dl_btn.add_css_class("suggested-action");
                let url = a.url.clone();
                let data_subdir = if is_steam_id { "steam" } else { "steamgriddb" };
                let dest_dir = format!("{}/data/{}/{}", SAVE_DIR, data_subdir, id_clone);
                let file_name = match asset_clone.as_str() {
                    "icon" => {
                        let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") {
                            "ico"
                        } else if a.mime.contains("png") {
                            "png"
                        } else if a.mime.contains("jpeg") || a.mime.contains("jpg") {
                            "jpg"
                        } else if a.mime.contains("webp") {
                            "webp"
                        } else {
                            std::path::Path::new(&url).extension().and_then(|e| e.to_str()).unwrap_or("png")
                        };
                        format!("icon.{}", ext)
                    }
                    "hero" => "library_hero.jpg".to_string(),
                    "grid" => "library_600x900.jpg".to_string(),
                    "logo" => "logo.png".to_string(),
                    _ => continue,
                };
                let dest = format!("{}/{}", dest_dir, file_name);
                let steam_c = steam_clone.clone();
                let picker_c = picker_clone.clone();
                let on_done_c = on_done.clone();
                dl_btn.connect_clicked(move |_| {
                    let _ = std::fs::create_dir_all(&dest_dir);
                    for old_ext in ["png", "ico", "jpg", "webp"] {
                        let old = format!("{}/icon.{}", dest_dir, old_ext);
                        let _ = std::fs::remove_file(&old);
                    }
                    if steam_c.download_file(&url, std::path::Path::new(&dest)).is_ok() {
                        if file_name.ends_with(".ico") {
                            match crate::parser::convert_ico_to_png(std::path::Path::new(&dest)) {
                                Ok(png_path) => eprintln!("Converted ICO to {}", png_path.display()),
                                Err(e) => {
                                    eprintln!("ICO conversion failed: {}, trying direct load", e);
                                    let png_dest = std::path::Path::new(&dest).with_extension("png");
                                    let _ = std::fs::rename(&dest, &png_dest);
                                }
                            }
                        }
                        on_done_c();
                        picker_c.close();
                    } else {
                        eprintln!("Download failed for {}", url);
                    }
                });
                row.append(&dl_btn);

                list_clone.append(&row);
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

pub fn show_add_game_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title(S::SELECT_GAME_FOLDER);

    let state_clone = state.clone();
    dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let folder = path.to_string_lossy().into_owned();

        if let Some(app_id) = crate::gamesetup::detect_app_id(&folder) {
            finish_add_game(&state_clone, &folder, &app_id);
        } else if crate::gamesetup::is_gog_game(&folder) {
            if let Some((_info_dir, product_id, game_name)) = crate::gamesetup::find_gog_info(&folder) {
                prompt_for_steam_id_gog(&state_clone, &folder, &product_id, &game_name);
            } else {
                prompt_for_app_id(&state_clone, &folder);
            }
        } else {
            prompt_for_app_id(&state_clone, &folder);
        }
    });
}

fn prompt_for_steam_id(state: &SharedState, title: &str, body: &str, on_add: impl Fn(&str) + 'static) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some(title), Some(body));

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("e.g. 1687950"));
    entry.set_input_purpose(gtk4::InputPurpose::Digits);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    entry.set_margin_start(8);
    entry.set_margin_end(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("add", S::ADD_GAME_BTN);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        on_add(&entry.text());
    });
    dialog.present(Some(&window));
}

fn prompt_for_app_id(state: &SharedState, folder: &str) {
    let folder = folder.to_string();
    let state_clone = state.clone();
    prompt_for_steam_id(state, S::ENTER_STEAM_ID, S::ENTER_STEAM_ID_BODY, move |app_id| {
        finish_add_game(&state_clone, &folder, app_id);
    });
}

fn prompt_for_steam_id_gog(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str) {
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let state_clone = state.clone();
    let body = format!(
        "{}: {}\n{}: {}\n\n{}",
        S::DETECTED_GOG_GAME, game_name,
        S::GOG_PRODUCT_ID, product_id,
        S::ENTER_STEAM_ID_GOG
    );
    prompt_for_steam_id(state, S::ADD_GOG_GAME, &body, move |steam_app_id| {
        finish_add_gog_game(&state_clone, &galaxy_folder, &product_id, &game_name, steam_app_id);
    });
}

fn finalize_added_game(
    app_id: &str,
    kind: &str,
    platform_id: &str,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: Sender<AppMessage>,
    db: DbConn,
) {
    let entry = match crate::db::find_by_steam_id(&db, app_id) {
        Ok(Some(e)) => e,
        _ => {
            eprintln!("Failed to find game in DB after adding: {}", app_id);
            return;
        }
    };
    match load_game(&entry, SAVE_DIR) {
        Ok(game) => {
            if let Some(ref watcher) = watcher {
                watcher.watch(&entry, &game.achievements);
            }
            let name = game.name.clone();
            let _ = sender.send(AppMessage::NewGame(game));
            enrich_game_async(
                app_id.to_string(),
                kind.to_string(),
                platform_id.to_string(),
                entry.id,
                0,
                name,
                steam,
                watcher,
                sender,
            );
        }
        Err(e) => eprintln!("Failed to load newly added game: {}", e),
    }
}

fn finish_add_game(state: &SharedState, folder: &str, app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let folder = folder.to_string();
    let app_id = app_id.to_string();
    std::thread::spawn(move || {
        match crate::gamesetup::add_game_from_folder(&folder, &app_id, &steam, &db, SAVE_DIR) {
            Ok(_) => finalize_added_game(&app_id, "steam", &app_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("Add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

fn finish_add_gog_game(state: &SharedState, galaxy_folder: &str, product_id: &str, game_name: &str, steam_app_id: &str) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    let galaxy_folder = galaxy_folder.to_string();
    let product_id = product_id.to_string();
    let game_name = game_name.to_string();
    let steam_app_id = steam_app_id.to_string();

    std::thread::spawn(move || {
        match crate::gamesetup::add_gog_game_from_folder(
            &galaxy_folder, &product_id, &game_name, &steam_app_id, &steam, &db, SAVE_DIR,
        ) {
            Ok(_) => finalize_added_game(&steam_app_id, "gog", &product_id, steam, watcher, sender, db),
            Err(e) => {
                eprintln!("GOG add game failed: {}", e);
                let _ = sender.send(AppMessage::AddGameError(e));
            }
        }
    });
}

fn match_game_to_steam(state: &SharedState, lutris_id: i64, steam_app_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &steam_app_id, "steam", &steam_app_id) {
            eprintln!("match_game_to_steam: upsert_matching failed: {}", e);
            return;
        }
        if let Err(e) = steam.generate_steam_settings(&steam_app_id) {
            eprintln!("match_game_to_steam: generate_steam_settings failed: {}", e);
        }
        match crate::db::find_by_lutris_id(&db, lutris_id) {
            Ok(Some(entry)) => {
                match crate::parser::load_game(&entry, SAVE_DIR) {
                    Ok(mut game) => {
                        if game.name.is_empty() || game.name.starts_with("App ID:") {
                            game.name = lutris_name.clone();
                        }
                        game.lutris_id = lutris_id;
                        let name = game.name.clone();
                        if let Some(ref watcher) = watcher {
                            watcher.watch(&entry, &game.achievements);
                        }
                        let _ = sender.send(AppMessage::NewGame(game));
                        enrich_game_async(
                            steam_app_id.clone(),
                            "steam".to_string(),
                            steam_app_id.clone(),
                            entry.id,
                            lutris_id,
                            name,
                            steam,
                            watcher,
                            sender,
                        );
                    }
                    Err(e) => eprintln!("match_game_to_steam: load_game failed: {}", e),
                }
            }
            Ok(None) => eprintln!("match_game_to_steam: find_by_lutris_id returned None for lutris_id={}", lutris_id),
            Err(e) => eprintln!("match_game_to_steam: find_by_lutris_id error: {}", e),
        }
    });
}

fn match_game_to_sgdb(state: &SharedState, lutris_id: i64, sgdb_id: String, lutris_name: String) {
    let steam = state.borrow().steam.clone();
    let sender = state.borrow().sender.clone();
    let db = state.borrow().db.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::db::upsert_matching(&db, lutris_id, &sgdb_id, "sgdb", &sgdb_id) {
            eprintln!("match_game_to_sgdb: upsert_matching failed: {}", e);
            return;
        }
        let (icon, hero, grid, logo) = steam.ensure_sgdb_assets(&sgdb_id);

        if let Ok(Some(entry)) = crate::db::find_by_lutris_id(&db, lutris_id) {
            let mut game = crate::parser::Game {
                app_id: sgdb_id.clone(),
                kind: "sgdb".to_string(),
                platform_id: sgdb_id.clone(),
                db_id: entry.id,
                name: if entry.title.is_empty() { lutris_name.clone() } else { entry.title.clone() },
                icon_path: icon,
                hero_image_path: hero,
                grid_path: grid,
                header_path: String::new(),
                logo_path: logo,
                achievements: Vec::new(),
                earned_count: 0,
                total_count: 0,
                hidden: entry.hidden,
                lutris_id,
                slug: String::new(),
                playtime: 0.0,
                lastplayed: 0,
                logo_position: entry.logo_position.clone(),
                logo_size: entry.logo_size,
                lutris_name: lutris_name.clone(),
                manual_unmatch: false,
            };
            let _ = sender.send(AppMessage::NewGame(game));
        }
    });
}

pub fn confirm_mark_unlocked(state: &SharedState, kind: &str, app_id: &str, platform_id: &str, ach: &MergedAchievement, reload: impl Fn() + 'static) {
    let window = state.borrow().window.clone();
    let kind = kind.to_string();
    let app_id = app_id.to_string();
    let platform_id = platform_id.to_string();
    let ach_name = ach.name.clone();
    confirm_dialog(
        &window,
        S::MARK_UNLOCKED,
        &format!(
            "This will mark \u{201C}{}\u{201D} as earned without a real unlock time. \
             Use this only if you already unlocked it previously (e.g. before using this tool).",
            ach.display_name
        ),
        S::MARK_AS_UNLOCKED,
        adw::ResponseAppearance::Destructive,
        move || {
            if let Err(e) = set_achievement_earned(SAVE_DIR, &kind, &app_id, &platform_id, &ach_name, true) {
                eprintln!("Failed to mark achievement as unlocked: {}", e);
                return;
            }
            reload();
        },
    );
}

pub fn enrich_game_async(
    app_id: String,
    kind: String,
    platform_id: String,
    db_id: i64,
    lutris_id: i64,
    title: String,
    steam: Arc<SteamClient>,
    watcher: Option<AchievementWatcher>,
    sender: Sender<AppMessage>,
) {
    std::thread::spawn(move || {
        let entry = GameEntry {
            id: db_id,
            kind: kind.clone(),
            steam_id: app_id.clone(),
            platform_id: platform_id.clone(),
            title,
            lutris_db_id: if lutris_id != 0 { Some(lutris_id) } else { None },
            sgdb_id: None,
            hidden: false,
            logo_position: String::new(),
            logo_size: 0,
            ignored: Some(0),
            manual_unmatch: Some(0),
        };

        let meta_path = crate::parser::achievements_dir(SAVE_DIR, &app_id).join("achievements.json");
        if !meta_path.exists() {
            if let Err(e) = steam.generate_steam_settings(&app_id) {
                eprintln!("Could not generate achievements for {}: {}", app_id, e);
            }
        }

        let Ok(mut game) = load_game(&entry, SAVE_DIR) else {
            eprintln!("Failed reloading {}", app_id);
            return;
        };

        if game.name.starts_with("App ID:") {
            if let Some(name) = steam.fetch_nemirtingas_game_name(&app_id) {
                game.name = name;
            }
        }

        if let Some(details) = steam.fetch_game_details(&app_id) {
            if game.name.starts_with("App ID:") && !details.name.is_empty() {
                game.name = details.name.clone();
            }
            let has_local_icon = !game.icon_path.is_empty();
            let (icon_path, hero_path) = steam.ensure_assets(&app_id, has_local_icon);
            if game.icon_path.is_empty() && !icon_path.is_empty() {
                game.icon_path = icon_path;
            }
            if game.hero_image_path.is_empty() && !hero_path.is_empty() {
                game.hero_image_path = hero_path;
            }
        }

        let (grid_path, header_path, logo_path) = steam.ensure_grids(&app_id);
        if game.grid_path.is_empty() && !grid_path.is_empty() {
            game.grid_path = grid_path;
        }
        if game.header_path.is_empty() && !header_path.is_empty() {
            game.header_path = header_path;
        }
        if game.logo_path.is_empty() && !logo_path.is_empty() {
            game.logo_path = logo_path;
        }

        if let Some(pcts) = steam.fetch_global_achievements(&app_id) {
            for a in &mut game.achievements {
                if let Some(&pct) = pcts.get(&a.name) {
                    a.global_percent = pct;
                }
            }
        }

        if let Some(ref watcher) = watcher {
            watcher.watch(&entry, &game.achievements);
        }

        let _ = sender.send(AppMessage::EnrichedGame(game));
    });
}

fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    let suffixes = ["the", "final", "cut", "edition", "complete", "definitive", "remastered", "hd"];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

pub fn show_mass_match_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();

    let (unmatched, title_map) = {
        let s = state.borrow();
        let games = s.games.clone();
        let unmatched: Vec<Game> = games.into_iter().filter(|g| g.app_id.is_empty() && !g.manual_unmatch).collect();
        let data_dir = std::path::Path::new(SAVE_DIR).join("data").join("steam");
        let mut map: Vec<(String, String, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let app_id = match entry.file_name().to_str() {
                    Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                    _ => continue,
                };
                if let Some(name) = crate::parser::read_app_name(SAVE_DIR, &app_id) {
                    map.push((normalize_title(&name), app_id, name));
                }
            }
        }
        (unmatched, map)
    };

    if unmatched.is_empty() {
        let d = adw::AlertDialog::new(Some("No unmatched games"), Some("Every game already has a trophy source linked."));
        d.add_response("ok", "OK");
        d.present(Some(&window));
        return;
    }

    let dialog = adw::Window::new();
    dialog.set_default_width(600);
    dialog.set_default_height(500);
    dialog.set_transient_for(Some(&window));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("Match unmatched games"))));
    outer.append(&header_bar);

    let header = gtk4::Label::new(Some(&format!("{} unmatched game(s)", unmatched.len())));
    header.set_margin_top(16);
    header.set_margin_bottom(8);
    header.set_margin_start(16);
    header.set_margin_end(16);
    header.set_xalign(0.0);
    header.add_css_class("heading");
    outer.append(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);

    let mut row_action_boxes: Vec<gtk4::Box> = Vec::new();

    for game in &unmatched {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);

        let name_label = gtk4::Label::new(Some(&game.name));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&name_label);

        let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        action_box.append(&searching);
        row.append(&action_box);

        list.append(&row);
        row_action_boxes.push(action_box);
    }

    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_top(8);
    close_btn.set_margin_bottom(12);
    close_btn.set_margin_start(16);
    close_btn.set_margin_end(16);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));
    dialog.present();

    let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<(String, String)>, String, i64)>();
    let rx = std::cell::RefCell::new(rx);
    let remaining = std::cell::Cell::new(unmatched.len());

    let steam = state.borrow().steam.clone();
    let games_for_thread: Vec<(String, i64)> = unmatched.iter().map(|g| (g.name.clone(), g.lutris_id)).collect();
    std::thread::spawn(move || {
        for (i, (game_name, lutris_id)) in games_for_thread.iter().enumerate() {
            let norm = normalize_title(game_name);

            let matched: Option<(String, String)> = if norm.is_empty() {
                None
            } else {
                title_map
                    .iter()
                    .find(|(t, _, _)| t == &norm)
                    .map(|(_, id, name)| (id.clone(), name.clone()))
            };

            let final_match = if matched.is_some() {
                matched
            } else {
                let results = steam.search_steam_store(game_name);
                if results.is_empty() {
                    None
                } else {
                    results
                        .iter()
                        .find(|(_, name)| normalize_title(name) == norm)
                        .map(|(id, name)| (id.clone(), name.clone()))
                }
            };

            let _ = tx.send((i, final_match, game_name.clone(), *lutris_id));
        }
    });

    let state_rx = state.clone();
    let steam_rx = state.borrow().steam.clone();
    let row_boxes = row_action_boxes.clone();
    let parent_dialog = dialog.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok((row_idx, matched, game_name, lutris_id)) = rx.borrow_mut().try_recv() {
            if row_idx < row_boxes.len() {
                let action_box = &row_boxes[row_idx];
                while let Some(child) = action_box.first_child() {
                    action_box.remove(&child);
                }

                if let Some((sid, matched_name)) = matched {
                    match_game_to_steam(&state_rx, lutris_id, sid.clone(), game_name.clone());

                    let label = gtk4::Label::new(Some(&format!("Matched: {} ({})", matched_name, sid)));
                    label.add_css_class("success-label");
                    action_box.append(&label);

                    let undo_btn = gtk4::Button::with_label("Undo");
                    let sc = state_rx.clone();
                    let lid = lutris_id;
                    undo_btn.connect_clicked(move |_| {
                        let _ = crate::db::unmatch_game(&sc.borrow().db, lid);
                        if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.lutris_id == lid) {
                            g.app_id.clear();
                            g.kind.clear();
                            g.achievements.clear();
                            g.manual_unmatch = true;
                        }
                        rebuild_sidebar(&sc);
                    });
                    action_box.append(&undo_btn);
                } else {
                    let label = gtk4::Label::new(Some("Not found"));
                    label.add_css_class("dim-label");
                    action_box.append(&label);

                    let ab = action_box.clone();
                    let on_match: std::rc::Rc<dyn Fn(&str, &str)> = std::rc::Rc::new(move |sid, name| {
                        while let Some(child) = ab.first_child() {
                            ab.remove(&child);
                        }
                        let text = if name.is_empty() {
                            format!("Matched: {}", sid)
                        } else {
                            format!("Matched: {} ({})", name, sid)
                        };
                        let l = gtk4::Label::new(Some(&text));
                        l.add_css_class("success-label");
                        ab.append(&l);
                    });

                    let id_btn = gtk4::Button::with_label("Enter ID");
                    let sc = state_rx.clone();
                    let name = game_name.clone();
                    let lid = lutris_id;
                    let cb = on_match.clone();
                    id_btn.connect_clicked(move |_| {
                        let sc2 = sc.clone();
                        let name2 = name.clone();
                        let cb2 = cb.clone();
                        let body = format!("Enter the Steam app ID for \u{201C}{}\u{201D}:", name);
                        prompt_for_steam_id(&sc, "Match to Steam", &body, move |app_id| {
                            match_game_to_steam(&sc2, lid, app_id.to_string(), name2.clone());
                            cb2(&app_id, "");
                        });
                    });
                    action_box.append(&id_btn);

                    let steam_btn = gtk4::Button::with_label("Search Steam");
                    let sc2 = state_rx.clone();
                    let name2 = game_name.clone();
                    let steam2 = steam_rx.clone();
                    let cb2 = on_match.clone();
                    let pd = parent_dialog.clone();
                    steam_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc2, steam2.clone(), "Steam", &name2, lutris_id, SearchSource::Steam, cb2.clone(), pd.upcast_ref());
                    });
                    action_box.append(&steam_btn);

                    let sgdb_btn = gtk4::Button::with_label("Search SGDB");
                    let sc3 = state_rx.clone();
                    let name3 = game_name.clone();
                    let steam3 = steam_rx.clone();
                    let cb3 = on_match.clone();
                    let pd = parent_dialog.clone();
                    sgdb_btn.connect_clicked(move |_| {
                        show_search_results_dialog(&sc3, steam3.clone(), "SteamGridDB", &name3, lutris_id, SearchSource::SGDB, cb3.clone(), pd.upcast_ref());
                    });
                    action_box.append(&sgdb_btn);
                }

                let left = remaining.get();
                if left <= 1 {
                    return glib::ControlFlow::Break;
                }
                remaining.set(left - 1);
            }
        }
        glib::ControlFlow::Continue
    });
}

#[derive(Clone, Copy)]
enum SearchSource {
    Steam,
    SGDB,
}

fn show_search_results_dialog(
    state: &SharedState,
    steam: Arc<crate::steam::SteamClient>,
    source_name: &str,
    game_name: &str,
    lutris_id: i64,
    source: SearchSource,
    on_match: std::rc::Rc<dyn Fn(&str, &str)>,
    parent: &gtk4::Window,
) {
    let dialog = adw::Window::new();
    dialog.set_default_width(450);
    dialog.set_default_height(400);
    dialog.set_transient_for(Some(parent));
    dialog.set_modal(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header_bar = adw::HeaderBar::new();
    let title_label = gtk4::Label::new(Some(&format!("Search {}", source_name)));
    header_bar.set_title_widget(Some(&title_label));
    outer.append(&header_bar);

    let entry = gtk4::Entry::new();
    entry.set_text(game_name);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_top(12);
    entry.set_margin_bottom(8);
    outer.append(&entry);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let results_list = gtk4::ListBox::new();
    results_list.set_selection_mode(gtk4::SelectionMode::None);
    results_list.set_margin_start(12);
    results_list.set_margin_end(12);
    results_list.set_margin_bottom(8);

    let placeholder = gtk4::Label::new(Some("Searching..."));
    placeholder.add_css_class("dim-label");
    results_list.append(&placeholder);

    scrolled.set_child(Some(&results_list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = dialog.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));

    let entry_clone = entry.clone();
    let results_clone = results_list.clone();
    let state_clone = state.clone();
    let steam_clone = steam.clone();
    let name_clone = game_name.to_string();
    let dialog_clone = dialog.clone();
    let on_match_clone = on_match.clone();
    entry.connect_activate(move |_| {
        let term = entry_clone.text().to_string();
        if term.is_empty() {
            return;
        }

        while let Some(child) = results_clone.first_child() {
            results_clone.remove(&child);
        }
        let searching = gtk4::Label::new(Some("Searching..."));
        searching.add_css_class("dim-label");
        results_clone.append(&searching);

        let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
        let rx = std::cell::RefCell::new(rx);

        let steam = steam_clone.clone();
        let src = source;
        std::thread::spawn(move || {
            let search_results = match src {
                SearchSource::Steam => steam.search_steam_store(&term),
                SearchSource::SGDB => steam.search_sgdb(&term),
            };
            let _ = tx.send(search_results);
        });

        let sc = state_clone.clone();
        let results = results_clone.clone();
        let name = name_clone.clone();
        let cb = on_match_clone.clone();
        let dlg = dialog_clone.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok(search_results) = rx.borrow_mut().try_recv() {
                while let Some(child) = results.first_child() {
                    results.remove(&child);
                }

                if search_results.is_empty() {
                    let none = gtk4::Label::new(Some("No results found"));
                    none.add_css_class("dim-label");
                    results.append(&none);
                    return glib::ControlFlow::Break;
                }

                for (app_id, result_name) in search_results {
                    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    row.set_margin_top(4);
                    row.set_margin_bottom(4);

                    let label = gtk4::Label::new(Some(&format!("{} ({})", result_name, app_id)));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    row.append(&label);

                    let match_btn = gtk4::Button::with_label("Match");
                    match_btn.add_css_class("suggested-action");
                    let sc2 = sc.clone();
                    let name2 = name.clone();
                    let sid = app_id.clone();
                    let matched_name = result_name.clone();
                    let lid = lutris_id;
                    let dialog_clone = dlg.clone();
                    let callback = cb.clone();
                    let src_type = source;
                    match_btn.connect_clicked(move |_| {
                        match src_type {
                            SearchSource::Steam => match_game_to_steam(&sc2, lid, sid.clone(), name2.clone()),
                            SearchSource::SGDB => match_game_to_sgdb(&sc2, lid, sid.clone(), name2.clone()),
                        }
                        callback(&sid, &matched_name);
                        dialog_clone.close();
                    });
                    row.append(&match_btn);

                    results.append(&row);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });

    dialog.present();
    entry.emit_activate();
}
