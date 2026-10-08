use super::super::state::PendingImage;
use super::types::SgdbPickerCtx;
use adw::prelude::*;
use ira_api::types::SgdbAsset;
use ira_models::AssetType;
use std::cell::Cell;
use std::rc::Rc;

pub(super) fn build_on_download(ctx: &SgdbPickerCtx, a: &SgdbAsset) -> Rc<dyn Fn()> {
    let dest_dir = ctx.dest_dir.clone().unwrap_or_else(|| {
        let data_subdir = if ctx.is_steam_id {
            "steam".to_string()
        } else {
            "steamgriddb".to_string()
        };
        format!("{}/data/{}/{}", ctx.save_dir, data_subdir, ctx.id)
    });
    let Some(at) = AssetType::from_string(&ctx.asset) else {
        return Rc::new(|| {});
    };
    let file_name = match at {
        AssetType::Icon => {
            let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") {
                "ico"
            } else if a.mime.contains("png") {
                "png"
            } else if a.mime.contains("jpeg") || a.mime.contains("jpg") {
                "jpg"
            } else if a.mime.contains("webp") {
                "webp"
            } else {
                std::path::Path::new(&a.url)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("png")
            };
            format!("{}.{}", at.file_base(), ext)
        }
        AssetType::Hero => format!("{}.jpg", at.file_base()),
        AssetType::Grid => format!("{}.jpg", at.file_base()),
        AssetType::Header => format!("{}.jpg", at.file_base()),
        AssetType::Logo => format!("{}.png", at.file_base()),
        AssetType::Square => format!("{}.png", at.file_base()),
    };
    let dl_url = a.url.clone();
    let steam_dl = ctx.steam.clone();
    let picker_dl = ctx.picker.clone();
    let on_done_dl = ctx.on_done.clone();
    let asset_dl = ctx.asset.clone();
    let pending_dl = ctx.pending_copies.clone();

    Rc::new(move || {
        let _s = tracing::info_span!("on_download", asset = %asset_dl, url = %dl_url).entered();
        if let Some(ref pc) = pending_dl {
            let (tx, rx) = std::sync::mpsc::channel::<(Vec<u8>, bool)>();
            let rx = std::cell::RefCell::new(rx);
            let url = dl_url.clone();
            let steam = steam_dl.clone();
            std::thread::spawn(move || {
                let _s = tracing::info_span!("download_and_convert", url = %url).entered();
                match steam.download_bytes(&url) {
                    Ok(bytes) if !bytes.is_empty() => {
                        let _ = tx.send((bytes.clone(), false));
                        let converted = ira_parser::convert_bytes_to_lossless_webp(&bytes);
                        let _ = tx.send((converted.unwrap_or_else(|| bytes.clone()), true));
                    }
                    _ => {
                        eprintln!("Download failed for {}", url);
                    }
                }
            });
            let pc_c = pc.clone();
            let on_done_c = on_done_dl.clone();
            let picker_weak = picker_dl.clone();
            let asset_c = asset_dl.clone();
            let done = Rc::new(Cell::new(false));
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if picker_weak.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                loop {
                    match rx.borrow_mut().try_recv() {
                        Ok((bytes, is_converted)) => {
                            pc_c.borrow_mut().insert(
                                asset_c.clone(),
                                PendingImage::Bytes(gtk4::glib::Bytes::from_owned(bytes)),
                            );
                            if !is_converted {
                                on_done_c();
                                if let Some(picker) = picker_weak.upgrade() {
                                    picker.close();
                                }
                            } else {
                                done.set(true);
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            done.set(true);
                            break;
                        }
                    }
                }
                if done.get() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        } else {
            let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
            let rx = std::cell::RefCell::new(rx);
            let url = dl_url.clone();
            let steam = steam_dl.clone();
            let dest_dir_c = dest_dir.clone();
            let file_name_c = file_name.clone();
            let asset_at = AssetType::from_string(&asset_dl).unwrap_or(AssetType::Icon);
            std::thread::spawn(move || {
                let _s = tracing::info_span!("download_direct", url = %url).entered();
                // Byte-validated download: decode before anything touches
                // disk and persist WebP only, so a raw .ico can never land
                // in the game's image directory.
                let result = match steam.download_bytes(&url) {
                    Ok(bytes) if !bytes.is_empty() && ira_parser::is_decodable_image(&bytes) => {
                        let webp =
                            ira_parser::convert_bytes_to_lossless_webp(&bytes).unwrap_or(bytes);
                        let _ = std::fs::create_dir_all(&dest_dir_c);
                        // The pick replaces the slot's previous art: without
                        // this sweep an old file under another extension (or
                        // the previous `_small` thumbnail) survives and keeps
                        // the old picture on screen.
                        ira_parser::remove_image_variants(
                            std::path::Path::new(&dest_dir_c),
                            asset_at.file_base(),
                        );
                        let dest = std::path::Path::new(&dest_dir_c)
                            .join(&file_name_c)
                            .with_extension("webp");
                        match std::fs::write(&dest, &webp) {
                            Ok(()) => dest.to_string_lossy().into_owned(),
                            Err(error) => {
                                eprintln!("Failed to write {}: {error}", dest.display());
                                String::new()
                            }
                        }
                    }
                    Ok(_) => {
                        eprintln!("Download is not a decodable image: {url}");
                        String::new()
                    }
                    Err(error) => {
                        eprintln!("Download failed for {url}: {error}");
                        String::new()
                    }
                };
                if !result.is_empty() {
                    let (sw, sh) = asset_at.thumb_dims();
                    ira_parser::ensure_small_image(
                        std::path::Path::new(&dest_dir_c),
                        asset_at.file_base(),
                        sw,
                        sh,
                    );
                }
                let _ = tx.send(if result.is_empty() {
                    None
                } else {
                    Some(result)
                });
            });
            let on_done_c = on_done_dl.clone();
            let picker_weak = picker_dl.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if picker_weak.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                if let Ok(result) = rx.borrow_mut().try_recv() {
                    if result.is_some() {
                        on_done_c();
                        if let Some(picker) = picker_weak.upgrade() {
                            picker.close();
                        }
                    }
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    })
}
