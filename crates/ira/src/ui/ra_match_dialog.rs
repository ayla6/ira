use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use super::css::*;
use super::enrichment::{enrich_game_async, EnrichGameParams};
use super::helpers::clear_children;
use super::state::SharedState;
use ira_platforms::retroachievements::api::{RaClient, RaGameEntry};

fn apply_ra_match(
    sc: &SharedState,
    db_id: i64,
    platform_id: &str,
    ra_id: u32,
    ra_title: &str,
    on_match: &Option<Rc<dyn Fn()>>,
    dialog: &adw::Window,
) {
    let app_id = ra_id.to_string();
    if let Err(e) = ira_db::update_game_ids(
        &sc.borrow().db,
        db_id,
        "",
        &app_id,
        ira_models::TrophySource::Ra,
        platform_id,
    ) {
        eprintln!("Failed to update game IDs for RA match: {}", e);
    }
    if let Err(e) = ira_db::set_manual_unmatch(&sc.borrow().db, db_id, false) {
        eprintln!("Failed to clear manual unmatch: {}", e);
    }
    if let Some(g) = sc.borrow_mut().games.iter_mut().find(|g| g.db_id == db_id) {
        g.app_id = app_id;
        g.trophy_source = ira_models::TrophySource::Ra;
        if !ra_title.is_empty() {
            g.set_name(ra_title);
        }
        g.total_count = 0;
        g.achievements.clear();
    }
    if let Some(ref sd) = sc.borrow().settings_data {
        if sd.db_id == db_id {
            let key = format!("__ra_unmatch_{}", db_id);
            sd.pending_copies.borrow_mut().remove(&key);
        }
    }
    let (ra_username, ra_web_api_key, steam, sender, save_dir, db) = {
        let s = sc.borrow();
        (
            s.cfg.ra_username.clone(),
            s.cfg.ra_web_api_key.clone(),
            s.steam.clone(),
            s.sender.clone(),
            s.save_dir.clone(),
            s.db.clone(),
        )
    };
    let g = sc.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
    if let Some(g) = g {
        enrich_game_async(EnrichGameParams {
            app_id: g.app_id.clone(),
            trophy_source: g.trophy_source,
            platform_id: g.platform_id.clone(),
            db_id: g.db_id,
            title: g.name,
            steam,
            sender,
            save_dir,
            db,
            ra_username,
            ra_web_api_key,
            game: None,
        });
    }
    if let Some(ref cb) = on_match {
        cb();
    }
    dialog.close();
    let sc_refresh = sc.clone();
    glib::idle_add_local_once(move || {
        super::game_settings::refresh_ra_section(&sc_refresh, db_id);
        super::helpers::refresh_settings_images_page(
            &sc_refresh,
            db_id,
            |s, game, win, pc, scache| {
                super::image_manager::build_image_manager_content_with_drafts(
                    s, game, win, pc, scache,
                )
                .upcast()
            },
        );
    });
}

fn populate_results(
    list: &gtk4::ListBox,
    state: &SharedState,
    db_id: i64,
    platform_id: &str,
    dialog: &adw::Window,
    on_match: &Option<Rc<dyn Fn()>>,
    outcome: (Option<String>, Vec<RaGameEntry>),
) {
    let (notice, results) = outcome;
    clear_children(list);
    if let Some(notice) = notice {
        let row = adw::ActionRow::new();
        row.set_title(&notice);
        row.set_sensitive(false);
        list.append(&row);
        return;
    }
    if results.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("No results found"));
        row.set_sensitive(false);
        list.append(&row);
        return;
    }
    for game in &results {
        let row = adw::ActionRow::new();
        row.set_title(&super::helpers::esc(&game.title));
        let achievements = if game.num_achievements == 0 {
            crate::tr!("No achievements yet")
        } else {
            crate::tr!("{} achievements").replacen("{}", &game.num_achievements.to_string(), 1)
        };
        row.set_subtitle(&format!("RA ID: {} · {}", game.id, achievements));
        let match_btn = gtk4::Button::with_label(&crate::tr!("Match"));
        match_btn.add_css_class(CSS_SUGGESTED_ACTION);
        match_btn.set_valign(gtk4::Align::Center);
        let sc = state.clone();
        let dc = dialog.clone();
        let ra_id = game.id;
        let ra_title = game.title.clone();
        let on_match_c = on_match.clone();
        let pid = platform_id.to_string();
        match_btn.connect_clicked(move |_| {
            apply_ra_match(&sc, db_id, &pid, ra_id, &ra_title, &on_match_c, &dc);
        });
        row.add_suffix(&match_btn);
        list.append(&row);
    }
}

pub fn show_ra_search_dialog(
    state: &SharedState,
    db_id: i64,
    game_name: &str,
    platform_id: &str,
    parent: &adw::Window,
    on_match: Option<Rc<dyn Fn()>>,
) {
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
    header.set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!(
        "Match to RetroAchievements"
    )))));
    outer.append(&header);

    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Game name…")));
    entry.set_text(game_name);
    entry.set_hexpand(true);
    let search_btn = gtk4::Button::with_label(&crate::tr!("Search"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
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
    let list_c = list;
    let platform_id = platform_id.to_string();

    let entry_s = entry.clone();
    let dialog_s = dialog.clone();
    let do_search = move || {
        let term = entry_s.text().trim().to_string();
        if term.is_empty() {
            return;
        }
        let (cfg, save_dir) = {
            let s = state_c.borrow();
            (s.cfg.clone(), s.save_dir.clone())
        };
        let list_c2 = list_c.clone();
        let state_c2 = state_c.clone();
        let dialog_c2 = dialog_s.clone();
        let on_match_c2 = on_match.clone();
        let platform_id_c2 = platform_id.clone();
        let (tx, rx) = mpsc::channel::<(Option<String>, Vec<RaGameEntry>)>();
        std::thread::spawn(move || {
            let (notice, results) = match RaClient::from_config(&cfg) {
                Some(client) => (None, client.search_ra_games(&save_dir, console_id, &term)),
                None => (
                    Some(crate::tr!("RetroAchievements credentials not configured")),
                    Vec::new(),
                ),
            };
            let _ = tx.send((notice, results));
        });
        let rx = Rc::new(RefCell::new(rx));
        glib::source::idle_add_local_full(glib::Priority::DEFAULT, move || {
            let rx = rx.borrow();
            match rx.try_recv() {
                Ok((notice, results)) => {
                    populate_results(
                        &list_c2,
                        &state_c2,
                        db_id,
                        &platform_id_c2,
                        &dialog_c2,
                        &on_match_c2,
                        (notice, results),
                    );
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
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
