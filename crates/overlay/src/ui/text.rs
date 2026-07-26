//! Text rendering via pango + cairo.
//!
//! Each unique (text, font_size) pair is rendered once to a cairo ImageSurface,
//! uploaded to the GPU texture atlas, and cached. Subsequent draws reuse the
//! cached texture.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use cairo::{Context as CairoContext, Format, ImageSurface};
use pango::{FontDescription, FontMap, Layout};
use pango::prelude::FontMapExt;
use pangocairo::functions::show_layout;

use super::atlas::{self, ATLAS_HEIGHT, ATLAS_WIDTH};
use super::vertex::Vertex;
use super::widget::Size;

/// Initialize fontconfig before creating Pango font maps.
/// Called once per process via `get_or_init`.
fn ensure_fontconfig() {
    let lib_name = CString::new("libfontconfig.so.1").unwrap();
    let init_name = CString::new("FcInit").unwrap();
    unsafe {
        let lib = libc::dlopen(lib_name.as_ptr(), libc::RTLD_LAZY);
        if lib.is_null() {
            eprintln!("ira-overlay: libfontconfig.so.1 not found");
            return;
        }
        let sym = libc::dlsym(lib, init_name.as_ptr());
        if sym.is_null() {
            eprintln!("ira-overlay: FcInit not found");
            return;
        }
        let init: unsafe extern "C" fn() -> i32 = std::mem::transmute(sym);
        eprintln!("ira-overlay: FcInit() = {}", init());
    }
}

struct TextState {
    font_map: FontMap,
    cache: HashMap<(String, u32), atlas::AtlasSlot>,
}

// SAFETY: TextState is only accessed from the Vulkan present hook thread.
unsafe impl Send for TextState {}

impl TextState {
    fn new() -> Self {
        ensure_fontconfig();
        let font_map = pangocairo::FontMap::default();
        let families = font_map.list_families();
        eprintln!("ira-overlay: {} font families available", families.len());
        Self {
            font_map,
            cache: HashMap::new(),
        }
    }

    fn create_layout(&self, text: &str, font_size: f32) -> Layout {
        let context = self.font_map.create_context();
        let layout = Layout::new(&context);
        let desc = FontDescription::from_string(&format!("Sans {}", font_size as i32));
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        layout
    }
}

static STATE: OnceLock<Mutex<TextState>> = OnceLock::new();

fn state() -> Option<std::sync::MutexGuard<'static, TextState>> {
    STATE.get().map(|m| m.lock().unwrap())
}

pub fn init_fonts() {
    STATE.get_or_init(|| Mutex::new(TextState::new()));
}

/// Clears the text cache. Must be called when the atlas texture is recreated
/// (e.g. new swapchain) so that text is re-rendered into the new atlas.
pub fn clear_cache() {
    if let Some(mut st) = state() {
        st.cache.clear();
    }
}

pub fn measure_text(text: &str, font_size: f32) -> Size {
    let Some(st) = state() else {
        return Size { width: 0.0, height: font_size * 1.4 };
    };
    let layout = st.create_layout(text, font_size);
    let (w, h) = layout.pixel_size();
    Size {
        width: if w > 0 { w as f32 } else { 0.0 },
        height: if h > 0 { h as f32 } else { font_size * 1.4 },
    }
}

pub fn shape_text(
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    color: [u8; 4],
) -> (Vec<Vertex>, Vec<u32>) {
    if text.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let Some(mut st) = state() else {
        return (Vec::new(), Vec::new());
    };

    let key = (text.to_string(), font_size as u32);

    let slot = if let Some(&s) = st.cache.get(&key) {
        s
    } else {
        let layout = st.create_layout(text, font_size);
        let (width, height) = layout.pixel_size();
        if width <= 0 || height <= 0 {
            return (Vec::new(), Vec::new());
        }

        let surface = match ImageSurface::create(Format::ARgb32, width, height) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ira-overlay: ImageSurface create failed: {:?}", e);
                return (Vec::new(), Vec::new());
            }
        };

        // Render text in white — the vertex color provides the actual color.
        // The Cairo context holds a reference to the surface; scope it so
        // it's dropped before we read the pixel data.
        {
            let cr = match CairoContext::new(&surface) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("ira-overlay: CairoContext create failed: {:?}", e);
                    return (Vec::new(), Vec::new());
                }
            };
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            show_layout(&cr, &layout);
        }

        // Read pixel data via FFI to bypass cairo-rs's refcount check.
        // The context is dropped above, but cairo-rs still sees refcount > 1
        // because the Shared wrapper may not have fully released the surface.
        // We call flush() to ensure all rendering is complete, then read
        // the data pointer directly.
        surface.flush();
        let raw_ptr = surface.to_raw_none();
        let data_ptr = unsafe { cairo::ffi::cairo_image_surface_get_data(raw_ptr) };
        let stride = surface.stride();
        let w = surface.width() as usize;
        let h = surface.height() as usize;

        if data_ptr.is_null() {
            eprintln!("ira-overlay: surface data is null for {:?}", text);
            return (Vec::new(), Vec::new());
        }

        // Extract pixels and convert BGRA → RGBA for the Vulkan texture.
        let pixels: Vec<u8> = unsafe {
            (0..h)
                .flat_map(|row| {
                    let row_ptr = data_ptr.add(row * stride as usize);
                    let row_bytes = std::slice::from_raw_parts(row_ptr, w * 4);
                    row_bytes.chunks_exact(4)
                        .flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]])
                        .collect::<Vec<u8>>()
                })
                .collect()
        };

        let mut cache = atlas::lock_cache();
        let slot = atlas::pack(&mut cache, width as u32, height as u32);
        if slot.w > 0 && slot.h > 0 {
            cache.pending.push(atlas::PendingUpload {
                atlas_x: slot.x,
                atlas_y: slot.y,
                width: slot.w,
                height: slot.h,
                pixels,
            });
        }
        drop(cache);

        st.cache.insert(key.clone(), slot);
        slot
    };

    if slot.w == 0 || slot.h == 0 {
        return (Vec::new(), Vec::new());
    }

    let aw = ATLAS_WIDTH as f32;
    let ah = ATLAS_HEIGHT as f32;
    let w = slot.w as f32;
    let h = slot.h as f32;

    let u0 = slot.x as f32 / aw;
    let v0 = slot.y as f32 / ah;
    let u1 = (slot.x + slot.w) as f32 / aw;
    let v1 = (slot.y + slot.h) as f32 / ah;

    let vertices = vec![
        Vertex { pos: [x, y], uv: [u0, v0], color },
        Vertex { pos: [x + w, y], uv: [u1, v0], color },
        Vertex { pos: [x, y + h], uv: [u0, v1], color },
        Vertex { pos: [x + w, y + h], uv: [u1, v1], color },
    ];
    let indices = vec![0u32, 1, 2, 1, 3, 2];

    (vertices, indices)
}
