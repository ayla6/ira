use std::sync::{Mutex, OnceLock};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Hinting, Metrics, Shaping, SwashCache};

use super::atlas::{self, ATLAS_HEIGHT, ATLAS_WIDTH};
use super::widget::Size;
use super::vertex::Vertex;

static FONT_SYSTEM: OnceLock<Mutex<FontSystem>> = OnceLock::new();
static SWASH_CACHE: OnceLock<Mutex<SwashCache>> = OnceLock::new();

pub fn init_fonts() {
    FONT_SYSTEM.get_or_init(|| Mutex::new(FontSystem::new()));
    SWASH_CACHE.get_or_init(|| Mutex::new(SwashCache::new()));
}

pub fn measure_text(text: &str, font_size: f32) -> Size {
    let mut fs_guard = FONT_SYSTEM.get().map(|m| m.lock().unwrap());
    let Some(fs) = fs_guard.as_mut() else {
        return Size { width: 0.0, height: font_size * 1.4 };
    };
    let metrics = Metrics::new(font_size, font_size * 1.4);
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(Some(f32::MAX), Some(f32::MAX));
    buffer.set_hinting(Hinting::Enabled);
    buffer.set_text(text, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);

    let mut max_w = 0.0f32;
    let mut lines = 0u32;
    for run in buffer.layout_runs() {
        lines += 1;
        for glyph in run.glyphs.iter() {
            max_w = max_w.max(glyph.x + glyph.w);
        }
    }
    let height = (font_size * 1.4).round() * lines.max(1) as f32;
    Size { width: max_w.round(), height }
}

pub fn pre_rasterize(texts: &[(f32, &str)]) {
    let mut fs_guard = FONT_SYSTEM.get().map(|m| m.lock().unwrap());
    let mut sc_guard = SWASH_CACHE.get().map(|m| m.lock().unwrap());
    let (Some(fs), Some(sc)) = (fs_guard.as_mut(), sc_guard.as_mut()) else {
        return;
    };
    let mut cache = atlas::lock_cache();

    for &(font_size, text_str) in texts {
        let metrics = Metrics::new(font_size, font_size * 1.4);
        let mut buffer = Buffer::new(fs, metrics);
        buffer.set_size(Some(4096.0), Some(4096.0));
        buffer.set_hinting(Hinting::Enabled);
        buffer.set_text(text_str, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
        buffer.shape_until_scroll(fs, false);

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                let key = physical.cache_key;
                if cache.entries.contains_key(&key) {
                    continue;
                }
                if let Some(image) = sc.get_image(fs, key) {
                    let slot = atlas::pack_glyph(
                        &mut cache,
                        image.placement.width,
                        image.placement.height,
                        image.placement.left,
                        image.placement.top,
                    );
                    if slot.w > 0 && slot.h > 0 {
                        let pixels = atlas::convert_pixels(image);
                        cache.pending.push(atlas::PendingUpload {
                            atlas_x: slot.x,
                            atlas_y: slot.y,
                            width: slot.w,
                            height: slot.h,
                            pixels,
                        });
                    }
                    cache.entries.insert(key, slot);
                }
            }
        }
    }
}

pub fn shape_text(
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    color: [u8; 4],
) -> (Vec<Vertex>, Vec<u32>) {
    let mut fs_guard = FONT_SYSTEM.get().map(|m| m.lock().unwrap());
    let mut sc_guard = SWASH_CACHE.get().map(|m| m.lock().unwrap());
    let (Some(fs), Some(sc)) = (fs_guard.as_mut(), sc_guard.as_mut()) else {
        return (Vec::new(), Vec::new());
    };
    let mut cache = atlas::lock_cache();

    let metrics = Metrics::new(font_size, font_size * 1.4);
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(Some(4096.0), Some(4096.0));
        buffer.set_hinting(Hinting::Enabled);
    buffer.set_text(text, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);

    let aw = ATLAS_WIDTH as f32;
    let ah = ATLAS_HEIGHT as f32;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((0.0, run.line_y), 1.0);
            let key = physical.cache_key;

            let slot = if let Some(s) = cache.entries.get(&key).copied() {
                s
            } else {
                let slot = match sc.get_image(fs, key) {
                    Some(image) => {
                        let s = atlas::pack_glyph(
                            &mut cache,
                            image.placement.width,
                            image.placement.height,
                            image.placement.left,
                            image.placement.top,
                        );
                        if s.w > 0 && s.h > 0 {
                            let pixels = atlas::convert_pixels(image);
                            cache.pending.push(atlas::PendingUpload {
                                atlas_x: s.x,
                                atlas_y: s.y,
                                width: s.w,
                                height: s.h,
                                pixels,
                            });
                        }
                        s
                    }
                    None => continue,
                };
                cache.entries.insert(key, slot);
                slot
            };

            if slot.w == 0 || slot.h == 0 {
                continue;
            }

            let gx = x + physical.x as f32 + slot.offset_x as f32;
            let gy = y + physical.y as f32 - slot.offset_y as f32;
            let gw = slot.w as f32;
            let gh = slot.h as f32;

            let u0 = slot.x as f32 / aw;
            let v0 = slot.y as f32 / ah;
            let u1 = (slot.x + slot.w) as f32 / aw;
            let v1 = (slot.y + slot.h) as f32 / ah;

            let i = vertices.len() as u32;
            vertices.push(Vertex { pos: [gx, gy], uv: [u0, v0], color });
            vertices.push(Vertex { pos: [gx + gw, gy], uv: [u1, v0], color });
            vertices.push(Vertex { pos: [gx, gy + gh], uv: [u0, v1], color });
            vertices.push(Vertex { pos: [gx + gw, gy + gh], uv: [u1, v1], color });
            indices.extend_from_slice(&[i, i + 1, i + 2, i + 1, i + 3, i + 2]);
        }
    }

    (vertices, indices)
}
