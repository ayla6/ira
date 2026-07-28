use gtk4::prelude::*;
use crate::Game;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use ira_models::AssetType;
use super::helpers::clear_children;
use super::message_helpers::apply_game_update;
use super::state::{PendingImage, SharedState};
use super::css::*;

pub(super) fn find_best_image_path(game: &Game, field: &str, _base: &str, _id: &str, save_dir: &str) -> String {
    let field_path = match AssetType::from_string(field) {
        Some(AssetType::Icon) if !game.icon_path.is_empty() => game.icon_path.clone(),
        Some(AssetType::Hero) if !game.hero_image_path.is_empty() => game.hero_image_path.clone(),
        Some(AssetType::Grid) if !game.grid_path.is_empty() => game.grid_path.clone(),
        Some(AssetType::Header) if !game.header_path.is_empty() => game.header_path.clone(),
        Some(AssetType::Logo) if !game.logo_path.is_empty() => game.logo_path.clone(),
        _ => String::new(),
    };
    if !field_path.is_empty() && std::path::Path::new(&field_path).is_file() {
        return field_path;
    }
    let native_dir = ira_parser::game_data_dir(save_dir, game);
    if let Some(f) = ira_parser::find_image_file(&native_dir, field) {
        return f.to_string_lossy().into_owned();
    }
    if AssetType::from_string(field) == Some(AssetType::Icon) && game.kind.is_trophy_console() && !game.icon_path.is_empty() && std::path::Path::new(&game.icon_path).is_file() {
        return game.icon_path.clone();
    }
    String::new()
}

fn image_path_for_asset<'a>(game: &'a Game, asset: &'a str) -> &'a str {
    match AssetType::from_string(asset) {
        Some(AssetType::Icon) => &game.icon_path,
        Some(AssetType::Hero) => &game.hero_image_path,
        Some(AssetType::Grid) => &game.grid_path,
        Some(AssetType::Header) => &game.header_path,
        Some(AssetType::Logo) => &game.logo_path,
        _ => "",
    }
}

pub(super) struct AssetRefreshCtx<'a> {
    pub(super) cloud_dir: &'a std::path::Path,
    pub(super) base_name: &'a str,
    pub(super) asset_type: &'a str,
    pub(super) thumb_size: (i32, i32),
}

pub(super) fn make_refresh_closure(
    preview_wrapper: &gtk4::Box,
    ctx: AssetRefreshCtx,
    state: &SharedState,
    game: &Game,
    pending_copies: Option<Rc<RefCell<HashMap<String, PendingImage>>>>,
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
                    .and_then(|pc| pc.borrow().get(&asset_c).cloned());
                let shown = match &from_pending {
                    Some(PendingImage::Path(p)) if std::path::Path::new(p).is_file() => {
                        let pic = gtk4::Picture::new();
                        ira_images::set_picture_contain_async(&pic, p, th.max(tw));
                        preview_wrapper.append(&pic);
                        true
                    }
                    Some(PendingImage::Bytes(b)) if !b.is_empty() => {
                        if let Ok(texture) = gdk4::Texture::from_bytes(&glib::Bytes::from_owned(b.clone())) {
                            let pic = gtk4::Picture::for_paintable(&texture);
                            pic.set_content_fit(gtk4::ContentFit::Contain);
                            pic.set_height_request(th.max(tw));
                            preview_wrapper.append(&pic);
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if !shown {
                    if let Some(p) = ira_parser::find_image_file(&cloud_dir, &base_name) {
                        let pic = gtk4::Picture::new();
                        ira_images::set_picture_contain_async(&pic, &p.to_string_lossy(), th.max(tw));
                        preview_wrapper.append(&pic);
                    } else {
                        let ph = gtk4::Label::new(Some("—"));
                        ph.add_css_class(CSS_DIM_LABEL);
                        ph.set_height_request(th.max(tw));
                        preview_wrapper.append(&ph);
                    }
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
                }
            }
        }
    })
}
