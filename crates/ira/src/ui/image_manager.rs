use gtk4::prelude::*;
use adw::prelude::*;
use crate::Game;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use super::helpers::{clear_children, make_browse_button, refresh_settings_images_page};
use super::sgdb_match_dialog::show_sgdb_search_dialog;
use super::sgdb_picker::{show_sgdb_picker, ShowSgdbPickerParams};
use super::message_handler::apply_game_update;
use super::state::SharedState;
use super::game_item::GameItem;

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

    let is_steam = game.trophy_source.has_steam_enrichment();

    let sections: [SectionEntry; 5] = [
        ("Icon", "icon", "icon", 48, 48, &[]),
        ("Hero", "hero", "hero", 96, 64, &[]),
        ("Capsule", "vertical", "grid", 48, 64, &["600x900"]),
        ("Header", "header", "header", 96, 48, &["460x215", "920x430"]),
        ("Logo", "logo", "logo", 64, 64, &[]),
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
                    refresh_settings_images_page(&sc, did, |s, game, win, pc| {
                        let mut g2 = game.clone();
                        g2.sgdb_id.clear();
                        build_image_manager_content_with_drafts(s, &g2, win, pc).upcast()
                    });
                }
            });
            btn_box.append(&unmatch_btn);
        }

        content.append(&btn_box);
    }

    content
}

fn find_best_image_path(game: &Game, field: &str, _base: &str, id: &str, save_dir: &str) -> String {
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
    if game.kind == ira_models::GameKind::Retro {
        if let Some(f) = ira_parser::find_image_file(&ira_parser::retro_data_dir(save_dir, game.db_id), field) {
            return f.to_string_lossy().into_owned();
        }
    }
    let is_steam = game.trophy_source.has_steam_enrichment();
    if !is_steam && !game.sgdb_id.is_empty() {
        if let Some(f) = ira_parser::find_image_file(&ira_parser::sgdb_data_dir(save_dir, &game.sgdb_id), field) {
            return f.to_string_lossy().into_owned();
        }
    }
    let native_dir = if game.kind == ira_models::GameKind::Ps4 {
        ira_parser::ps4_data_dir(save_dir, id)
    } else if game.kind == ira_models::GameKind::Ps3 {
        ira_parser::ps3_data_dir(save_dir, id)
    } else {
        ira_parser::data_dir(save_dir, id)
    };
    if let Some(f) = ira_parser::find_image_file(&native_dir, field) {
        return f.to_string_lossy().into_owned();
    }
    if field == "icon" && game.kind.is_trophy_console() && !game.icon_path.is_empty() && std::path::Path::new(&game.icon_path).is_file() {
        return game.icon_path.clone();
    }
    String::new()
}

fn image_path_for_asset<'a>(game: &'a Game, asset: &'a str) -> &'a str {
    match asset {
        "icon" => &game.icon_path,
        "hero" => &game.hero_image_path,
        "grid" => &game.grid_path,
        "header" => &game.header_path,
        "logo" => &game.logo_path,
        _ => "",
    }
}

struct AssetRefreshCtx<'a> {
    cloud_dir: &'a std::path::Path,
    base_name: &'a str,
    asset_type: &'a str,
    thumb_size: (i32, i32),
}

