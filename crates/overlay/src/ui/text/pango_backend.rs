//! Pango text backend — per-string rasterization via pango + cairo.
//!
//! Uses pango for text layout and cairo for rasterization. Requires glib,
//! which may conflict with AppImages that bundle older glib versions.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use cairo::{Context as CairoContext, Format, ImageSurface};
use pango::{FontDescription, Layout};
use pango::prelude::*;
use pangocairo::prelude::*;
use pangocairo::functions::show_layout;

use crate::ui::atlas::{self, ATLAS_HEIGHT, ATLAS_WIDTH};
use crate::ui::vertex::Vertex;
use crate::ui::widget::Size;

use super::backend::TextBackend;

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

struct PangoState {
    font_map: pangocairo::FontMap,
}

unsafe impl Send for PangoState {}

impl PangoState {
    fn new() -> Self {
        ensure_fontconfig();
        let font_map = pangocairo::FontMap::new();
        let cairo_map: pangocairo::FontMap = font_map.downcast().unwrap();
        cairo_map.set_resolution(96.0);
        let families = cairo_map.list_families();
        eprintln!("ira-overlay: {} font families available", families.len());
        Self { font_map: cairo_map }
    }

    fn create_layout(&self, text: &str, font_size: f32) -> Layout {
        let context = self.font_map.create_context();
        let layout = Layout::new(&context);
        let family = std::env::var("IRA_OVERLAY_FONT_FAMILY").unwrap_or_else(|_| "Sans".to_string());
        let mut desc = FontDescription::from_string(&family);
        desc.set_size((font_size as f64 * pango::SCALE as f64) as i32);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        layout
    }
}

static STATE: OnceLock<Mutex<PangoState>> = OnceLock::new();
use std::sync::LazyLock;

static STRING_CACHE: LazyLock<Mutex<HashMap<(String, u32), atlas::AtlasSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn state() -> Option<std::sync::MutexGuard<'static, PangoState>> {
    STATE.get().map(|m| m.lock().unwrap())
}

pub struct PangoBackend;

impl TextBackend for PangoBackend {
    fn init() {
        STATE.get_or_init(|| Mutex::new(PangoState::new()));
    }

    fn measure(text: &str, font_size: f32) -> Size {
        let Some(st) = state() else {
            return Size { width: 0.0, height: font_size * 1.2 * 4.0 / 3.0 };
        };
        let layout = st.create_layout(text, font_size);
        let (w, h) = layout.pixel_size();
        Size {
            width: if w > 0 { w as f32 } else { 0.0 },
            height: if h > 0 { h as f32 } else { font_size * 1.2 * 4.0 / 3.0 },
        }
    }

    fn shape_text(
        text: &str,
        x: f32,
        y: f32,
        font_size: f32,
        color: [u8; 4],
    ) -> (Vec<Vertex>, Vec<u32>) {
        if text.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let key = (text.to_string(), font_size as u32);

        let cached = STRING_CACHE.lock().unwrap().get(&key).copied();
        let slot = if let Some(s) = cached {
            s
        } else {
            let Some(st) = state() else {
                return (Vec::new(), Vec::new());
            };
            let layout = st.create_layout(text, font_size);
            let (width, height) = layout.pixel_size();
            if width <= 0 || height <= 0 {
                return (Vec::new(), Vec::new());
            }

            let surface = ImageSurface::create(Format::ARgb32, width, height).ok();
            let Some(surface) = surface else {
                return (Vec::new(), Vec::new());
            };

            {
                let cr = CairoContext::new(&surface).ok();
                if let Some(cr) = cr {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    show_layout(&cr, &layout);
                }
            }

            surface.flush();
            let raw_ptr = surface.to_raw_none();
            let data_ptr = unsafe { cairo::ffi::cairo_image_surface_get_data(raw_ptr) };
            let stride = surface.stride();
            let w = surface.width() as usize;
            let h = surface.height() as usize;

            if data_ptr.is_null() {
                return (Vec::new(), Vec::new());
            }

            let pixels: Vec<u8> = unsafe {
                (0..h)
                    .flat_map(|row| {
                        let row_ptr = data_ptr.add(row * stride as usize);
                        let row_bytes = std::slice::from_raw_parts(row_ptr, w * 4);
                        let (chunks, _) = row_bytes.as_chunks::<4>();
                        chunks
                            .iter()
                            .flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]])
                            .collect::<Vec<u8>>()
                    })
                    .collect()
            };

            drop(st);

            let mut atlas_cache = atlas::lock_cache();
            let slot = atlas::pack_glyph(&mut atlas_cache, w as u32, h as u32, 0, 0);
            if slot.w > 0 && slot.h > 0 {
                atlas_cache.pending.push(atlas::PendingUpload {
                    atlas_x: slot.x,
                    atlas_y: slot.y,
                    width: slot.w,
                    height: slot.h,
                    pixels,
                });
            }
            drop(atlas_cache);

            STRING_CACHE.lock().unwrap().insert(key, slot);
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

    fn clear_cache() {
        STRING_CACHE.lock().unwrap().clear();
    }
}
