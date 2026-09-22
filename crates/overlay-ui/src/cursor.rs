//! System cursor loader — renders the real pointer on the canvas.
//!
//! Two sources, live first: the X server's current cursor image (theme,
//! size, hotspot, and any game custom shape, read over our own xcb
//! connection) with the `left_ptr` file as fallback for Wayland sessions
//! or missing XFixes. Pixels are premultiplied for the canvas.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// One cursor image with premultiplied RGBA pixels.
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub xhot: u32,
    pub yhot: u32,
    pub pixels: Vec<u8>,
}

const XCURSOR_MAGIC: u32 = 0x72756358;
const XCURSOR_IMAGE_TYPE: u32 = 2;

fn u32le(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parses all images from an Xcursor file. Returns premultiplied pixels.
///
/// Accepts image chunks tagged either `2` (classic xcursorgen) or
/// `0xFFFD0002` (what Fedora's Adwaita files carry); the chunk layout is
/// identical and libXcursor loads both.
pub fn parse_xcursor(data: &[u8]) -> Vec<CursorImage> {
    let mut out = Vec::new();
    if u32le(data, 0) != Some(XCURSOR_MAGIC) {
        return out;
    }
    let ntoc = u32le(data, 12).unwrap_or(0) as usize;
    for i in 0..ntoc {
        let base = 16 + i * 12;
        let (typ, subtype, pos) = match (u32le(data, base), u32le(data, base + 4), u32le(data, base + 8)) {
            (Some(t), Some(s), Some(p)) => (t, s, p as usize),
            _ => continue,
        };
        if typ & 0xffff != XCURSOR_IMAGE_TYPE {
            continue;
        }
        let (w, h, xhot, yhot) = match (
            u32le(data, pos + 16),
            u32le(data, pos + 20),
            u32le(data, pos + 24),
            u32le(data, pos + 28),
        ) {
            (Some(w), Some(h), Some(x), Some(y)) if w > 0 && h > 0 => (w, h, x, y),
            _ => continue,
        };
        let count = (w as usize).checked_mul(h as usize).unwrap_or(0);
        if count == 0 || count > 1024 * 1024 {
            continue;
        }
        let mut pixels = Vec::with_capacity(count * 4);
        for p in 0..count {
            let argb = match u32le(data, pos + 36 + p * 4) {
                Some(v) => v,
                None => break,
            };
            let a = (argb >> 24) & 0xff;
            // Straight ARGB → premultiplied RGBA.
            pixels.push((((argb >> 16) & 0xff) * a / 255) as u8);
            pixels.push((((argb >> 8) & 0xff) * a / 255) as u8);
            pixels.push(((argb & 0xff) * a / 255) as u8);
            pixels.push(a as u8);
        }
        if pixels.len() == count * 4 {
            out.push(CursorImage {
                width: w,
                height: h,
                xhot,
                yhot,
                pixels,
            });
        }
        let _ = subtype;
    }
    out
}

/// Picks the smallest image at least `size` px, else the largest available.
pub fn pick_image(images: &[CursorImage], size: u32) -> Option<&CursorImage> {
    let mut best: Option<&CursorImage> = None;
    for img in images {
        let w = img.width.max(img.height);
        match best {
            None => best = Some(img),
            Some(b) => {
                let bw = b.width.max(b.height);
                if (w >= size && (bw < size || w < bw)) || (bw < size && w > bw) {
                    best = Some(img);
                }
            }
        }
    }
    best
}

fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".icons"));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home).join(".local/share/icons"));
        dirs.push(data);
    }
    dirs.push(PathBuf::from("/usr/share/icons"));
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs
}

fn find_in_theme(theme: &str, name: &str) -> Option<PathBuf> {
    theme_dirs()
        .iter()
        .map(|d| d.join(theme).join("cursors").join(name))
        .find(|p| p.is_file())
}

/// Reads `Inherits=` from a theme's index file (comma list, no quotes).
fn inherits(theme: &str) -> Vec<String> {
    let path = theme_dirs()
        .iter()
        .map(|d| d.join(theme).join("index.theme"))
        .find(|p| p.is_file());
    let text = path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let mut out = Vec::new();
    let mut in_icon_theme = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_icon_theme = line == "[Icon Theme]";
        } else if in_icon_theme && let Some(list) = line.strip_prefix("Inherits=") {
            out.extend(list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        }
    }
    out
}

