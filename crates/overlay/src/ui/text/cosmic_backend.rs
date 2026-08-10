//! Cosmic-text backend — per-glyph rendering with subpixel positioning.
//!
//! Each glyph is rasterized once, cached by `CacheKey`, and positioned
//! individually at its exact physical coordinates. This matches the
//! approach from commit b28a5ed which produced correct text alignment.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Hinting, Metrics, Shaping, SwashCache,
    SwashContent, SwashImage, Weight,
};

use crate::ui::atlas::{self, ATLAS_HEIGHT, ATLAS_WIDTH};
use crate::ui::vertex::Vertex;
use crate::ui::widget::Size;

use super::backend::TextBackend;

/// Returns the font family from `IRA_OVERLAY_FONT_FAMILY` env var,
/// or `None` to use the default (system sans-serif).
fn configured_family<'a>() -> Option<Family<'a>> {
    std::env::var("IRA_OVERLAY_FONT_FAMILY")
        .ok()
        .map(|name| Family::Name(Box::leak(name.into_boxed_str())))
}

fn attrs() -> Attrs<'static> {
    let attrs = Attrs::new().weight(Weight::NORMAL);
    match configured_family() {
        Some(family) => attrs.family(family),
        None => attrs.family(Family::SansSerif),
    }
}

struct CosmicState {
    font_system: FontSystem,
    swash_cache: SwashCache,
    buffer: Buffer,
}

unsafe impl Send for CosmicState {}

impl CosmicState {
    fn new() -> Self {
        let mut font_system = FontSystem::new();
        let face_count = font_system.db().faces().count();
        eprintln!(
            "ira-overlay: cosmic-text initialized ({} font faces)",
            face_count
        );
        let metrics = Metrics::new(16.0, 22.0);
        let buffer = Buffer::new(&mut font_system, metrics);
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            buffer,
        }
    }
}

static STATE: OnceLock<Mutex<CosmicState>> = OnceLock::new();
use std::sync::LazyLock;

static GLYPH_CACHE: LazyLock<Mutex<HashMap<CacheKey, atlas::AtlasSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn state() -> Option<std::sync::MutexGuard<'static, CosmicState>> {
    STATE.get().map(|m| m.lock().unwrap())
}

pub struct CosmicBackend;

fn convert_pixels(image: &SwashImage) -> Vec<u8> {
    match image.content {
        SwashContent::Mask => image.data.iter().flat_map(|&m| [m, m, m, m]).collect(),
        SwashContent::Color => {
            let (chunks, _) = image.data.as_chunks::<4>();
            chunks
                .iter()
                .flat_map(|bgra| {
                    let r = bgra[2];
                    let g = bgra[1];
                    let b = bgra[0];
                    let a = bgra[3];
                    [
                        (r as u16 * a as u16 / 255) as u8,
                        (g as u16 * a as u16 / 255) as u8,
                        (b as u16 * a as u16 / 255) as u8,
                        a,
                    ]
                })
                .collect()
        }
        SwashContent::SubpixelMask => {
            let (chunks, _) = image.data.as_chunks::<4>();
            chunks
                .iter()
                .flat_map(|rgba| [rgba[3], rgba[3], rgba[3], rgba[3]])
                .collect()
        }
    }
}

impl TextBackend for CosmicBackend {
    fn init() {
        STATE.get_or_init(|| Mutex::new(CosmicState::new()));
    }

