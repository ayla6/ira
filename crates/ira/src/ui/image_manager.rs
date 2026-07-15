use gtk4::prelude::*;
use crate::Game;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use super::helpers::{clear_children, make_browse_button, refresh_settings_images_page};
use super::sgdb_match_dialog::show_sgdb_search_dialog;
use super::sgdb_picker::{show_sgdb_picker, ShowSgdbPickerParams};
use super::message_handler::apply_game_update;
use super::state::SharedState;

type SectionEntry = (&'static str, &'static str, &'static str, i32, i32, &'static [&'static str]);

pub fn build_image_manager_content(state: &SharedState, game: &Game, parent_win: &adw::Window) -> gtk4::Box {
    build_image_manager_content_with_drafts(state, game, parent_win, None)
}

pub fn build_image_manager_content_with_drafts(
    state: &SharedState,
    game: &Game,
    parent_win: &adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
) -> gtk4::Box {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let is_steam = ira_models::has_steam_enrichment(&game.trophy_source);

    let sections: [SectionEntry; 5] = [
        ("Icon", "icon.png", "icon", 48, 48, &[]),
        ("Hero", "library_hero.jpg", "hero", 96, 48, &[]),
        ("Capsule", "library_600x900.jpg", "grid", 32, 48, &["600x900"]),
        ("Header", "header.jpg", "header", 96, 48, &["460x215", "920x430"]),
        ("Logo", "logo.png", "logo", 96, 48, &[]),
    ];

    for &(label, file, asset, thumb_w, thumb_h, dimensions) in &sections {
        let section = build_image_section(BuildImageSectionParams {
            label, file_base: file, asset_type: asset,
            thumb_w, thumb_h, dims: dimensions,
            game, state, parent_win,
            pending_copies: pending_copies.clone(),
        });
        content.append(&section);
    }

    {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        btn_box.set_halign(gtk4::Align::Center);
        btn_box.set_margin_top(24);

        if game.sgdb_id.is_empty() && !is_steam {
            let match_btn = gtk4::Button::with_label("Match to SteamGridDB…");
            match_btn.add_css_class("suggested-action");
            let sc = state.clone();
            let gn = game.name.clone();
            let did = game.db_id;
            let pw = parent_win.clone();
            match_btn.connect_clicked(move |_| {
                show_sgdb_search_dialog(&sc, did, &gn, &pw, None);
            });
            btn_box.append(&match_btn);
        }

        if !game.sgdb_id.is_empty() {
            let label = gtk4::Label::new(Some(&format!("Matched (SGDB ID: {})", game.sgdb_id)));
            label.add_css_class("success-label");
            btn_box.append(&label);
            let unmatch_btn = gtk4::Button::with_label("Unmatch SGDB");
            unmatch_btn.add_css_class("destructive-action");
            let pending_pc = pending_copies.clone();
            let sc = state.clone();
            let did = game.db_id;
            unmatch_btn.connect_clicked(move |_| {
                if let Some(ref pc) = pending_pc {
                    pc.borrow_mut().insert("__unmatch__".to_string(), String::new());
                    refresh_settings_images_page(&sc, did, |s, game, win| {
                        let mut g2 = game.clone();
                        g2.sgdb_id.clear();
                        build_image_manager_content_with_drafts(s, &g2, win, Some(pc.clone())).upcast()
                    });
                }
            });
            btn_box.append(&unmatch_btn);
        }

        content.append(&btn_box);
    }

    content
}

fn find_best_image_path(game: &Game, field: &str, file: &str, id: &str, save_dir: &str) -> String {
    let field_path = match field {
        "icon" if !game.icon_path.is_empty() => game.icon_path.clone(),
        "hero" if !game.hero_image_path.is_empty() => game.hero_image_path.clone(),
        "grid" if !game.grid_path.is_empty() => game.grid_path.clone(),
        "header" if !game.header_path.is_empty() => game.header_path.clone(),
        "logo" if !game.logo_path.is_empty() => game.logo_path.clone(),
        _ => String::new(),
    };
    if !field_path.is_empty() && std::path::Path::new(&field_path).is_file() {
        return field_path;
    }
    if !game.sgdb_id.is_empty() {
        let sgdb = format!("{}/{}", ira_parser::sgdb_data_dir(save_dir, &game.sgdb_id).to_string_lossy(), file);
        if std::path::Path::new(&sgdb).is_file() {
            return sgdb;
        }
    }
    let native = if game.kind == ira_models::PS4 {
        format!("{}/{}", ira_parser::ps4_data_dir(save_dir, id).to_string_lossy(), file)
    } else {
        format!("{}/{}", ira_parser::data_dir(save_dir, id).to_string_lossy(), file)
    };
    if std::path::Path::new(&native).is_file() {
        return native;
    }
    if field == "icon" && game.kind == ira_models::PS4 && !game.icon_path.is_empty() && std::path::Path::new(&game.icon_path).is_file() {
        return game.icon_path.clone();
    }
    String::new()
}

fn make_refresh_closure(
    preview_wrapper: &gtk4::Box,
    dest_path: &str,
    state: &SharedState,
    game: &Game,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
    asset_type: &str,
) -> Rc<dyn Fn()> {
    let save_dir = state.borrow().save_dir.clone();
    Rc::new({
        let preview_wrapper = preview_wrapper.clone();
        let dest_path = dest_path.to_string();
        let state_clone = state.clone();
        let game_clone = game.clone();
        let pending_copies = pending_copies.clone();
        let asset_c = asset_type.to_string();
        move || {
            clear_children(&preview_wrapper);
            let preview_src = pending_copies.as_ref()
                .and_then(|pc| pc.borrow().get(&asset_c).cloned())
                .filter(|p| std::path::Path::new(p).is_file())
                .or_else(|| {
                    if std::path::Path::new(&dest_path).exists() {
                        Some(dest_path.clone())
                    } else {
                        None
                    }
                });
            if let Some(path) = preview_src {
                let p = gtk4::Picture::for_filename(&path);
                p.set_content_fit(gtk4::ContentFit::ScaleDown);
                preview_wrapper.append(&p);
            } else {
                let ph = gtk4::Label::new(Some("—"));
                ph.add_css_class("dim-label");
                preview_wrapper.append(&ph);
            }
            let s = state_clone.borrow();
            if let Ok(Some(entry)) = ira_db::find_by_lutris_id(&s.db, game_clone.lutris_id) {
                drop(s);
                if let Ok(updated) = crate::game_loader::load_game(&entry, &save_dir) {
                    apply_game_update(&state_clone, updated);
                }
            }
        }
    })
}

struct BuildImageSectionParams<'a> {
    label: &'a str,
    file_base: &'a str,
    asset_type: &'a str,
    thumb_w: i32,
    thumb_h: i32,
    dims: &'a [&'static str],
    game: &'a Game,
    state: &'a SharedState,
    parent_win: &'a adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
}

