use super::css::*;
use super::helpers::{clear_children, make_browse_button, refresh_settings_images_page};
use super::image_manager_helpers::{find_best_image_path, make_refresh_closure, AssetRefreshCtx};
use super::sgdb_match_dialog::show_sgdb_search_dialog;
use super::sgdb_picker::{show_sgdb_picker, ShowSgdbPickerParams};
use super::state::{PendingImage, SharedState};
use crate::Game;
use adw::prelude::*;
use ira_models::AssetType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn build_image_manager_content(
    state: &SharedState,
    game: &Game,
    parent_win: &adw::Window,
) -> gtk4::Box {
    build_image_manager_content_with_drafts(state, game, parent_win, None)
}

pub fn build_image_manager_content_with_drafts(
    state: &SharedState,
    game: &Game,
    parent_win: &adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
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
            label: at.display_name(),
            file_base: at.file_base(),
            asset_type: at.as_str(),
            thumb_w,
            thumb_h,
            dims: at.sgdb_dimensions(),
            game,
            state,
            parent_win,
            pending_copies: pending_copies.clone(),
        });
        content.append(&section);
    }

    {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        btn_box.set_halign(gtk4::Align::Center);
        btn_box.set_margin_top(24);

        if game.sgdb_id.is_empty() && !is_steam {
            let match_btn = gtk4::Button::with_label(&crate::tr!("Match to SteamGridDB…"));
            match_btn.add_css_class(CSS_SUGGESTED_ACTION);
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
            let label = gtk4::Label::new(Some(&crate::tr!("Matched (SGDB ID: {})").replacen(
                "{}",
                &game.sgdb_id,
                1,
            )));
            label.add_css_class(CSS_SUCCESS_LABEL);
            btn_box.append(&label);
            let unmatch_btn = gtk4::Button::with_label(&crate::tr!("Unmatch SGDB"));
            unmatch_btn.add_css_class(CSS_DESTRUCTIVE_ACTION);
            let pending_pc = pending_copies.clone();
            let sc = state.clone();
            let did = game.db_id;
            unmatch_btn.connect_clicked(move |_| {
                if let Some(ref pc) = pending_pc {
                    pc.borrow_mut()
                        .insert("__unmatch__".to_string(), PendingImage::Path(String::new()));
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
    pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
}

fn resolve_image_source(
    pending_copies: &Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    asset_type: &str,
    game: &Game,
    file_base: &str,
    id: &str,
    save_dir: &str,
) -> Option<PendingImage> {
    if let Some(pc) = pending_copies {
        if let Some(img) = pc.borrow().get(asset_type).cloned() {
            match &img {
                PendingImage::Path(p) if std::path::Path::new(p).is_file() => return Some(img),
                PendingImage::Path(_) => {}
                PendingImage::Bytes(_) => return Some(img),
            }
        }
    }
    let path = find_best_image_path(game, asset_type, file_base, id, save_dir);
    if path.is_empty() {
        None
    } else {
        Some(PendingImage::Path(path))
    }
}

fn build_image_preview(source: Option<&PendingImage>, max_h: i32) -> gtk4::Box {
    let preview = gtk4::Picture::new();
    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_valign(gtk4::Align::Center);
    let has_image = match source {
        Some(PendingImage::Path(p)) if !p.is_empty() && std::path::Path::new(p).is_file() => {
            ira_images::set_picture_contain_async(&preview, p, max_h);
            true
        }
        Some(PendingImage::Bytes(b)) if !b.is_empty() => {
            if let Ok(texture) = gdk4::Texture::from_bytes(&glib::Bytes::from_owned(b.clone())) {
                preview.set_paintable(Some(&texture));
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if has_image {
        preview_wrapper.append(&preview);
    } else {
        let ph = gtk4::Label::new(Some("—"));
        ph.add_css_class(CSS_DIM_LABEL);
        ph.set_height_request(max_h);
        preview_wrapper.append(&ph);
    }
    preview_wrapper
}

fn build_steam_non_icon_button(
    is_steam: bool,
    asset_type: &str,
    state: &SharedState,
    id: &str,
    refresh_images: &Rc<dyn Fn()>,
) -> Option<gtk4::Button> {
    if !is_steam || AssetType::from_string(asset_type) == Some(AssetType::Icon) {
        return None;
    }
    let btn = gtk4::Button::with_label(&crate::tr!("Steam"));
    let steam = state.borrow().steam.clone();
    let id_c = id.to_string();
    let asset_c = asset_type.to_string();
    let refresh = Rc::clone(refresh_images);
    let btn_clone = btn.clone();
    btn.connect_clicked(move |_| {
        btn_clone.set_sensitive(false);
        btn_clone.set_label(&crate::tr!("Downloading…"));
        let steam = steam.clone();
        let id_c = id_c.clone();
        let asset_c = asset_c.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::cell::RefCell::new(rx);
        let asset_at = AssetType::from_string(&asset_c).unwrap_or(AssetType::Icon);
        std::thread::spawn(move || {
            let _s =
                tracing::info_span!("steam_download", app_id = %id_c, asset = %asset_c).entered();
            let _ = steam.force_download_steam(&id_c, asset_at);
            let _ = tx.send(());
        });
        let btn_weak = btn_clone.downgrade();
        let refresh = refresh.clone();
        glib::source::idle_add_local_full(glib::Priority::LOW, move || {
            if rx.borrow_mut().try_recv().is_ok() {
                if let Some(btn) = btn_weak.upgrade() {
                    btn.set_sensitive(true);
                    btn.set_label(&crate::tr!("Steam"));
                }
                refresh();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
    Some(btn)
}

fn build_steam_icon_button(
    is_steam: bool,
    asset_type: &str,
    _trophy_source: ira_models::TrophySource,
    state: &SharedState,
    id: &str,
    save_dir: &str,
    refresh_images: &Rc<dyn Fn()>,
) -> Option<gtk4::Button> {
    if !is_steam || AssetType::from_string(asset_type) != Some(AssetType::Icon) {
        return None;
    }
    let btn = gtk4::Button::with_label(&crate::tr!("Steam"));
    let steam = state.borrow().steam.clone();
    let id_c = id.to_string();
    let save_dir_c = save_dir.to_string();
    let refresh = Rc::clone(refresh_images);
    let btn_clone = btn.clone();
    btn.connect_clicked(move |_| {
        btn_clone.set_sensitive(false);
        btn_clone.set_label(&crate::tr!("Downloading…"));
        let steam = steam.clone();
        let id_c = id_c.clone();
        let save_dir_c = save_dir_c.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::cell::RefCell::new(rx);
        std::thread::spawn(move || {
            let _s = tracing::info_span!("steam_icon_download", app_id = %id_c).entered();
            let clienticon = steam.cached_clienticon(&id_c)
                .or_else(|| {
                    let app_id_num: u32 = id_c.parse().ok()?;
                    ira_platforms::steam::get_clienticon(app_id_num)
                });
            if let Some(clienticon) = clienticon {
                let dest_webp = ira_parser::data_dir(&save_dir_c, &id_c).join("icon.webp");
                if !dest_webp.is_file() {
                    if let Some(parent) = dest_webp.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let ico_bytes = {
                        let ico_path = ira_platforms::steam::steam_install_dir()
                            .map(|d| d.join("steam").join("games").join(format!("{}.ico", clienticon)));
                        ico_path.as_ref().and_then(|p| {
                            if p.is_file() { std::fs::read(p).ok() } else { None }
                        })
                    };
                    let ico_bytes = ico_bytes.unwrap_or_else(|| {
                        let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", id_c, clienticon);
                        steam.download_bytes(&url).ok().unwrap_or_default()
                    });
                    if let Some(webp) = ira_parser::convert_bytes_to_lossless_webp(&ico_bytes) {
                        let _ = std::fs::write(&dest_webp, &webp);
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
                    btn.set_label(&crate::tr!("Steam"));
                }
                refresh();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
    Some(btn)
}

struct SgdbPickerCtx<'a> {
    state: &'a SharedState,
    asset_type: &'a str,
    parent_win: &'a adw::Window,
    refresh_images: &'a Rc<dyn Fn()>,
    dims: &'a [&'static str],
    pending_copies: &'a Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    save_dir: &'a str,
}

fn build_sgdb_picker_button(
    game: &Game,
    is_steam: bool,
    id: &str,
    ctx: &SgdbPickerCtx,
) -> Option<gtk4::Button> {
    let sgdb_id_for_picker = if !game.sgdb_id.is_empty() {
        game.sgdb_id.clone()
    } else {
        id.to_string()
    };
    let sgdb_is_steam_id = is_steam && game.sgdb_id.is_empty();
    let pending_copies_btn = ctx.pending_copies.clone();
    if sgdb_id_for_picker.is_empty() {
        return None;
    }
    let btn = gtk4::Button::with_label(&crate::tr!("SGDB…"));
    let steam = ctx.state.borrow().steam.clone();
    let asset_c = ctx.asset_type.to_string();
    let parent = ctx.parent_win.clone();
    let refresh = Rc::clone(ctx.refresh_images);
    let dims_vec: Vec<&str> = ctx.dims.to_vec();
    let sgdb_id_c = sgdb_id_for_picker.clone();
    let save_dir_c = ctx.save_dir.to_string();
    let dest_dir = ira_parser::game_data_dir(&save_dir_c, game)
        .to_string_lossy()
        .into_owned();
    btn.connect_clicked(move |_| {
        show_sgdb_picker(ShowSgdbPickerParams {
            steam: &steam,
            id: &sgdb_id_c,
            asset: &asset_c,
            is_steam_id: sgdb_is_steam_id,
            dimensions: &dims_vec,
            parent: &parent,
            on_done: refresh.clone(),
            pending_copies: pending_copies_btn.clone(),
            save_dir: &save_dir_c,
            dest_dir: Some(&dest_dir),
        });
    });
    Some(btn)
}

fn build_reset_icon_button(
    asset_type: &str,
    game: &Game,
    refresh_images: &Rc<dyn Fn()>,
    pending_copies: &Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
    save_dir: &str,
) -> Option<gtk4::Button> {
    if AssetType::from_string(asset_type) != Some(AssetType::Icon) || !game.kind.is_trophy_console()
    {
        return None;
    }
    let label = match game.kind {
        ira_models::GameKind::Ps4 => crate::tr!("PS4"),
        ira_models::GameKind::Ps3 => crate::tr!("PS3"),
        _ => crate::tr!("Use icon"),
    };
    let reset_btn = gtk4::Button::with_label(&label);
    let gc = game.clone();
    let refresh = Rc::clone(refresh_images);
    let pending_copies_reset = pending_copies.clone();
    let asset_reset = asset_type.to_string();
    let save_dir_c2 = save_dir.to_string();
    reset_btn.connect_clicked(move |_| {
        let app_id = gc.app_id.clone();
        let game_path = gc.game_path.clone();
        let kind = gc.kind;
        let image_dir = match kind {
            ira_models::GameKind::Ps4 => std::path::Path::new(&save_dir_c2)
                .join("data")
                .join("ps4")
                .join(&app_id),
            ira_models::GameKind::Ps3 => std::path::Path::new(&save_dir_c2)
                .join("data")
                .join("ps3")
                .join(&app_id),
            _ => return,
        };
        let game_icon = match kind {
            ira_models::GameKind::Ps4 => std::path::Path::new(&game_path)
                .join("sce_sys")
                .join("icon0.png"),
            ira_models::GameKind::Ps3 => std::path::Path::new(&game_path).join("ICON0.PNG"),
            _ => return,
        };
        if !game_icon.is_file() {
            return;
        }
        let _ = std::fs::create_dir_all(&image_dir);
        ira_parser::remove_image_variants(&image_dir, "icon");
        ira_parser::remove_image_variants(&image_dir, "icon_small");
        let tmp_png = image_dir.join("icon.png");
        if std::fs::copy(&game_icon, &tmp_png).is_ok() {
            ira_parser::convert_to_lossless_webp(&tmp_png);
            ira_parser::ensure_small_image(
                &image_dir,
                "icon",
                AssetType::Icon.thumb_dims().0,
                AssetType::Icon.thumb_dims().1,
            );
            if let Some(p) = ira_parser::find_image_file(&image_dir, "icon") {
                ira_images::invalidate_texture(&p.to_string_lossy());
            }
            if let Some(ref pc) = pending_copies_reset {
                pc.borrow_mut().remove(&asset_reset);
            }
        }
        refresh();
    });
    Some(reset_btn)
}

fn build_ra_icon_button(
    asset_type: &str,
    game: &Game,
    state: &SharedState,
    refresh_images: &Rc<dyn Fn()>,
    pending_copies: &Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
) -> Option<gtk4::Button> {
    if AssetType::from_string(asset_type) != Some(AssetType::Icon)
        || game.kind != ira_models::GameKind::Retro
        || game.trophy_source != ira_models::TrophySource::Ra
    {
        return None;
    }
    let btn = gtk4::Button::with_label(&crate::tr!("RA icon"));
    let db_id = game.db_id;
    let app_id = game.app_id.clone();
    let save_dir = state.borrow().save_dir.clone();
    let ra_username = state.borrow().cfg.ra_username.clone();
    let ra_token = state.borrow().cfg.ra_token.clone();
    let ra_password = state.borrow().cfg.ra_password.clone();
    let refresh = Rc::clone(refresh_images);
    let pending_copies_ra = pending_copies.clone();
    let asset_ra = asset_type.to_string();
    let btn_clone = btn.clone();
    btn.connect_clicked(move |_| {
        btn_clone.set_sensitive(false);
        btn_clone.set_label(&crate::tr!("Downloading…"));
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        let rx = std::cell::RefCell::new(rx);
        let ra_username = ra_username.clone();
        let ra_token = ra_token.clone();
        let ra_password = ra_password.clone();
        let app_id = app_id.clone();
        let save_dir = save_dir.clone();
        let db_id = db_id;
        std::thread::spawn(move || {
            let _s = tracing::info_span!("ra_icon_download", db_id).entered();
            if ira_platforms::retroachievements::RaClient::auth_is_broken() {
                let _ = tx.send(None);
                return;
            }
            let client = ira_platforms::retroachievements::RaClient::new(
                &ra_username,
                &ra_token,
                &ra_password,
            );
            match client.fetch_game_data(&save_dir, &app_id) {
                Ok(game_data) if !game_data.image_icon.is_empty() => {
                    let icon = client.download_game_icon(&save_dir, db_id, &game_data.image_icon);
                    let _ = tx.send(if icon.is_empty() { None } else { Some(icon) });
                }
                _ => {
                    let _ = tx.send(None);
                }
            }
        });
        let btn_weak = btn_clone.downgrade();
        let refresh = refresh.clone();
        let pc = pending_copies_ra.clone();
        let asset = asset_ra.clone();
        glib::source::idle_add_local_full(glib::Priority::LOW, move || {
            if let Ok(result) = rx.borrow_mut().try_recv() {
                if let Some(btn) = btn_weak.upgrade() {
                    btn.set_sensitive(true);
                    btn.set_label(&crate::tr!("RA icon"));
                }
                if let Some(path) = result {
                    if let Some(ref pc) = pc {
                        pc.borrow_mut()
                            .insert(asset.clone(), PendingImage::Path(path));
                    }
                    refresh();
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
    Some(btn)
}

fn build_image_section(params: BuildImageSectionParams) -> gtk4::Box {
    let BuildImageSectionParams {
        label,
        file_base,
        asset_type,
        thumb_w,
        thumb_h,
        dims,
        game,
        state,
        parent_win,
        pending_copies,
    } = params;
    let is_steam = game.trophy_source.has_steam_enrichment();
    let id = game.app_id.clone();
    let save_dir = state.borrow().save_dir.clone();

    let cloud_dir = ira_parser::game_data_dir(&save_dir, game);
    let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let lbl = gtk4::Label::new(Some(label));
    lbl.set_halign(gtk4::Align::Start);
    lbl.add_css_class(CSS_HEADING);
    section.append(&lbl);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_hexpand(true);
    row.set_valign(gtk4::Align::Center);

    let img_source =
        resolve_image_source(&pending_copies, asset_type, game, file_base, &id, &save_dir);
    let max_h = thumb_h.max(thumb_w);
    let preview_wrapper = build_image_preview(img_source.as_ref(), max_h);
    row.append(&preview_wrapper);

    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btns.set_hexpand(true);
    btns.set_halign(gtk4::Align::End);
    btns.set_valign(gtk4::Align::Center);
    btns.set_vexpand(false);

    let refresh_images = make_refresh_closure(
        &preview_wrapper,
        AssetRefreshCtx {
            cloud_dir: &cloud_dir,
            base_name: file_base,
            asset_type,
            thumb_size: (thumb_w, thumb_h),
        },
        state,
        game,
        pending_copies.clone(),
    );

    let browse_btn = make_browse_button(
        Some(parent_win),
        &crate::tr!("Select image"),
        false,
        Some((
            &crate::tr!("Images"),
            &["image/png", "image/jpeg", "image/webp", "image/x-icon"],
        )),
        || None,
        {
            let pc = pending_copies.clone();
            let refresh = Rc::clone(&refresh_images);
            let asset_name = asset_type.to_string();
            move |path| {
                if let Some(ref pc_inner) = pc {
                    pc_inner.borrow_mut().insert(
                        asset_name.clone(),
                        PendingImage::Path(path.to_string_lossy().into_owned()),
                    );
                    refresh();
                }
            }
        },
    );
    btns.append(&browse_btn);

    if let Some(btn) =
        build_steam_non_icon_button(is_steam, asset_type, state, &id, &refresh_images)
    {
        btns.append(&btn);
    }

    if let Some(btn) = build_steam_icon_button(
        is_steam,
        asset_type,
        game.trophy_source,
        state,
        &id,
        &save_dir,
        &refresh_images,
    ) {
        btns.append(&btn);
    }

    if let Some(btn) = build_reset_icon_button(
        asset_type,
        game,
        &refresh_images,
        &pending_copies,
        &save_dir,
    ) {
        btns.append(&btn);
    }

    let sgdb_ctx = SgdbPickerCtx {
        state,
        asset_type,
        parent_win,
        refresh_images: &refresh_images,
        dims,
        pending_copies: &pending_copies,
        save_dir: &save_dir,
    };
    if let Some(btn) = build_sgdb_picker_button(game, is_steam, &id, &sgdb_ctx) {
        btns.append(&btn);
    }

    if let Some(btn) =
        build_ra_icon_button(asset_type, game, state, &refresh_images, &pending_copies)
    {
        btns.append(&btn);
    }

    row.append(&btns);
    section.append(&row);
    section
}

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

fn setup_dir_preview(
    target_dir: &std::path::Path,
    file_base: &str,
    max_h: i32,
    preview_wrapper: &gtk4::Box,
) -> Rc<dyn Fn()> {
    let target_dir = target_dir.to_path_buf();
    let preview_wrapper = preview_wrapper.clone();
    let file_base = file_base.to_string();
    let refresh_preview: Rc<dyn Fn()> = Rc::new(move || {
        clear_children(&preview_wrapper);
        if let Some(p) = ira_parser::find_image_file(&target_dir, &file_base) {
            let pic = gtk4::Picture::new();
            ira_images::set_picture_contain_async(&pic, &p.to_string_lossy(), max_h);
            preview_wrapper.append(&pic);
        } else {
            let ph = gtk4::Label::new(Some("—"));
            ph.add_css_class(CSS_DIM_LABEL);
            ph.set_height_request(max_h);
            preview_wrapper.append(&ph);
        }
    });
    refresh_preview();
    refresh_preview
}

struct DirButtonsCtx<'a> {
    state: &'a SharedState,
    asset_type: &'a str,
    dimensions: &'static [&'static str],
    refresh_preview: &'a Rc<dyn Fn()>,
}

fn build_dir_buttons(
    parent_win: &adw::Window,
    target_dir: &std::path::Path,
    file_base: &str,
    entry: &ira_models::GameEntry,
    ctx: &DirButtonsCtx,
) -> gtk4::Box {
    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btns.set_valign(gtk4::Align::Center);
    btns.set_vexpand(false);

    let browse_btn = make_browse_button(
        Some(parent_win),
        &crate::tr!("Select image"),
        false,
        Some((
            &crate::tr!("Images"),
            &["image/png", "image/jpeg", "image/webp", "image/x-icon"],
        )),
        || None,
        {
            let target_dir = target_dir.to_path_buf();
            let file_base = file_base.to_string();
            let refresh = Rc::clone(ctx.refresh_preview);
            move |path| {
                ira_parser::remove_image_variants(&target_dir, &file_base);
                ira_parser::remove_image_variants(&target_dir, &format!("{}_small", file_base));
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
                let dest = target_dir.join(format!("{}.{}", file_base, ext));
                let _ = std::fs::copy(path, &dest);
                ira_parser::convert_to_lossless_webp(&dest);
                let (sw, sh) = match AssetType::all()
                    .iter()
                    .find(|at| at.file_base() == file_base.as_str())
                {
                    Some(at) => at.thumb_dims(),
                    None => (128, 128),
                };
                ira_parser::ensure_small_image(&target_dir, &file_base, sw, sh);
                let webp = target_dir.join(format!("{}.webp", file_base));
                if webp.is_file() {
                    ira_images::invalidate_texture(&webp.to_string_lossy());
                }
                let small = target_dir.join(format!("{}_small.webp", file_base));
                if small.is_file() {
                    ira_images::invalidate_texture(&small.to_string_lossy());
                }
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
        let btn = gtk4::Button::with_label(&crate::tr!("SGDB"));
        btn.add_css_class(CSS_FLAT);
        let steam = ctx.state.borrow().steam.clone();
        let asset_c = ctx.asset_type.to_string();
        let parent = parent_win.clone();
        let refresh = Rc::clone(ctx.refresh_preview);
        let sgdb_id_c = sgdb_id_for_picker.clone();
        let save_dir = ctx.state.borrow().save_dir.clone();
        let sgdb_id_empty = sgdb_id.is_empty();
        let dims_vec: Vec<&str> = ctx.dimensions.to_vec();
        let target_dir_c = target_dir.to_string_lossy().into_owned();
        btn.connect_clicked(move |_| {
            let on_done: Rc<dyn Fn()> = {
                let refresh = refresh.clone();
                Rc::new(move || {
                    refresh();
                })
            };
            show_sgdb_picker(ShowSgdbPickerParams {
                steam: &steam,
                id: &sgdb_id_c,
                asset: &asset_c,
                is_steam_id: is_steam && sgdb_id_empty,
                dimensions: &dims_vec,
                parent: &parent,
                on_done,
                pending_copies: None,
                save_dir: &save_dir,
                dest_dir: Some(&target_dir_c),
            });
        });
        btns.append(&btn);
    }

    let reset_btn = gtk4::Button::with_label(&crate::tr!("Reset"));
    reset_btn.add_css_class(CSS_FLAT);
    {
        let target_dir = target_dir.to_path_buf();
        let file_base = file_base.to_string();
        let refresh = Rc::clone(ctx.refresh_preview);
        reset_btn.connect_clicked(move |_| {
            ira_parser::remove_image_variants(&target_dir, &file_base);
            let small = format!("{}_small", file_base);
            ira_parser::remove_image_variants(&target_dir, &small);
            refresh();
        });
    }
    btns.append(&reset_btn);

    btns
}

pub fn build_image_section_for_dir(params: VariantImageSectionParams) -> adw::ActionRow {
    let VariantImageSectionParams {
        target_dir,
        label,
        file_base,
        asset_type,
        dimensions,
        max_h,
        state,
        entry,
        parent_win,
    } = params;
    let row = adw::ActionRow::new();
    row.set_title(label);

    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_valign(gtk4::Align::Center);

    let refresh_preview = setup_dir_preview(target_dir, file_base, max_h, &preview_wrapper);
    row.add_prefix(&preview_wrapper);

    let dir_ctx = DirButtonsCtx {
        state,
        asset_type,
        dimensions,
        refresh_preview: &refresh_preview,
    };
    let btns = build_dir_buttons(parent_win, target_dir, file_base, entry, &dir_ctx);
    row.add_suffix(&btns);
    row
}
