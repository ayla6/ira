use std::path::{Path, PathBuf};

use super::decode::encode_lossless_webp;

pub fn find_image_file(dir: &Path, base_name: &str) -> Option<PathBuf> {
    for ext in &["webp", "jpg"] {
        let p = dir.join(format!("{}.{}", base_name, ext));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Decode an image file with format guessing. Returns `None` when the file
/// is missing or undecodable — callers keep their previous state.
fn decode_image_file(path: &Path) -> Option<image::DynamicImage> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

/// Dimensions that fit inside (`max_w`, `max_h`) preserving aspect ratio.
fn thumbnail_dims(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let ratio = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let new_w = (w as f64 * ratio).ceil() as u32;
    let new_h = (h as f64 * ratio).ceil() as u32;
    (new_w.max(1), new_h.max(1))
}

/// Encode `img` as lossless WebP and write it to `dest`. Returns whether
/// the write succeeded — on failure callers leave any previous file alone.
fn write_lossless_webp(dest: &Path, img: &image::DynamicImage) -> bool {
    let data = img.to_rgba8();
    let (w, h) = data.dimensions();
    let _s = tracing::info_span!("encode_lossless_webp", w, h, path = %dest.display()).entered();
    std::fs::write(dest, encode_lossless_webp(data.as_raw(), w, h)).is_ok()
}

/// Encode both a lossless and a lossy (95%) WebP and return whichever is
/// smaller. For a ≥2× downscale the resample already removed high-frequency
/// detail, so photos shrink under lossy while flat graphics stay smaller
/// lossless.
fn encode_smaller_webp(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
    let lossless = encode_lossless_webp(pixels, w, h);
    let lossy = webp::Encoder::from_rgba(pixels, w, h).encode(95.0);
    if lossless.len() <= lossy.len() {
        lossless
    } else {
        lossy.to_vec()
    }
}

/// Generate a `_small` thumbnail variant of an image if one doesn't already exist.
///
/// Format conversion matrix:
///
/// | Source     | Case                     | Action                          |
/// |------------|--------------------------|---------------------------------|
/// | JPEG       | Already ≤ max dims       | Copy to `_small.jpg`            |
/// | Icon       | Any                      | Always lossless WebP            |
/// | any        | Mild resize (ratio > .5) | Resize → lossless WebP          |
/// | any        | ≥2× downscale            | Resize → smaller of lossless / lossy (95%) WebP |
///
pub fn ensure_small_image(dir: &Path, base_name: &str, max_w: u32, max_h: u32) {
    let _s = tracing::info_span!("ensure_small_image", base_name, max_w, max_h).entered();
    if find_image_file(dir, &format!("{}_small", base_name)).is_some() {
        return;
    }
    let Some(source) = find_image_file(dir, base_name) else {
        return;
    };

    let Some(img) = decode_image_file(&source) else {
        return;
    };
    write_small_variant(dir, base_name, &source, max_w, max_h, &img);
}

/// Normalize a just-written image file and its thumbnail in a single
/// decode: WebP/JPEG sources stay as-is and only gain a thumbnail, while
/// every other format converts to lossless WebP first — decoded once,
/// written as the full image and its small variant, with the original
/// removed. A source that does not decode is left untouched.
pub fn finalize_stored_image(dest: &Path, max_w: u32, max_h: u32) {
    let _s = tracing::info_span!("finalize_stored_image", path = %dest.display()).entered();
    let dir = dest.parent().unwrap_or(Path::new("."));
    let base = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if base.is_empty() {
        return;
    }
    let ext = dest
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "webp" || ext == "jpg" {
        ensure_small_image(dir, base, max_w, max_h);
        return;
    }
    let Some(img) = decode_image_file(dest) else {
        return;
    };
    let full = dir.join(format!("{base}.webp"));
    if !write_lossless_webp(&full, &img) {
        // The conversion failed with the original still on disk: fall back
        // to thumbnailing it.
        ensure_small_image(dir, base, max_w, max_h);
        return;
    }
    let _ = std::fs::remove_file(dest);
    // Typed by the converted file, not the original: a ".jpeg" source is
    // already WebP here, so the ≥2× smaller-of-both rule applies to it.
    write_small_variant(dir, base, &full, max_w, max_h, &img);
}

/// Write the `_small` thumbnail variant of an already-decoded image.
/// `source` is the file the pixels came from — only its extension matters
/// (JPEG sources copy instead of re-encoding when already small enough).
fn write_small_variant(
    dir: &Path,
    base_name: &str,
    source: &Path,
    max_w: u32,
    max_h: u32,
    img: &image::DynamicImage,
) {
    let small_name = format!("{}_small", base_name);
    let is_jpeg = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
    let is_icon = base_name == ira_models::AssetType::Icon.file_base();
    let (w, h) = (img.width(), img.height());

    let dest = dir.join(format!("{}.webp", small_name));
    if w <= max_w && h <= max_h {
        if is_jpeg {
            let _ = std::fs::copy(source, dir.join(format!("{}.jpg", small_name)));
            return;
        }
        write_lossless_webp(&dest, img);
        return;
    }

    let ratio = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let (new_w, new_h) = thumbnail_dims(w, h, max_w, max_h);
    let resized = {
        let _s = tracing::info_span!(
            "ensure_small_resize",
            base_name,
            src_w = w,
            src_h = h,
            dst_w = new_w,
            dst_h = new_h,
            filter = "Lanczos3"
        )
        .entered();
        img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
    };

    // Icons and mild resizes stay lossless; a ≥2× downscale keeps the
    // smaller encoding (see `encode_smaller_webp`).
    if is_icon || ratio > 0.5 {
        write_lossless_webp(&dest, &resized);
    } else {
        let data = resized.to_rgba8();
        let (fw, fh) = data.dimensions();
        let _s = tracing::info_span!(
            "ensure_small_encode",
            base_name,
            w = fw,
            h = fh,
            mode = "smaller_of_both"
        )
        .entered();
        let _ = std::fs::write(&dest, encode_smaller_webp(data.as_raw(), fw, fh));
    }
}

/// Remove all image files with the given base_name (e.g. "icon", "hero", "vertical")
/// in the directory, across all known image extensions, together with their
/// `{base}_small` thumbnails. Call before saving a new image so nothing stale
/// survives the replacement — neither a full-size file under another extension
/// nor a thumbnail of the previous image (the image loader prefers `_small`
/// files, so a leftover one would keep the old picture on screen).
pub fn remove_image_variants(dir: &Path, base_name: &str) {
    let _s = tracing::info_span!("remove_image_variants", base_name).entered();
    // "ico" stays: edit_game_save.rs still stages picked ICO-content files as
    // transient `{base}.ico` before converting them; a failed conversion is
    // only ever cleaned up by this sweep.
    for base in [base_name.to_string(), format!("{base_name}_small")] {
        for ext in ["png", "jpg", "jpeg", "webp", "ico"] {
            let p = dir.join(format!("{}.{}", base, ext));
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Given a path that may point to a `_small` image, return the full-size path.
pub fn full_image_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let p = Path::new(path);
    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    // Only the file name marks a thumbnail: a parent directory may itself
    // contain "_small" without the file being one.
    if !fname.contains("_small") {
        return path.to_string();
    }
    let parent = p.parent().unwrap_or(Path::new(""));
    let stem = fname.replace("_small", "");
    let stem = stem.split('.').next().unwrap_or(&stem);
    if let Some(full) = find_image_file(parent, stem) {
        return full.to_string_lossy().into_owned();
    }
    path.to_string()
}

/// Open an image file (PNG, ICO, etc.) and re-save as lossless WebP,
/// removing the original. Does nothing if the file is already WebP or is
/// a JPEG (JPEGs are kept as-is to avoid generation loss).
pub fn convert_to_lossless_webp(path: &Path) {
    let _s = tracing::info_span!("convert_to_lossless_webp", path = %path.display()).entered();
    if path.extension().is_some_and(|e| {
        e.eq_ignore_ascii_case("webp")
            || e.eq_ignore_ascii_case("jpg")
            || e.eq_ignore_ascii_case("jpeg")
    }) {
        return;
    }
    let base = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path.parent().unwrap_or(Path::new("."));
    let webp = parent.join(format!("{}.webp", base));

    let Some(img) = decode_image_file(path) else {
        return;
    };
    if write_lossless_webp(&webp, &img) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webp_is_lossless(data: &[u8]) -> bool {
        data.len() > 16 && data.starts_with(b"RIFF") && &data[12..16] == b"VP8L"
    }

    fn write_webp_source(dir: &std::path::Path, name: &str, w: u32, h: u32) {
        let pixels = vec![0u8; (w * h * 4) as usize];
        let encoded = encode_lossless_webp(&pixels, w, h);
        std::fs::write(dir.join(name), encoded).unwrap();
    }

    fn write_png_source(dir: &std::path::Path, name: &str, w: u32, h: u32) {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255]));
        img.save(dir.join(name)).unwrap();
    }

    #[test]
    fn test_finalize_stored_image_converts_png_and_thumbnails_once() {
        // Arrange: a PNG hero as the save thread leaves it.
        let tmp = tempfile::tempdir().unwrap();
        write_png_source(tmp.path(), "hero.png", 480, 155);
        finalize_stored_image(&tmp.path().join("hero.png"), 240, 78);
        // Assert: converted in place with a small variant beside it.
        assert!(!tmp.path().join("hero.png").is_file());
        assert!(tmp.path().join("hero.webp").is_file());
        assert!(tmp.path().join("hero_small.webp").is_file());
    }

    #[test]
    fn test_finalize_stored_image_keeps_jpeg_and_only_thumbnails() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("hero.jpg");
        let img = image::DynamicImage::new_rgb8(4, 4);
        img.save_with_format(&dest, image::ImageFormat::Jpeg).unwrap();
        finalize_stored_image(&tmp.path().join("hero.jpg"), 240, 78);
        // Assert: the JPEG stays; only the small copy is added.
        assert!(tmp.path().join("hero.jpg").is_file());
        assert!(!tmp.path().join("hero.webp").is_file());
        assert!(tmp.path().join("hero_small.jpg").is_file());
    }

    #[test]
    fn test_finalize_stored_image_leaves_garbage_alone() {
        // Arrange: undecodable bytes convert to nothing and gain no small.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("hero.png"), b"not an image").unwrap();
        finalize_stored_image(&tmp.path().join("hero.png"), 240, 78);
        assert!(tmp.path().join("hero.png").is_file());
        assert!(!tmp.path().join("hero.webp").is_file());
        assert!(!tmp.path().join("hero_small.webp").is_file());
    }

    #[test]
    fn test_decode_image_file_decodes_png_rejects_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let png = tmp.path().join("icon.png");
        image::RgbaImage::new(4, 4).save(&png).unwrap();
        let img = decode_image_file(&png).expect("valid PNG must decode");
        assert_eq!((img.width(), img.height()), (4, 4));
        assert!(decode_image_file(&tmp.path().join("missing.png")).is_none());
        assert!(decode_image_file(&tmp.path().join("missing.webp")).is_none());
    }

    #[test]
    fn test_thumbnail_dims_downscales_proportionally() {
        assert_eq!(thumbnail_dims(480, 155, 240, 78), (240, 78));
        assert_eq!(thumbnail_dims(100, 50, 60, 60), (60, 30));
    }

    #[test]
    fn test_thumbnail_dims_never_returns_zero() {
        assert_eq!(thumbnail_dims(100, 100, 0, 0), (1, 1));
    }

    #[test]
    fn test_write_lossless_webp_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("icon.webp");
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([10, 200, 30, 255]),
        ));
        assert!(write_lossless_webp(&dest, &img));
        let bytes = std::fs::read(&dest).unwrap();
        assert!(webp_is_lossless(&bytes));
        let back = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(back.dimensions(), (4, 4));
    }

    #[test]
    fn test_encode_smaller_webp_prefers_lossless_for_flat() {
        let flat = vec![0u8; 240 * 78 * 4];
        let chosen = encode_smaller_webp(&flat, 240, 78);
        assert_eq!(chosen, encode_lossless_webp(&flat, 240, 78));
    }

    #[test]
    fn test_encode_smaller_webp_prefers_lossy_for_noise() {
        let mut state: u32 = 7;
        let noise: Vec<u8> = (0..(240 * 78 * 4) as usize)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let chosen = encode_smaller_webp(&noise, 240, 78);
        assert!(
            !webp_is_lossless(&chosen),
            "noisy pixels should pick the smaller lossy WebP"
        );
    }

    #[test]
    fn test_convert_to_lossless_webp_skips_webp() {
        let tmp = tempfile::tempdir().unwrap();
        let webp_path = tmp.path().join("vertical.webp");
        let img = image::RgbaImage::new(2, 2);
        let encoded = encode_lossless_webp(img.as_raw(), 2, 2);
        std::fs::write(&webp_path, &encoded).unwrap();
        convert_to_lossless_webp(&webp_path);
        assert!(
            webp_path.is_file(),
            "webp file should not be deleted when input is already webp"
        );
    }

    #[test]
    fn test_convert_to_lossless_webp_converts_png() {
        let tmp = tempfile::tempdir().unwrap();
        let png_path = tmp.path().join("icon.png");
        let img = image::RgbaImage::new(2, 2);
        img.save(&png_path).unwrap();
        convert_to_lossless_webp(&png_path);
        assert!(
            !png_path.is_file(),
            "png file should be removed after conversion"
        );
        let webp_out = tmp.path().join("icon.webp");
        assert!(
            webp_out.is_file(),
            "webp file should exist after conversion"
        );
    }

    #[test]
    fn test_convert_to_lossless_webp_skips_jpg() {
        let tmp = tempfile::tempdir().unwrap();
        let jpg_path = tmp.path().join("hero.jpg");
        let img = image::DynamicImage::new_rgb8(4, 4);
        img.save_with_format(&jpg_path, image::ImageFormat::Jpeg)
            .unwrap();
        convert_to_lossless_webp(&jpg_path);
        assert!(
            jpg_path.is_file(),
            "jpg file should not be deleted or converted"
        );
        let webp_out = tmp.path().join("hero.webp");
        assert!(
            !webp_out.is_file(),
            "webp file should not be created from jpg"
        );
    }

    #[test]
    fn test_full_image_path_finds_jpg_from_small_webp() {
        let tmp = tempfile::tempdir().unwrap();
        let jpg = tmp.path().join("header.jpg");
        std::fs::write(&jpg, [0u8]).unwrap();
        let small_webp = tmp.path().join("header_small.webp");
        std::fs::write(&small_webp, [0u8]).unwrap();
        let result = full_image_path(small_webp.to_str().unwrap());
        assert!(
            result.ends_with("header.jpg"),
            "should find header.jpg, got: {result}"
        );
    }

    #[test]
    fn test_full_image_path_no_small_suffix() {
        assert_eq!(full_image_path("/foo/bar/hero.jpg"), "/foo/bar/hero.jpg");
        assert_eq!(full_image_path(""), "");
    }

    #[test]
    fn test_full_image_path_ignores_small_in_directory_name() {
        // Arrange: the directory name contains "_small" but the file itself
        // is a full-size image with a WebP sibling.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pack_small");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hero.jpg"), [0u8]).unwrap();
        std::fs::write(dir.join("hero.webp"), [0u8]).unwrap();
        let input = dir.join("hero.jpg").to_string_lossy().into_owned();
        let result = full_image_path(&input);
        // Assert: not a thumbnail, so the input comes back untouched.
        assert_eq!(result, input);
    }

    #[test]
    fn test_remove_image_variants_clears_base_and_small_family() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "hero.webp",
            "hero.png",
            "hero.jpg",
            "hero_small.webp",
            "hero_small.jpg",
        ] {
            std::fs::write(tmp.path().join(name), b"stale").unwrap();
        }

        remove_image_variants(tmp.path(), "hero");

        for name in [
            "hero.webp",
            "hero.png",
            "hero.jpg",
            "hero_small.webp",
            "hero_small.jpg",
        ] {
            assert!(!tmp.path().join(name).is_file(), "{name} must be removed");
        }
    }

    #[test]
    fn test_ensure_small_image_2x_downscale_picks_smaller_of_both_encodings() {
        let tmp = tempfile::tempdir().unwrap();
        write_webp_source(tmp.path(), "hero.webp", 480, 155);
        ensure_small_image(tmp.path(), "hero", 240, 78);
        let small = std::fs::read(tmp.path().join("hero_small.webp")).unwrap();

        let solid = vec![0u8; (240 * 78 * 4) as usize];
        let lossless = encode_lossless_webp(&solid, 240, 78);
        let lossy = webp::Encoder::from_rgba(&solid, 240, 78).encode(95.0);
        let expected = if lossless.len() <= lossy.len() {
            lossless
        } else {
            lossy.to_vec()
        };
        assert_eq!(
            small, expected,
            "≥2× downscale should keep the smaller of the lossless/lossy encodings"
        );
    }

    #[test]
    fn test_ensure_small_image_2x_downscale_noise_picks_lossy() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (480u32, 155u32);
        let mut state: u32 = 7;
        let pixels: Vec<u8> = (0..(w * h * 4) as usize)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let encoded = encode_lossless_webp(&pixels, w, h);
        std::fs::write(tmp.path().join("hero.webp"), &encoded).unwrap();
        ensure_small_image(tmp.path(), "hero", 240, 78);
        let small = std::fs::read(tmp.path().join("hero_small.webp")).unwrap();
        assert!(
            !webp_is_lossless(&small),
            "noisy ≥2× downscale should pick the smaller lossy WebP"
        );
    }

    #[test]
    fn test_ensure_small_image_mild_resize_stays_lossless() {
        let tmp = tempfile::tempdir().unwrap();
        write_webp_source(tmp.path(), "hero.webp", 480, 155);
        ensure_small_image(tmp.path(), "hero", 400, 129);
        let small = std::fs::read(tmp.path().join("hero_small.webp")).unwrap();
        assert!(
            webp_is_lossless(&small),
            "mild resize should stay lossless WebP"
        );
    }

    #[test]
    fn test_ensure_small_image_already_small_stays_lossless() {
        let tmp = tempfile::tempdir().unwrap();
        write_webp_source(tmp.path(), "hero.webp", 960, 310);
        ensure_small_image(tmp.path(), "hero", 960, 310);
        let small = std::fs::read(tmp.path().join("hero_small.webp")).unwrap();
        assert!(
            webp_is_lossless(&small),
            "already-small source should stay lossless WebP"
        );
    }
}