fn make_refresh_closure(
    preview_wrapper: &gtk4::Box,
    ctx: AssetRefreshCtx,
    state: &SharedState,
    game: &Game,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
) -> Rc<dyn Fn()> {
    let asset_type = ctx.asset_type;
    let _s = tracing::info_span!("make_refresh_closure", asset = %asset_type, db_id = game.db_id).entered();
    let save_dir = state.borrow().save_dir.clone();
        let (tw, th) = ctx.thumb_size;
        Rc::new({
            let preview_wrapper = preview_wrapper.clone();
            let cloud_dir = ctx.cloud_dir.to_path_buf();
            let base_name = ctx.base_name.to_string();
            let state_clone = state.clone();
            let game_clone = game.clone();
            let pending_copies = pending_copies.clone();
            let asset_c = asset_type.to_string();
            move || {
                clear_children(&preview_wrapper);
                let from_pending = pending_copies.as_ref()
                    .and_then(|pc| pc.borrow().get(&asset_c).cloned())
                    .filter(|p| std::path::Path::new(p).is_file());
                let preview_src = from_pending.clone()
                    .or_else(|| {
                        ira_parser::find_image_file(&cloud_dir, &base_name)
                            .map(|p| p.to_string_lossy().into_owned())
                    });
                if let Some(path) = preview_src {
                    let p = gtk4::Picture::new();
                    ira_images::set_picture_contain_async(&p, &path, th.max(tw));
                    preview_wrapper.append(&p);
                } else {
                    let ph = gtk4::Label::new(Some("—"));
                    ph.add_css_class("dim-label");
                    ph.set_height_request(th.max(tw));
                    preview_wrapper.append(&ph);
                }
                if from_pending.is_some() {
                    return;
                }
            let s = state_clone.borrow();
            if let Ok(Some(entry)) = ira_db::find_by_db_id(&s.db, game_clone.db_id) {
                drop(s);
                if let Ok(updated) = crate::game_loader::load_game(&entry, &save_dir) {
                    let new_path = image_path_for_asset(&updated, &asset_c).to_string();
                    if !new_path.is_empty() {
                        ira_images::invalidate_texture(&new_path);
                    }
                    apply_game_update(&state_clone, updated);
                    let store = state_clone.borrow().grid_store.clone();
                    for i in 0..store.n_items() {
                        if let Some(item) = store.item(i).and_then(|o| o.downcast::<GameItem>().ok()) {
                            if item.game().is_some_and(|gi| gi.db_id == game_clone.db_id) {
                                let s = state_clone.borrow();
                                if let Some(g) = s.games.iter().find(|g| g.db_id == game_clone.db_id) {
                                    store.splice(i, 1, &[GameItem::new(g)]);
                                }
                                break;
                            }
                        }
                    }
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
    let is_steam = game.trophy_source.has_steam_enrichment();
    let id = game.app_id.clone();
    let save_dir = state.borrow().save_dir.clone();

    let cloud_dir = ira_parser::game_data_dir(&save_dir, game);
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

    let preview = gtk4::Picture::new();
    let max_h = thumb_h.max(thumb_w);
    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_valign(gtk4::Align::Center);
    if !img_path.is_empty() && std::path::Path::new(&img_path).is_file() {
        ira_images::set_picture_contain_async(&preview, &img_path, max_h);
        preview_wrapper.append(&preview);
    } else {
        let ph = gtk4::Label::new(Some("—"));
        ph.add_css_class("dim-label");
        ph.set_height_request(max_h);
        preview_wrapper.append(&ph);
    }
    row.append(&preview_wrapper);

    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btns.set_hexpand(true);
    btns.set_halign(gtk4::Align::End);

    let refresh_images = make_refresh_closure(
        &preview_wrapper,
        AssetRefreshCtx { cloud_dir: &cloud_dir, base_name: file_base, asset_type, thumb_size: (thumb_w, thumb_h) },
        state, game, pending_copies.clone(),
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
            let _s = tracing::info_span!("steam_button", app_id = %id_c, asset = %asset_c).entered();
            let _ = steam.force_download_steam(&id_c, &asset_c);
            refresh();
        });
        btns.append(&btn);
    }

    if is_steam && asset_type == "icon" && game.trophy_source == ira_models::TrophySource::SteamNative {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let save_dir_c = save_dir.clone();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            let _s = tracing::info_span!("steam_button_icon", app_id = %id_c).entered();
            if let Ok(app_id_num) = id_c.parse::<u32>() {
                if let Some(clienticon) = ira_platforms::steam::get_clienticon(app_id_num) {
                    let ico_file = ira_parser::data_dir(&save_dir_c, &id_c).join("icon.ico");
                    let _ = std::fs::create_dir_all(ico_file.parent().unwrap());
                    let webp_file = ico_file.with_extension("webp");
                    let ico_path = ira_platforms::steam::steam_install_dir()
                        .map(|d| d.join("steam").join("games").join(format!("{}.ico", clienticon)));
                    let have_local = ico_path.as_ref().is_some_and(|p| p.is_file());
                    if have_local {
                        if let Ok(ico_data) = std::fs::read(ico_path.as_ref().unwrap()) {
                            if std::fs::write(&ico_file, &ico_data).is_ok() {
                                ira_parser::convert_to_lossless_webp(&ico_file);
                            }
                        }
                    }
                    if !have_local || (!ico_file.is_file() && !webp_file.is_file()) {
                        let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", id_c, clienticon);
                        let _ = std::fs::remove_file(&ico_file);
                        if steam.download_file(&url, &ico_file).is_ok() {
                            ira_parser::convert_to_lossless_webp(&ico_file);
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

    if asset_type == "icon" && game.kind.is_trophy_console() {
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

/// Build a compact image section for a variant directory.
/// Images are saved directly to `target_dir` (no pending_copies).
/// Used inside variant cards' expandable "Manage images" section.
pub struct VariantImageSectionParams<'a> {
    pub target_dir: &'a std::path::Path,
    pub label: &'a str,
    pub file_base: &'a str,
    pub asset_type: &'a str,
    pub max_h: i32,
    pub state: &'a SharedState,
    pub entry: &'a ira_models::GameEntry,
    pub parent_win: &'a adw::Window,
}

pub fn build_image_section_for_dir(params: VariantImageSectionParams) -> adw::ActionRow {
    let VariantImageSectionParams { target_dir, label, file_base, asset_type, max_h, state, entry, parent_win } = params;
    let row = adw::ActionRow::new();
    row.set_title(label);

    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_valign(gtk4::Align::Center);

    let refresh_preview = {
        let target_dir = target_dir.to_path_buf();
        let preview_wrapper = preview_wrapper.clone();
        let file_base = file_base.to_string();
        Rc::new(move || {
            clear_children(&preview_wrapper);
            if let Some(p) = ira_parser::find_image_file(&target_dir, &file_base) {
                let pic = gtk4::Picture::new();
                ira_images::set_picture_contain_async(&pic, &p.to_string_lossy(), max_h);
                preview_wrapper.append(&pic);
            } else {
                let ph = gtk4::Label::new(Some("—"));
                ph.add_css_class("dim-label");
                ph.set_height_request(max_h);
                preview_wrapper.append(&ph);
            }
        })
    };
    refresh_preview();
    row.add_prefix(&preview_wrapper);

    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

    let browse_btn = make_browse_button(
        Some(parent_win),
        "Select image",
        false,
        Some(("Images", &["image/png", "image/jpeg", "image/webp", "image/x-icon"])),
        {
            let target_dir = target_dir.to_path_buf();
            let file_base = file_base.to_string();
            let refresh = refresh_preview.clone();
            move |path| {
                ira_parser::remove_image_variants(&target_dir, &file_base);
                ira_parser::remove_image_variants(&target_dir, &format!("{}_small", file_base));
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
                let dest = target_dir.join(format!("{}.{}", file_base, ext));
                let _ = std::fs::copy(path, &dest);
                ira_parser::convert_to_lossless_webp(&dest);
                let (sw, sh) = match file_base.as_str() {
                    "icon" => (32u32, 32u32),
                    "hero" => (1920, 620),
                    "vertical" => (300, 450),
                    "header" => (460, 215),
                    "logo" => (620, 620),
                    _ => (128, 128),
                };
                ira_parser::ensure_small_image(&target_dir, &file_base, sw, sh);
                refresh();
            }
        },
    );
    btns.append(&browse_btn);

    let sgdb_id = entry.sgdb_id.clone().unwrap_or_default();
    let is_steam = entry.trophy_source.has_steam_enrichment();
    let sgdb_id_for_picker = if !sgdb_id.is_empty() {
        sgdb_id.clone()
    } else if !entry.steam_id.is_empty() {
        entry.steam_id.clone()
    } else {
        entry.game_id.clone()
    };
    if !sgdb_id_for_picker.is_empty() {
        let btn = gtk4::Button::with_label("SGDB");
        btn.add_css_class("flat");
        let steam = state.borrow().steam.clone();
        let asset_c = asset_type.to_string();
        let parent = parent_win.clone();
        let refresh = refresh_preview.clone();
        let sgdb_id_c = sgdb_id_for_picker.clone();
        let save_dir = state.borrow().save_dir.clone();
        let sgdb_id_empty = sgdb_id.is_empty();
        btn.connect_clicked(move |_| {
            let on_done: Rc<dyn Fn()> = {
                let refresh = refresh.clone();
                Rc::new(move || {
                    refresh();
                })
            };
            show_sgdb_picker(ShowSgdbPickerParams {
                steam: &steam, id: &sgdb_id_c, asset: &asset_c,
                is_steam_id: is_steam && sgdb_id_empty,
                dimensions: &[],
                parent: &parent, on_done,
                pending_copies: None,
                save_dir: &save_dir,
            });
        });
        btns.append(&btn);
    }

    let reset_btn = gtk4::Button::with_label("Reset");
    reset_btn.add_css_class("flat");
    {
        let target_dir = target_dir.to_path_buf();
        let file_base = file_base.to_string();
        let refresh = refresh_preview.clone();
        reset_btn.connect_clicked(move |_| {
            ira_parser::remove_image_variants(&target_dir, &file_base);
            let small = format!("{}_small", file_base);
            ira_parser::remove_image_variants(&target_dir, &small);
            refresh();
        });
    }
    btns.append(&reset_btn);

    row.add_suffix(&btns);
    row
}