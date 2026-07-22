use gtk4::prelude::*;
use adw::prelude::*;
use crate::Game;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use ira_models::AssetType;
use super::helpers::{clear_children, make_browse_button, refresh_settings_images_page};
use super::image_manager_helpers::{find_best_image_path, AssetRefreshCtx, make_refresh_closure};
use super::sgdb_match_dialog::show_sgdb_search_dialog;
use super::sgdb_picker::{show_sgdb_picker, ShowSgdbPickerParams};
use super::state::SharedState;

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

    for &at in AssetType::all() {
        let (thumb_w, thumb_h) = match at {
            AssetType::Icon => (48, 48),
            AssetType::Hero => (96, 64),
            AssetType::Grid => (48, 64),
            AssetType::Header => (96, 48),
            AssetType::Logo => (64, 64),
        };
        let section = build_image_section(BuildImageSectionParams {
            label: at.display_name(), file_base: at.file_base(), asset_type: at.as_str(),
            thumb_w, thumb_h, dims: at.sgdb_dimensions(),
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

    if is_steam && AssetType::from_string(asset_type) != Some(AssetType::Icon) {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let asset_c = asset_type.to_string();
        let refresh = refresh_images.clone();
        let btn_clone = btn.clone();
        btn.connect_clicked(move |_| {
            btn_clone.set_sensitive(false);
            btn_clone.set_label("Downloading…");
            let steam = steam.clone();
            let id_c = id_c.clone();
            let asset_c = asset_c.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let rx = std::cell::RefCell::new(rx);
            let asset_at = AssetType::from_string(&asset_c).unwrap_or(AssetType::Icon);
            std::thread::spawn(move || {
                let _s = tracing::info_span!("steam_download", app_id = %id_c, asset = %asset_c).entered();
                let _ = steam.force_download_steam(&id_c, asset_at);
                let _ = tx.send(());
            });
            let btn_weak = btn_clone.downgrade();
            let refresh = refresh.clone();
            glib::source::idle_add_local_full(glib::Priority::LOW, move || {
                if rx.borrow_mut().try_recv().is_ok() {
                    if let Some(btn) = btn_weak.upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label("Steam");
                    }
                    refresh();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        });
        btns.append(&btn);
    }

    if is_steam && AssetType::from_string(asset_type) == Some(AssetType::Icon) && game.trophy_source == ira_models::TrophySource::SteamNative {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let save_dir_c = save_dir.clone();
        let refresh = refresh_images.clone();
        let btn_clone = btn.clone();
        btn.connect_clicked(move |_| {
            btn_clone.set_sensitive(false);
            btn_clone.set_label("Downloading…");
            let steam = steam.clone();
            let id_c = id_c.clone();
            let save_dir_c = save_dir_c.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let rx = std::cell::RefCell::new(rx);
            std::thread::spawn(move || {
                let _s = tracing::info_span!("steam_icon_download", app_id = %id_c).entered();
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
                let _ = tx.send(());
            });
            let btn_weak = btn_clone.downgrade();
            let refresh = refresh.clone();
            glib::source::idle_add_local_full(glib::Priority::LOW, move || {
                if rx.borrow_mut().try_recv().is_ok() {
                    if let Some(btn) = btn_weak.upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label("Steam");
                    }
                    refresh();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
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

    if AssetType::from_string(asset_type) == Some(AssetType::Icon) && game.kind.is_trophy_console() {
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
    pub dimensions: &'static [&'static str],
    pub max_h: i32,
    pub state: &'a SharedState,
    pub entry: &'a ira_models::GameEntry,
    pub parent_win: &'a adw::Window,
}

pub fn build_image_section_for_dir(params: VariantImageSectionParams) -> adw::ActionRow {
    let VariantImageSectionParams { target_dir, label, file_base, asset_type, dimensions, max_h, state, entry, parent_win } = params;
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
                let (sw, sh) = match AssetType::all().iter().find(|at| at.file_base() == file_base.as_str()) {
                    Some(at) => at.thumb_dims(),
                    None => (128, 128),
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
                dimensions,
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