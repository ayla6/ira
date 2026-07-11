use crate::models::{GameLaunchConfig, WineConfig};
use crate::AppMessage;
use gtk4::prelude::*;
use adw::prelude::*;
use super::state::SharedState;

pub fn show_add_game_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let db = state.borrow().db.clone();
    let sender = state.borrow().sender.clone();
    let steam = state.borrow().steam.clone();
    let watcher = state.borrow().watcher.clone();
    let save_dir = state.borrow().save_dir.clone();
    let dialog = adw::Dialog::new();
    dialog.set_title("Add Game");
    dialog.set_content_width(550);
    dialog.set_content_height(600);

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);

    let groups = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&groups));
    scroll.set_vexpand(true);
    page.append(&scroll);

    // --- Game Info ---
    let info_group = adw::PreferencesGroup::new();
    info_group.set_title("Game Info");

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Name");
    info_group.add(&name_entry);

    let kind_model = gtk4::StringList::new(&["Native Linux", "Wine (Windows)"]);
    let kind_row = adw::ComboRow::new();
    kind_row.set_title("Kind");
    kind_row.set_model(Some(&kind_model));
    kind_row.set_selected(1);
    info_group.add(&kind_row);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable");
    info_group.add(&exe_entry);

    let (wine_page, wine_widgets) = {
        let cfg = WineConfig { enabled: true, ..Default::default() };
        super::wine_config_widget::build_wine_config_page(&cfg)
    };
    wine_page.set_visible(true);
    groups.append(&info_group);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    info_group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");
    info_group.add(&wd_entry);

    // --- Achievement Source ---
    let ach_group = adw::PreferencesGroup::new();
    ach_group.set_title("Achievement Source");

    let steam_id_entry = adw::EntryRow::new();
    steam_id_entry.set_title("Steam App ID");
    ach_group.add(&steam_id_entry);

    let gog_id_entry = adw::EntryRow::new();
    gog_id_entry.set_title("GOG Product ID");
    ach_group.add(&gog_id_entry);

    groups.append(&ach_group);
    groups.append(&wine_page);

    // --- Kind toggle visibility ---
    {
        let wp = wine_page.clone();
        kind_row.connect_selected_notify(move |row| {
            wp.set_visible(row.selected() == 1);
        });
    }

    // --- Auto-detect ---
    let detect_group = adw::PreferencesGroup::new();
    detect_group.set_title("Quick detect");
    let detect_row = adw::ActionRow::new();
    detect_row.set_title("Select game folder to auto-detect");
    let detect_btn = gtk4::Button::with_label("Browse");
    detect_btn.add_css_class("flat");
    let sc = state.clone();
    let n_detect = name_entry.clone();
    let exe_detect = exe_entry.clone();
    let sid_detect = steam_id_entry.clone();
    let gid_detect = gog_id_entry.clone();
    detect_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::new();
        file_dialog.set_title("Select game folder");
        let sc_c = sc.clone();
        let n = n_detect.clone();
        let exe = exe_detect.clone();
        let sid = sid_detect.clone();
        let gid = gid_detect.clone();
        file_dialog.select_folder(Some(&sc_c.borrow().window), None::<&gio::Cancellable>, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            let folder = path.to_string_lossy().into_owned();
            if let Some(app_id) = crate::platforms::steam_setup::detect_app_id(&folder) {
                sid.set_text(&app_id);
            }
            if crate::platforms::gog::is_gog_game(&folder) {
                if let Some((_info_dir, product_id, game_name)) = crate::platforms::gog::find_gog_info(&folder) {
                    gid.set_text(&product_id);
                    n.set_text(&game_name);
                }
            }
            if exe.text().is_empty() {
                if let Ok(entries) = std::fs::read_dir(&folder) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if let Some(ext) = p.extension() {
                            if ext == "exe" || ext == "x86_64" || ext == "AppRun" || ext == "sh" {
                                exe.set_text(&p.to_string_lossy());
                                break;
                            }
                        }
                    }
                }
            }
        });
    });
    detect_row.add_suffix(&detect_btn);
    detect_group.add(&detect_row);
    groups.append(&detect_group);

    let add_btn = gtk4::Button::with_label("Add Game");
    add_btn.add_css_class("suggested-action");
    add_btn.set_halign(gtk4::Align::End);
    add_btn.set_margin_top(16);
    page.append(&add_btn);

    toolbar_view.set_content(Some(&page));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(&window));

    add_btn.connect_clicked(move |_| {
        let name = name_entry.text().to_string();
        if name.is_empty() {
            return;
        }
        let exe_path = exe_entry.text().to_string();
        if exe_path.is_empty() {
            return;
        }
        let is_wine = kind_row.selected() == 1;
        let args = args_entry.text().to_string();
        let wd = wd_entry.text().to_string();
        let steam_app_id = steam_id_entry.text().to_string();
        let gog_product_id = gog_id_entry.text().to_string();

        let trophy_source = if !steam_app_id.is_empty() { "gse" } else if !gog_product_id.is_empty() { "nge" } else { "" };
        let kind = if is_wine { "wine" } else { "linux" };
        let platform_id = if !steam_app_id.is_empty() { steam_app_id.clone() } else if !gog_product_id.is_empty() { gog_product_id.clone() } else { format!("manual_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()) };

        let launch_config = GameLaunchConfig {
            exe: exe_path,
            args,
            working_dir: wd,
            ..Default::default()
        };
        let wine_config = if is_wine { wine_widgets.to_wine_config() } else { WineConfig::default() };

        let db_c = db.clone();
        let sender_c = sender.clone();
        let name_c = name.clone();
        let app_id_c = platform_id.clone();
        let kind_c = kind.to_string();
        let ts_c = trophy_source.to_string();
        let steam_c = steam.clone();
        let watcher_c = watcher.clone();
        let save_dir_c = save_dir.clone();

        std::thread::spawn(move || {
            match add_game_to_db(&db_c, &name_c, &kind_c, &ts_c, &app_id_c, &platform_id, &launch_config, &wine_config, &steam_c, &save_dir_c) {
                Ok(_game_id) => {
                    let entry = crate::db::find_by_steam_id(&db_c, &app_id_c).ok().flatten();
                    if let Some(entry) = entry {
                        if let Ok(mut game) = crate::parser::load_game(&entry, &save_dir_c) {
                            game.name = name_c.clone();
                            let _ = crate::db::update_game_title(&db_c, game.db_id, &name_c);
                            if let Some(ref w) = watcher_c {
                                w.watch(&entry, &game.achievements);
                            }
                            let _ = sender_c.send(AppMessage::NewGame(game.clone()));
                            let g_name = game.name.clone();
                            crate::ui::enrichment::enrich_game_async(
                                game.app_id.clone(), game.trophy_source.clone(), game.platform_id.clone(),
                                game.db_id, game.lutris_id, g_name,
                                steam_c, watcher_c, sender_c, save_dir_c,
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to add game: {}", e);
                    let _ = sender_c.send(AppMessage::AddGameError(e));
                }
            }
        });
    });
}

fn add_game_to_db(
    db: &crate::db::DbConn,
    name: &str,
    kind: &str,
    trophy_source: &str,
    app_id: &str,
    platform_id: &str,
    launch_config: &GameLaunchConfig,
    wine_config: &WineConfig,
    steam: &crate::api::SteamClient,
    save_dir: &str,
) -> Result<i64, String> {
    let game_id = crate::db::add_game(db, kind, trophy_source, app_id, platform_id, name)?;
    crate::db::save_game_config(db, game_id, launch_config, wine_config)?;
    if !app_id.is_empty() && app_id.parse::<i64>().is_ok() {
        let folder = std::path::Path::new(&launch_config.exe).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        if !folder.is_empty() {
            let _ = crate::platforms::steam_setup::add_game_from_folder(&folder, app_id, kind, steam, db, save_dir);
        }
    }
    Ok(game_id)
}
