use gtk4::prelude::*;
use adw::prelude::*;
use std::rc::Rc;

use super::state::SharedState;
use super::helpers::clear_children;
use super::enrichment::{enrich_game_async, EnrichGameParams};

pub fn show_ra_search_dialog(state: &SharedState, db_id: i64, game_name: &str, platform_id: &str, parent: &adw::Window, on_match: Option<Rc<dyn Fn()>>) {
    let console_id = match ira_models::find_console(platform_id) {
        Some(c) => c.ra_console_id,
        None => return,
    };

    let dialog = adw::Window::new();
    dialog.set_default_width(500);
    dialog.set_default_height(400);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some("Match to RetroAchievements"))));
    outer.append(&header);

    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Game name…"));
    entry.set_text(game_name);
    entry.set_hexpand(true);
    let search_btn = gtk4::Button::with_label("Search");
    search_btn.add_css_class("suggested-action");
    search_box.append(&entry);
    search_box.append(&search_btn);
    outer.append(&search_box);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_margin_top(8);
    let list = gtk4::ListBox::new();
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(8);
    list.set_margin_bottom(12);
    list.set_valign(gtk4::Align::Start);
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let state_c = state.clone();
    let dialog_c = dialog.clone();
    let list_c = list.clone();
    let platform_id = platform_id.to_string();

    let entry_s = entry.clone();
    let do_search = move || {
        let term = entry_s.text().to_string();
        if term.is_empty() { return; }
        let save_dir = state_c.borrow().save_dir.clone();
        let results = ira_platforms::retroachievements::api::RaClient::search_ra_games(&save_dir, console_id, &term);
        clear_children(&list_c);
        if results.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No results found");
            row.set_sensitive(false);
            list_c.append(&row);
        } else {
            for game in &results {
                let row = adw::ActionRow::new();
                row.set_title(&super::helpers::esc(&game.title));
                row.set_subtitle(&format!("RA ID: {} · {} achievements", game.id, game.num_achievements));
                let match_btn = gtk4::Button::with_label("Match");
                match_btn.add_css_class("suggested-action");
                match_btn.set_valign(gtk4::Align::Center);
                let sc = state_c.clone();
                let dc = dialog_c.clone();
                let ra_id = game.id;
                let ra_title = game.title.clone();
                let on_match_c = on_match.clone();
                let pid = platform_id.clone();
                match_btn.connect_clicked(move |_| {
                    let app_id = ra_id.to_string();
                    if let Err(e) = ira_db::update_game_ids(&sc.borrow().db, db_id, "", &app_id, ira_models::TrophySource::Ra, &pid) {
                        eprintln!("Failed to update game IDs for RA match: {}", e);
                    }
                    if let Err(e) = ira_db::set_manual_unmatch(&sc.borrow().db, db_id, false) {
                        eprintln!("Failed to clear manual unmatch: {}", e);
                    }
                    if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
                        g.app_id = app_id.clone();
                        g.trophy_source = ira_models::TrophySource::Ra;
                        g.name = ra_title.clone();
                        g.total_count = 0;
                        g.achievements.clear();
                    }
                    if let Some(ref sd) = sc.borrow().settings_data {
                        if sd.db_id == db_id {
                            let key = format!("__ra_unmatch_{}", db_id);
                            sd.pending_copies.borrow_mut().remove(&key);
                        }
                    }
                    let (ra_username, ra_token, ra_password, steam, sender, save_dir, db) = {
                        let s = sc.borrow();
                        (s.cfg.ra_username.clone(), s.cfg.ra_token.clone(), s.cfg.ra_password.clone(),
                         s.steam.clone(), s.sender.clone(), s.save_dir.clone(), s.db.clone())
                    };
                    let g = sc.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
                    if let Some(g) = g {
                        enrich_game_async(EnrichGameParams {
                            app_id: g.app_id.clone(),
                            trophy_source: g.trophy_source,
                            platform_id: g.platform_id.clone(),
                            db_id: g.db_id,
                            title: g.name.clone(),
                            steam, sender, save_dir, db,
                            ra_username, ra_token, ra_password,
                            game: None,
                        });
                    }
                    if let Some(ref cb) = on_match_c { cb(); }
                    dc.close();
                    let sc_refresh = sc.clone();
                    glib::idle_add_local_once(move || {
                        super::game_settings::refresh_ra_section(&sc_refresh, db_id);
                        super::helpers::refresh_settings_images_page(&sc_refresh, db_id, |s, game, win, pc| {
                            super::image_manager::build_image_manager_content_with_drafts(s, game, win, pc).upcast()
                        });
                    });
                });
                row.add_suffix(&match_btn);
                list_c.append(&row);
            }
        }
    };

    let do_search = Rc::new(do_search);
    entry.connect_activate({
        let ds = do_search.clone();
        move |_| ds()
    });
    search_btn.connect_clicked({
        let ds = do_search.clone();
        move |_| ds()
    });

    dialog.set_content(Some(&outer));
    dialog.present();
    do_search();
}
