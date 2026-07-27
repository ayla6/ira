//! Text rendering abstraction.
//!
//! Provides a unified API for text rendering with swappable backends:
//! - `cosmic-text` — per-glyph rendering, no glib dependency
//! - `pango` — per-string rasterization via pango+cairo
//!
//! When both features are enabled (default), the backend is selected at init
//! via the `IRA_OVERLAY_TEXT_BACKEND` env var. In debug builds, F10 toggles
//! between backends at runtime.
//!
//! Per-call backend dispatch uses a function pointer table indexed by an
//! atomic — no branch, just an array index + indirect call.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[cfg(all(feature = "pango", feature = "cosmic-text"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ui::atlas;
use crate::ui::vertex::Vertex;
use crate::ui::widget::Size;

mod backend;

#[cfg(feature = "pango")]
mod pango_backend;
#[cfg(feature = "cosmic-text")]
mod cosmic_backend;

use backend::TextBackend;

// --- Measure cache (shared across backends) ---

static MEASURE_CACHE: OnceLock<Mutex<HashMap<(String, u32), Size>>> = OnceLock::new();

fn measure_cache() -> std::sync::MutexGuard<'static, HashMap<(String, u32), Size>> {
    MEASURE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
}

// --- Function pointer tables ---
//
// Backend dispatch is a single atomic load + array index — no branch.
// Index 0 = cosmic-text, 1 = pango.

type ShapeFn = fn(&str, f32, f32, f32, [u8; 4]) -> (Vec<Vertex>, Vec<u32>);
type MeasureFn = fn(&str, f32) -> Size;

#[cfg(all(feature = "pango", feature = "cosmic-text"))]
static BACKEND_IDX: AtomicUsize = AtomicUsize::new(0);

#[cfg(all(feature = "pango", feature = "cosmic-text"))]
const SHAPE_FNS: [ShapeFn; 2] = [
    cosmic_backend::CosmicBackend::shape_text,
    pango_backend::PangoBackend::shape_text,
];

#[cfg(all(feature = "pango", feature = "cosmic-text"))]
const MEASURE_FNS: [MeasureFn; 2] = [
    cosmic_backend::CosmicBackend::measure,
    pango_backend::PangoBackend::measure,
];

#[cfg(not(any(feature = "pango", feature = "cosmic-text")))]
compile_error!("At least one of `pango` or `cosmic-text` features must be enabled for ira-overlay");

// --- Public API ---

pub fn init_fonts() {
    #[cfg(all(feature = "pango", feature = "cosmic-text"))]
    {
        let want_pango = std::env::var_os("IRA_OVERLAY_TEXT_BACKEND")
            .is_some_and(|v| v == "pango");
        BACKEND_IDX.store(want_pango as usize, Ordering::Relaxed);
        pango_backend::PangoBackend::init();
        cosmic_backend::CosmicBackend::init();
    }
    #[cfg(all(feature = "pango", not(feature = "cosmic-text")))]
    pango_backend::PangoBackend::init();
    #[cfg(all(feature = "cosmic-text", not(feature = "pango")))]
    cosmic_backend::CosmicBackend::init();
}

/// Toggles between pango and cosmic-text at runtime (debug builds only).
/// Only clears the measure cache and forces a UI rebuild — both backends'
/// glyph caches coexist in the atlas, so no re-rasterization is needed.
#[cfg(all(feature = "pango", feature = "cosmic-text"))]
pub fn toggle_backend() {
    let old = BACKEND_IDX.fetch_xor(1, Ordering::Relaxed);
    eprintln!(
        "ira-overlay: text backend switched to {}",
        if old == 0 { "pango" } else { "cosmic-text" }
    );
    measure_cache().clear();
    crate::ui::mark_ui_dirty();
}

pub fn clear_cache() {
    atlas::clear_cache();
    measure_cache().clear();
    #[cfg(feature = "pango")]
    pango_backend::PangoBackend::clear_cache();
    #[cfg(feature = "cosmic-text")]
    cosmic_backend::CosmicBackend::clear_cache();
}

pub fn measure_text(text: &str, font_size: f32) -> Size {
    let key = (text.to_string(), font_size as u32);

    {
        let cache = measure_cache();
        if let Some(&s) = cache.get(&key) {
            return s;
        }
    }

    let size = measure_text_uncached(text, font_size);
    measure_cache().insert(key, size);
    size
}

fn measure_text_uncached(text: &str, font_size: f32) -> Size {
    #[cfg(all(feature = "pango", feature = "cosmic-text"))]
    {
        MEASURE_FNS[BACKEND_IDX.load(Ordering::Relaxed)](text, font_size)
    }
    #[cfg(all(feature = "pango", not(feature = "cosmic-text")))]
    {
        pango_backend::PangoBackend::measure(text, font_size)
    }
    #[cfg(all(feature = "cosmic-text", not(feature = "pango")))]
    {
        cosmic_backend::CosmicBackend::measure(text, font_size)
    }
}

/// Shapes text into vertices and indices. Each backend implements its own
/// positioning strategy (cosmic-text: per-glyph, pango: per-string).
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

    #[cfg(all(feature = "pango", feature = "cosmic-text"))]
    {
        SHAPE_FNS[BACKEND_IDX.load(Ordering::Relaxed)](text, x, y, font_size, color)
    }
    #[cfg(all(feature = "pango", not(feature = "cosmic-text")))]
    {
        pango_backend::PangoBackend::shape_text(text, x, y, font_size, color)
    }
    #[cfg(all(feature = "cosmic-text", not(feature = "pango")))]
    {
        cosmic_backend::CosmicBackend::shape_text(text, x, y, font_size, color)
    }
}