    fn measure(text: &str, font_size: f32) -> Size {
        let mut st = match state() {
            Some(st) => st,
            None => {
                return Size {
                    width: 0.0,
                    height: font_size * 1.2 * 4.0 / 3.0,
                }
            }
        };

        let metrics = Metrics::pt(font_size, font_size * 1.2);
        let a = attrs();
        let CosmicState {
            font_system,
            buffer,
            ..
        } = &mut *st;
        buffer.set_metrics_and_size(metrics, Some(f32::MAX), Some(f32::MAX));
        buffer.set_hinting(Hinting::Enabled);
        buffer.set_text(text, &a, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        let mut max_w = 0.0f32;
        let mut total_h = 0.0f32;
        for run in buffer.layout_runs() {
            total_h += run.line_height;
            for glyph in run.glyphs.iter() {
                max_w = max_w.max(glyph.x + glyph.w);
            }
        }
        if total_h == 0.0 {
            total_h = font_size * 1.2 * 4.0 / 3.0;
        }
        Size {
            width: max_w.round(),
            height: total_h.round(),
        }
    }

    fn shape_text(
        text: &str,
        x: f32,
        y: f32,
        font_size: f32,
        color: [u8; 4],
    ) -> (Vec<Vertex>, Vec<u32>) {
        let mut st = match state() {
            Some(st) => st,
            None => return (Vec::new(), Vec::new()),
        };

        let metrics = Metrics::pt(font_size, font_size * 1.2);
        let a = attrs();
        let CosmicState {
            font_system,
            swash_cache,
            buffer,
        } = &mut *st;
        buffer.set_metrics_and_size(metrics, Some(4096.0), Some(4096.0));
        buffer.set_hinting(Hinting::Enabled);
        buffer.set_text(text, &a, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        // Collect layout data (immutable borrow of buffer ends after collect).
        struct LayoutItem {
            cache_key: CacheKey,
            x: i32,
            y: i32,
        }
        let layout: Vec<LayoutItem> = buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs.iter().map(move |glyph| {
                    let physical = glyph.physical((0.0, run.line_y), 1.0);
                    LayoutItem {
                        cache_key: physical.cache_key,
                        x: physical.x,
                        y: physical.y,
                    }
                })
            })
            .collect();

        let mut glyph_cache = GLYPH_CACHE.lock().unwrap();
        let mut atlas_cache = atlas::lock_cache();
        let aw = ATLAS_WIDTH as f32;
        let ah = ATLAS_HEIGHT as f32;

        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for item in &layout {
            let slot = if let Some(&s) = glyph_cache.get(&item.cache_key) {
                s
            } else {
                let image_opt = swash_cache.get_image(font_system, item.cache_key);
                let Some(image) = image_opt.as_ref() else {
                    continue;
                };
                let s = atlas::pack_glyph(
                    &mut atlas_cache,
                    image.placement.width,
                    image.placement.height,
                    image.placement.left,
                    image.placement.top,
                );
                if s.w > 0 && s.h > 0 {
                    let pixels = convert_pixels(image);
                    atlas_cache.pending.push(atlas::PendingUpload {
                        atlas_x: s.x,
                        atlas_y: s.y,
                        width: s.w,
                        height: s.h,
                        pixels,
                    });
                }
                glyph_cache.insert(item.cache_key, s);
                s
            };

            if slot.w == 0 || slot.h == 0 {
                continue;
            }

            let gx = x + item.x as f32 + slot.offset_x as f32;
            let gy = y + item.y as f32 - slot.offset_y as f32;
            let gw = slot.w as f32;
            let gh = slot.h as f32;

            let u0 = slot.x as f32 / aw;
            let v0 = slot.y as f32 / ah;
            let u1 = (slot.x + slot.w) as f32 / aw;
            let v1 = (slot.y + slot.h) as f32 / ah;

            let i = vertices.len() as u32;
            vertices.push(Vertex {
                pos: [gx, gy],
                uv: [u0, v0],
                color,
            });
            vertices.push(Vertex {
                pos: [gx + gw, gy],
                uv: [u1, v0],
                color,
            });
            vertices.push(Vertex {
                pos: [gx, gy + gh],
                uv: [u0, v1],
                color,
            });
            vertices.push(Vertex {
                pos: [gx + gw, gy + gh],
                uv: [u1, v1],
                color,
            });
            indices.extend_from_slice(&[i, i + 1, i + 2, i + 1, i + 3, i + 2]);
        }

        (vertices, indices)
    }

    fn clear_cache() {
        GLYPH_CACHE.lock().unwrap().clear();
    }
}