/// Loads a named cursor at `size` px: theme (+ inheritance) then Adwaita.
pub fn load_cursor(name: &str, size: u32) -> Option<CursorImage> {
    let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".to_string());
    let mut chain = vec![theme];
    chain.extend(inherits(&chain[0]));
    chain.push("Adwaita".to_string());
    let mut seen = std::collections::HashSet::new();
    for theme in chain {
        if !seen.insert(theme.clone()) {
            continue;
        }
        let Some(path) = find_in_theme(&theme, name) else {
            continue;
        };
        if let Some(img) = read_and_pick(&path, size) {
            return Some(img);
        }
    }
    None
}

fn read_and_pick(path: &Path, size: u32) -> Option<CursorImage> {
    let data = std::fs::read(path).ok()?;
    let images = parse_xcursor(&data);
    pick_image(&images, size).map(|img| CursorImage {
        width: img.width,
        height: img.height,
        xhot: img.xhot,
        yhot: img.yhot,
        pixels: img.pixels.clone(),
    })
}

/// Requested cursor size: `XCURSOR_SIZE` or 24.
pub fn cursor_size() -> u32 {
    std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&s| (8..=128).contains(&s))
        .unwrap_or(24)
}

// --- Live cursor over our own xcb connection (XFixes) ---
//
// Reads the server's current cursor image: whatever the user or game has
// set right now (theme, size, hotspot, customs, animation frames via the
// serial). Best-effort dlsym like the shim: no X server, no XFixes, or any
// failure just yields `None` and the file fallback covers. xcb (not Xlib:
// the host is multithreaded and Xlib without XInitThreads is a
// heap-corruption hazard).

type XcbConnectFn = unsafe extern "C" fn(*const std::os::raw::c_char, *mut i32) -> *mut c_void;
type XcbHasErrorFn = unsafe extern "C" fn(*mut c_void) -> i32;
type XcbDisconnectFn = unsafe extern "C" fn(*mut c_void);
type XcbGetCursorImageFn = unsafe extern "C" fn(*mut c_void) -> CursorCookie;
type XcbGetCursorImageReplyFn =
    unsafe extern "C" fn(*mut c_void, CursorCookie, *mut *mut c_void) -> *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CursorCookie {
    sequence: u32,
}

fn resolve(name: &std::ffi::CStr) -> *mut c_void {
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
}

macro_rules! sym {
    ($cell:ident, $name:literal, $ty:ty) => {{
        static $cell: std::sync::OnceLock<Option<$ty>> = std::sync::OnceLock::new();
        *$cell.get_or_init(|| {
            let p = resolve($name);
            (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<_, $ty>(&p) })
        })
    }};
}

