use super::css::*;
use super::helpers::clear_children;
use super::message_helpers::apply_game_update;
use super::state::{PendingImage, SharedState};
use crate::Game;
use gtk4::prelude::*;
use ira_models::AssetType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub(super) fn find_best_image_path(
    game: &Game,
    field: &str,
    _base: &str,
    _id: &str,
    save_dir: &str,
) -> String {
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
    if AssetType::from_string(field) == Some(AssetType::Icon)
        && game.kind.is_trophy_console()
        && !game.icon_path.is_empty()
        && std::path::Path::new(&game.icon_path).is_file()
    {
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
    let _s = tracing::info_span!("make_refresh_closure", asset = %asset_type, db_id = game.db_id)
        .entered();
    let save_dir = state.borrow().save_dir.clone();
    let (tw, th) = ctx.thumb_size;
    let db_id = game.db_id;
    Rc::new({
        let preview_wrapper = preview_wrapper.downgrade();
        let cloud_dir = ctx.cloud_dir.to_path_buf();
        let base_name = ctx.base_name.to_string();
        let state_w = Rc::downgrade(state);
        let pending_copies = pending_copies.as_ref().map(Rc::downgrade);
        let asset_c = asset_type.to_string();
        move || {
            let Some(preview_wrapper) = preview_wrapper.upgrade() else {
                return;
            };
            clear_children(&preview_wrapper);
            let from_pending = pending_copies
                .as_ref()
                .and_then(|pc| pc.upgrade())
                .and_then(|pc| pc.borrow().get(&asset_c).cloned());
            let shown = match &from_pending {
                Some(PendingImage::Path(p)) if std::path::Path::new(p).is_file() => {
                    let pic = gtk4::Picture::new();
                    ira_images::set_picture_contain_async(&pic, p, th.max(tw));
                    preview_wrapper.append(&pic);
                    true
                }
                Some(PendingImage::Bytes(b)) if !b.is_empty() => {
                    if let Ok(texture) = gdk4::Texture::from_bytes(b) {
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
            let Some(state) = state_w.upgrade() else {
                return;
            };
            let s = state.borrow();
            if let Ok(Some(entry)) = ira_db::find_by_db_id(&s.db, db_id) {
                drop(s);
                if let Ok(updated) = crate::game_loader::load_game(&entry, &save_dir) {
                    let new_path = image_path_for_asset(&updated, &asset_c).to_string();
                    if !new_path.is_empty() {
                        ira_images::invalidate_texture(&new_path);
                    }
                    apply_game_update(&state, updated);
                }
            }
        }
    })
}

/// Decodes an emulator-native icon into lossless WebP bytes without
/// touching anything on disk, so the result can be staged as a pending
/// image and only applied when the settings Save button is pressed.
/// 3DS and Wii U titles whose stored location vanished get relocated
/// through the emulator's own configuration.
pub(super) fn native_icon_bytes(
    game: &Game,
    cfg: &ira_config::Config,
    azahar_executable: &str,
    cemu_executable: &str,
    switch_executable: &str,
) -> Option<Vec<u8>> {
    // Resolve ROM path: ROM-library games store it relative to one of the
    // ROM roots (e.g. "game.nds" or "game.zip"), while console games store
    // an absolute path.
    let game_root_owned;
    let game_root: &std::path::Path =
        if matches!(
            (game.kind, game.platform_id.as_str()),
            (ira_models::GameKind::Retro, "nds")
                | (ira_models::GameKind::Switch, _)
        ) {
            let p = std::path::Path::new(&game.game_path);
            if p.is_absolute() {
                p
            } else {
                game_root_owned = cfg.resolve_rom_path(&game.platform_id, &game.game_path)?;
                &game_root_owned
            }
        } else {
            std::path::Path::new(&game.game_path)
        };
    match game.kind {
        ira_models::GameKind::Retro if game.platform_id == "nds" => decode_nds_icon(game_root),
        ira_models::GameKind::Switch => decode_switch_icon(game_root, switch_executable),
        ira_models::GameKind::ThreeDS => decode_smdh_icon(game_root).or_else(|| {
            ira_platforms::azahar::find_title_path_for(azahar_executable, &game.platform_id)
                .and_then(|path| decode_smdh_icon(&path))
        }),
        ira_models::GameKind::Ps4 => {
            import_image_bytes(&game_root.join("sce_sys").join("icon0.png"))
        }
        ira_models::GameKind::Ps3 => import_image_bytes(&game_root.join("ICON0.PNG")),
        ira_models::GameKind::WiiU => {
            if game_root.join("meta/iconTex.tga").is_file() {
                import_image_bytes(&game_root.join("meta").join("iconTex.tga"))
            } else {
                ira_platforms::cemu::find_title_dir(cemu_executable, &game.platform_id)
                    .map(|dir| dir.join("meta").join("iconTex.tga"))
                    .filter(|icon| icon.is_file())
                    .and_then(|icon| import_image_bytes(&icon))
            }
        }
        _ => None,
    }
}

/// Writes a rendered native icon into the game's data dir, replacing any
/// existing one. Only used when there is no settings dialog to Save from;
/// the dialog stages instead and lets the Save flow apply it.
pub(super) fn write_native_icon_to_disk(save_dir: &str, game: &Game, webp_bytes: &[u8]) -> bool {
    let image_dir = match game.kind {
        ira_models::GameKind::Ps4 => std::path::Path::new(save_dir)
            .join("data")
            .join("ps4")
            .join(&game.app_id),
        ira_models::GameKind::Ps3 => std::path::Path::new(save_dir)
            .join("data")
            .join("ps3")
            .join(&game.app_id),
        ira_models::GameKind::ThreeDS => ira_parser::three_ds_data_dir(save_dir, &game.app_id),
        ira_models::GameKind::WiiU => ira_parser::wiiu_data_dir(save_dir, &game.app_id),
        ira_models::GameKind::Switch => ira_parser::switch_data_dir(save_dir, game.db_id),
        ira_models::GameKind::Retro => ira_parser::retro_data_dir(save_dir, game.db_id),
        _ => return false,
    };
    let _ = std::fs::create_dir_all(&image_dir);
    ira_parser::remove_image_variants(&image_dir, "icon");
    ira_parser::remove_image_variants(&image_dir, "icon_small");
    if std::fs::write(image_dir.join("icon.webp"), webp_bytes).is_err() {
        return false;
    }
    ira_parser::ensure_small_image(
        &image_dir,
        "icon",
        ira_models::AssetType::Icon.thumb_dims().0,
        ira_models::AssetType::Icon.thumb_dims().1,
    );
    if let Some(p) = ira_parser::find_image_file(&image_dir, "icon") {
        ira_images::invalidate_texture(&p.to_string_lossy());
    }
    true
}

/// Decodes an icon file into lossless WebP bytes.
fn import_image_bytes(source: &std::path::Path) -> Option<Vec<u8>> {
    let data = std::fs::read(source).ok()?;
    ira_parser::load_image_bytes(&data)?;
    ira_parser::convert_bytes_to_lossless_webp(&data)
}

/// Renders the SMDH icon of a 3DS ROM or installed title into lossless
/// WebP bytes.
fn decode_smdh_icon(game_root: &std::path::Path) -> Option<Vec<u8>> {
    let icon = ira_platforms::azahar::read_icon(game_root)?;
    let png = std::env::temp_dir().join(format!("ira-icon-{}.png", std::process::id()));
    ira_parser::save_rgb565_png(&png, 48, 48, &icon).ok()?;
    let data = std::fs::read(&png);
    let _ = std::fs::remove_file(&png);
    let data = data.ok()?;
    ira_parser::convert_bytes_to_lossless_webp(&data)
}

/// Renders the banner icon of a DS ROM into lossless WebP bytes.
fn decode_nds_icon(game_root: &std::path::Path) -> Option<Vec<u8>> {
    let icon = ira_platforms::nds::read_icon(game_root)?;
    let png = std::env::temp_dir().join(format!("ira-icon-{}.png", std::process::id()));
    ira_parser::save_rgba_png(&png, 32, 32, &icon).ok()?;
    let data = std::fs::read(&png);
    let _ = std::fs::remove_file(&png);
    let data = data.ok()?;
    ira_parser::convert_bytes_to_lossless_webp(&data)
}

/// Reads a Switch ROM's native icon and re-encodes it as lossless WebP:
/// an emulator-cached JPEG, the ROM's decrypted control-NCA icon (JPEG or
/// PNG — the decoder sniffs the bytes), or a homebrew NRO's embedded PNG.
fn decode_switch_icon(game_root: &std::path::Path, switch_executable: &str) -> Option<Vec<u8>> {
    let cache = ira_platforms::switch::SwitchCaches::load(switch_executable);
    match ira_platforms::switch::native_icon(game_root, &cache, switch_executable) {
        ira_platforms::switch::SwitchIcon::Bytes(raw) => {
            ira_parser::encode_bytes_to_lossless_webp(&raw)
        }
        ira_platforms::switch::SwitchIcon::File(icon) => {
            let data = std::fs::read(icon).ok()?;
            ira_parser::encode_bytes_to_lossless_webp(&data)
        }
        ira_platforms::switch::SwitchIcon::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ira_models::GameKind;

    fn nds_game(game_path: &str, db_id: i64) -> Game {
        Game {
            kind: GameKind::Retro,
            platform_id: "nds".to_string(),
            game_path: game_path.to_string(),
            db_id,
            ..Default::default()
        }
    }

    /// A failed native decode must not touch the disk: nothing may be
    /// applied before the Save button, including on failure.
    #[test]
    fn test_native_icon_bytes_failure_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let game = nds_game("missing.nds", 0);

        let bytes = native_icon_bytes(&game, &ira_config::Config::default(), "", "", "");

        assert!(bytes.is_none());
        let data_dir = ira_parser::retro_data_dir(save_dir, game.db_id);
        assert!(ira_parser::find_image_file(&data_dir, "icon").is_none());
    }

    /// The direct-write fallback (no dialog to Save from) replaces the
    /// icon variants and leaves the small thumbnail behind.
    #[test]
    fn test_write_native_icon_to_disk_replaces_variants() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let game = nds_game("game.nds", 7);
        let data_dir = ira_parser::retro_data_dir(save_dir, 7);
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("icon.png"), b"stale").unwrap();
        let png = tmp.path().join("render.png");
        ira_parser::save_rgba_png(&png, 4, 4, &[255u8; 4 * 4 * 4]).unwrap();
        let webp =
            ira_parser::convert_bytes_to_lossless_webp(&std::fs::read(&png).unwrap()).unwrap();

        assert!(write_native_icon_to_disk(save_dir, &game, &webp));

        assert!(data_dir.join("icon.webp").is_file());
        assert!(!data_dir.join("icon.png").is_file());
        assert!(ira_parser::find_image_file(&data_dir, "icon_small").is_some());
    }
}