fn build_image_section(params: BuildImageSectionParams) -> gtk4::Box {
    let BuildImageSectionParams { label, file_base, asset_type, thumb_w, thumb_h, dims, game, state, parent_win, pending_copies } = params;
    let is_steam = ira_models::has_steam_enrichment(&game.trophy_source);
    let id = game.app_id.clone();
    let save_dir = state.borrow().save_dir.clone();

    let cloud_dir = if !game.sgdb_id.is_empty() {
        ira_parser::sgdb_data_dir(&save_dir, &game.sgdb_id)
    } else if game.kind == ira_models::PS4 {
        ira_parser::ps4_data_dir(&save_dir, &id)
    } else {
        ira_parser::data_dir(&save_dir, &id)
    };
    let cloud_base = cloud_dir.to_string_lossy().into_owned();

    let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let lbl = gtk4::Label::new(Some(label));
    lbl.set_halign(gtk4::Align::Start);
    lbl.add_css_class("heading");
    section.append(&lbl);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_hexpand(true);
    row.set_valign(gtk4::Align::Center);

    let img_path = {
        let draft_path = pending_copies.as_ref()
            .and_then(|pc| pc.borrow().get(asset_type).cloned());
        if let Some(ref src) = draft_path {
            if std::path::Path::new(src).is_file() {
                src.clone()
            } else {
                find_best_image_path(game, asset_type, file_base, &id, &save_dir)
            }
        } else {
            find_best_image_path(game, asset_type, file_base, &id, &save_dir)
        }
    };

    let preview = gtk4::Picture::for_filename(&img_path);
    preview.set_content_fit(gtk4::ContentFit::ScaleDown);
    preview.set_size_request(thumb_w, thumb_h);
    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_size_request(thumb_w, 48);
    preview_wrapper.set_valign(gtk4::Align::Center);
    if !img_path.is_empty() && std::path::Path::new(&img_path).is_file() {
        preview_wrapper.append(&preview);
    } else {
        let ph = gtk4::Label::new(Some("—"));
        ph.add_css_class("dim-label");
        preview_wrapper.append(&ph);
    }
    row.append(&preview_wrapper);

    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btns.set_hexpand(true);
    btns.set_halign(gtk4::Align::End);

    let dest_path = format!("{}/{}", cloud_base, file_base);
    let refresh_images = make_refresh_closure(
        &preview_wrapper, &dest_path, state, game, pending_copies.clone(), asset_type,
    );

    let browse_btn = make_browse_button(
        Some(parent_win),
        "Select image",
        false,
        Some(("Images", &["image/png", "image/jpeg", "image/webp", "image/x-icon"])),
        {
            let pc = pending_copies.clone();
            let refresh = refresh_images.clone();
            let asset_name = asset_type.to_string();
            move |path| {
                if let Some(ref pc_inner) = pc {
                    pc_inner.borrow_mut().insert(asset_name.clone(), path.to_string_lossy().into_owned());
                    refresh();
                }
            }
        },
    );
    btns.append(&browse_btn);

    if is_steam && asset_type != "icon" {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let asset_c = asset_type.to_string();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            let _ = steam.force_download_steam(&id_c, &asset_c);
            refresh();
        });
        btns.append(&btn);
    }

    if is_steam && asset_type == "icon" && game.trophy_source == ira_models::STEAM_NATIVE {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let save_dir_c = save_dir.clone();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            if let Ok(app_id_num) = id_c.parse::<u32>() {
                if let Some(clienticon) = ira_platforms::steam::get_clienticon(app_id_num) {
                    let dest = ira_parser::data_dir(&save_dir_c, &id_c).join("icon.png");
                    let _ = std::fs::create_dir_all(dest.parent().unwrap());
                    let ico_path = ira_platforms::steam::steam_install_dir()
                        .map(|d| d.join("steam").join("games").join(format!("{}.ico", clienticon)));
                    let got = if let Some(ref p) = ico_path {
                        if p.is_file() {
                            if let Ok(ico_data) = std::fs::read(p) {
                                let tmp = dest.with_extension("ico");
                                if std::fs::write(&tmp, &ico_data).is_ok() {
                                    let r = ira_parser::convert_ico_to_png(&tmp).ok()
                                        .map(|png| { let _ = std::fs::rename(&png, &dest); std::fs::remove_file(&tmp).ok();  });
                                    let _ = std::fs::remove_file(&tmp);
                                    r.is_some()
                                } else { false }
                            } else { false }
                        } else { false }
                    } else { false };
                    if !got {
                        let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", id_c, clienticon);
                        let tmp = dest.with_extension("ico");
                        if steam.download_file(&url, &tmp).is_ok() {
                            if let Ok(png) = ira_parser::convert_ico_to_png(&tmp) {
                                let _ = std::fs::rename(&png, &dest);
                            }
                            let _ = std::fs::remove_file(&tmp);
                        }
                    }
                }
            }
            refresh();
        });
        btns.append(&btn);
    }

    let sgdb_id_for_picker = if !game.sgdb_id.is_empty() {
        game.sgdb_id.clone()
    } else {
        id.clone()
    };
    let sgdb_is_steam_id = is_steam && game.sgdb_id.is_empty();
    let pending_copies_btn = pending_copies.clone();
    if !sgdb_id_for_picker.is_empty() {
        let btn = gtk4::Button::with_label("SGDB…");
        let steam = state.borrow().steam.clone();
        let asset_c = asset_type.to_string();
        let parent = parent_win.clone();
        let refresh = refresh_images.clone();
        let dims_vec: Vec<&str> = dims.to_vec();
    let sgdb_id_c = sgdb_id_for_picker.clone();
    let save_dir_c = save_dir.clone();
    btn.connect_clicked(move |_| {
        show_sgdb_picker(ShowSgdbPickerParams {
            steam: &steam, id: &sgdb_id_c, asset: &asset_c,
            is_steam_id: sgdb_is_steam_id, dimensions: &dims_vec,
            parent: &parent, on_done: refresh.clone(),
            pending_copies: pending_copies_btn.clone(), save_dir: &save_dir_c,
        });
    });
    btns.append(&btn);
    }

    if asset_type == "icon" && game.kind == ira_models::PS4 {
        let reset_btn = gtk4::Button::with_label("Reset");
        let gc = game.clone();
        let refresh = refresh_images.clone();
        let pending_copies_reset = pending_copies.clone();
        let asset_reset = asset_type.to_string();
        let save_dir_c2 = save_dir.clone();
        reset_btn.connect_clicked(move |_| {
            let app_id = gc.app_id.clone();
            let game_path = gc.game_path.clone();
            let image_dir = std::path::Path::new(&save_dir_c2).join("data").join("ps4").join(&app_id);
            let icon_png = image_dir.join("icon.png");
            let default_path = if icon_png.is_file() {
                Some(icon_png.to_string_lossy().into_owned())
            } else {
                let game_icon = std::path::Path::new(&game_path).join("sce_sys").join("icon0.png");
                if game_icon.is_file() {
                    let _ = std::fs::create_dir_all(&image_dir);
                    let _ = std::fs::copy(&game_icon, &icon_png);
                    Some(icon_png.to_string_lossy().into_owned())
                } else {
                    None
                }
            };
            if let Some(ref pc) = pending_copies_reset {
                pc.borrow_mut().remove(&asset_reset);
                if let Some(path) = default_path {
                    pc.borrow_mut().insert(asset_reset.clone(), path);
                }
            }
            refresh();
        });
        btns.append(&reset_btn);
    }

    row.append(&btns);
    section.append(&row);
    section
}