// Reply layout per libxcb xfixes.h (verified): u16 width@12, height@14,
// xhot@16, yhot@18, u32 serial@20, CARD32 ARGB pixels@32.
fn u16le(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

/// Parses an XFixesGetCursorImage reply into premultiplied pixels plus its
/// serial (callers republish on change only). Bounds-exact: the 32-byte
/// header first, then precisely w*h pixels — never reads past either.
/// Pure: unit-tested below.
fn parse_live_reply(reply: *const c_void) -> Option<(CursorImage, u32)> {
    if reply.is_null() {
        return None;
    }
    let head = unsafe { std::slice::from_raw_parts(reply as *const u8, 32) };
    let w = u16le(head, 12)? as u32;
    let h = u16le(head, 14)? as u32;
    let xhot = u16le(head, 16)? as u32;
    let yhot = u16le(head, 18)? as u32;
    let serial = u32le(head, 20)?;
    if w == 0 || h == 0 || w > 128 || h > 128 {
        return None;
    }
    let count = (w as usize).checked_mul(h as usize)?;
    let bytes = count.checked_mul(4)?;
    let px = unsafe { std::slice::from_raw_parts((reply as *const u8).add(32), bytes) };
    let mut pixels = Vec::with_capacity(bytes);
    for p in 0..count {
        let argb = u32::from_le_bytes(px[p * 4..p * 4 + 4].try_into().ok()?);
        let a = (argb >> 24) & 0xff;
        pixels.push((((argb >> 16) & 0xff) * a / 255) as u8);
        pixels.push((((argb >> 8) & 0xff) * a / 255) as u8);
        pixels.push(((argb & 0xff) * a / 255) as u8);
        pixels.push(a as u8);
    }
    Some((
        CursorImage {
            width: w,
            height: h,
            xhot: xhot.min(w.saturating_sub(1)),
            yhot: yhot.min(h.saturating_sub(1)),
            pixels,
        },
        serial,
    ))
}

/// Reads the server's current cursor image over a throwaway connection.
/// `None` when there is no X server, no XFixes, or anything fails — the
/// file fallback covers all of it.
pub fn load_live_cursor() -> Option<(CursorImage, u32)> {
    let (Some(connect), Some(has_error), Some(disconnect), Some(get_image), Some(get_reply)) = (
        sym!(CELL_CONNECT, c"xcb_connect", XcbConnectFn),
        sym!(CELL_HAS_ERROR, c"xcb_connection_has_error", XcbHasErrorFn),
        sym!(CELL_DISCONNECT, c"xcb_disconnect", XcbDisconnectFn),
        sym!(CELL_IMAGE, c"xcb_xfixes_get_cursor_image", XcbGetCursorImageFn),
        sym!(CELL_REPLY, c"xcb_xfixes_get_cursor_image_reply", XcbGetCursorImageReplyFn),
    ) else {
        return None;
    };
    unsafe {
        let conn = connect(std::ptr::null(), std::ptr::null_mut());
        if conn.is_null() || has_error(conn) != 0 {
            if !conn.is_null() {
                disconnect(conn);
            }
            return None;
        }
        let cookie = get_image(conn);
        let mut err: *mut c_void = std::ptr::null_mut();
        let reply = get_reply(conn, cookie, &mut err);
        if !err.is_null() {
            libc::free(err);
        }
        disconnect(conn);
        if reply.is_null() {
            return None;
        }
        let parsed = parse_live_reply(reply);
        libc::free(reply);
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal Xcursor blob: header + `entries` images.
    fn blob(entries: &[(u32, u32, u32, u32, Vec<u32>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let push = |v: &mut Vec<u8>, n: u32| v.extend_from_slice(&n.to_le_bytes());
        push(&mut out, XCURSOR_MAGIC);
        push(&mut out, 16);
        push(&mut out, 1);
        push(&mut out, entries.len() as u32);
        let mut pos = 16 + entries.len() * 12;
        for (w, h, _, _, px) in entries {
            push(&mut out, XCURSOR_IMAGE_TYPE);
            push(&mut out, (*w).max(*h));
            push(&mut out, pos as u32);
            pos += 36 + px.len() * 4;
        }
        for (w, h, xhot, yhot, px) in entries {
            push(&mut out, 36);
            push(&mut out, XCURSOR_IMAGE_TYPE);
            push(&mut out, (*w).max(*h));
            push(&mut out, 1);
            push(&mut out, *w);
            push(&mut out, *h);
            push(&mut out, *xhot);
            push(&mut out, *yhot);
            push(&mut out, 0);
            for p in px {
                push(&mut out, *p);
            }
        }
        out
    }

    #[test]
    fn test_parse_single_image_premultiplies() {
        // 50% opaque red: ARGB 0x80FF0000 → premultiplied (128, 0, 0, 128).
        let data = blob(&[(2, 1, 0, 0, vec![0x80FF0000, 0x8000FF00])]);
        let images = parse_xcursor(&data);
        assert_eq!(images.len(), 1);
        assert_eq!((images[0].width, images[0].height), (2, 1));
        assert_eq!(&images[0].pixels[0..4], &[128, 0, 0, 128]);
        assert_eq!(&images[0].pixels[4..8], &[0, 128, 0, 128]);
    }

    #[test]
    fn test_parse_rejects_garbage() {
        assert!(parse_xcursor(&[]).is_empty());
        assert!(parse_xcursor(b"nope nope nope nope").is_empty());
        assert!(parse_xcursor(&[0x58, 0x63, 0x75, 0x72]).is_empty());
    }

    #[test]
    fn test_pick_prefers_smallest_fitting() {
        let data = blob(&[
            (16, 16, 0, 0, vec![0xFF000000; 256]),
            (32, 32, 0, 0, vec![0xFF000000; 1024]),
            (48, 48, 0, 0, vec![0xFF000000; 2304]),
        ]);
        let images = parse_xcursor(&data);
        assert_eq!(images.len(), 3);
        assert_eq!(pick_image(&images, 24).unwrap().width, 32);
        assert_eq!(pick_image(&images, 64).unwrap().width, 48);
        assert_eq!(pick_image(&images, 16).unwrap().width, 16);
    }

        #[test]
    fn test_parse_accepts_fedora_image_tag() {
        // Fedora Adwaita files tag images 0xFFFD0002 with identical layout.
        let mut data = blob(&[(24, 24, 3, 1, vec![0xFFFFFFFF; 576])]);
        let tag_pos = 16;
        data[tag_pos..tag_pos + 4].copy_from_slice(&0xFFFD0002u32.to_le_bytes());
        let images = parse_xcursor(&data);
        assert_eq!(images.len(), 1);
        assert_eq!((images[0].width, images[0].xhot, images[0].yhot), (24, 3, 1));
    }

    #[test]
    fn test_load_real_adwaita_cursor_when_present() {
        // Integration against the system theme; skipped by succeeding
        // vacuously where no cursor theme is installed.
        let path = std::path::Path::new("/usr/share/icons/Adwaita/cursors/left_ptr");
        if !path.is_file() {
            return;
        }
        let img = load_cursor("left_ptr", 24).expect("Adwaita left_ptr must parse");
        assert!(img.width >= 16 && img.pixels.len() == img.width as usize * img.height as usize * 4);
    }

    #[test]
    fn test_cursor_size_bounds() {

        unsafe { std::env::set_var("XCURSOR_SIZE", "7") };
        assert_eq!(cursor_size(), 24);
        unsafe { std::env::set_var("XCURSOR_SIZE", "48") };
        assert_eq!(cursor_size(), 48);
        unsafe { std::env::remove_var("XCURSOR_SIZE") };
        assert_eq!(cursor_size(), 24);
    }

    /// Builds a synthetic XFixesGetCursorImage reply: 32-byte header
    /// (width/height/hotspot/serial at their libxcb offsets) + pixels.
    fn live_reply(w: u16, h: u16, xhot: u16, yhot: u16, serial: u32, px: &[u32]) -> Vec<u8> {
        let mut out = vec![0u8; 32 + px.len() * 4];
        out[12..14].copy_from_slice(&w.to_le_bytes());
        out[14..16].copy_from_slice(&h.to_le_bytes());
        out[16..18].copy_from_slice(&xhot.to_le_bytes());
        out[18..20].copy_from_slice(&yhot.to_le_bytes());
        out[20..24].copy_from_slice(&serial.to_le_bytes());
        for (i, p) in px.iter().enumerate() {
            out[32 + i * 4..36 + i * 4].copy_from_slice(&p.to_le_bytes());
        }
        out
    }

    #[test]
    fn test_parse_live_reply_premultiplies_and_serials() {
        // 2x1, hotspot (1, 0), serial 42: opaque blue + 50% red.
        let data = live_reply(2, 1, 1, 0, 42, &[0xFF0000FF, 0x80FF0000]);
        let (img, serial) = parse_live_reply(data.as_ptr() as *const std::ffi::c_void).unwrap();
        assert_eq!(serial, 42);
        assert_eq!((img.width, img.height, img.xhot, img.yhot), (2, 1, 1, 0));
        assert_eq!(&img.pixels[0..4], &[0, 0, 255, 255]);
        assert_eq!(&img.pixels[4..8], &[128, 0, 0, 128]);
    }

    #[test]
    fn test_parse_live_reply_rejects_absurd_sizes() {
        assert!(parse_live_reply(std::ptr::null()).is_none());
        let data = live_reply(0, 0, 0, 0, 1, &[]);
        assert!(parse_live_reply(data.as_ptr() as *const std::ffi::c_void).is_none());
        let data = live_reply(512, 512, 0, 0, 1, &[]);
        assert!(parse_live_reply(data.as_ptr() as *const std::ffi::c_void).is_none());
    }
}
