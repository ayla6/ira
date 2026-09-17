//! The bundled age-rating marks: official rating icons vendored from
//! Wikimedia Commons (freely licensed there — public-domain text-logos
//! and simple geometry; see `assets/ratings/README.md`), one file per
//! known board value (see `ira_models::ratings`), rasterized through
//! gdk-pixbuf on demand. Pairs without a bundled mark — boards the
//! assets don't cover — simply render as text.

/// The mark for a stored board+value pair, rendered to fit `size`
/// pixels. `None` for pairs without a bundled mark, or when no SVG
/// loader is installed for gdk-pixbuf.
pub fn rating_texture(kind: &str, value: &str, size: i32) -> Option<gdk4::Texture> {
    let slug = ira_models::ratings::rating_slug(kind, value)?;
    let bytes = svg_bytes(&slug)?;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(bytes));
    let pixbuf = gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream, size, size, true, gio::Cancellable::NONE,
    )
    .ok()?;
    Some(gdk4::Texture::for_pixbuf(&pixbuf))
}

/// The bundled SVG for an asset slug, if there is one.
fn svg_bytes(slug: &str) -> Option<&'static [u8]> {
    match slug {
        "acb-g" => Some(include_bytes!("../assets/ratings/acb-g.svg") as &[u8]),
        "acb-m" => Some(include_bytes!("../assets/ratings/acb-m.svg") as &[u8]),
        "acb-ma15" => Some(include_bytes!("../assets/ratings/acb-ma15.svg") as &[u8]),
        "acb-pg" => Some(include_bytes!("../assets/ratings/acb-pg.svg") as &[u8]),
        "acb-r18" => Some(include_bytes!("../assets/ratings/acb-r18.svg") as &[u8]),
        "bbfc-12" => Some(include_bytes!("../assets/ratings/bbfc-12.svg") as &[u8]),
        "bbfc-15" => Some(include_bytes!("../assets/ratings/bbfc-15.svg") as &[u8]),
        "bbfc-18" => Some(include_bytes!("../assets/ratings/bbfc-18.svg") as &[u8]),
        "bbfc-pg" => Some(include_bytes!("../assets/ratings/bbfc-pg.svg") as &[u8]),
        "bbfc-u" => Some(include_bytes!("../assets/ratings/bbfc-u.svg") as &[u8]),
        "cero-a" => Some(include_bytes!("../assets/ratings/cero-a.svg") as &[u8]),
        "cero-b" => Some(include_bytes!("../assets/ratings/cero-b.svg") as &[u8]),
        "cero-c" => Some(include_bytes!("../assets/ratings/cero-c.svg") as &[u8]),
        "cero-d" => Some(include_bytes!("../assets/ratings/cero-d.svg") as &[u8]),
        "cero-z" => Some(include_bytes!("../assets/ratings/cero-z.svg") as &[u8]),
        "classind-10" => Some(include_bytes!("../assets/ratings/classind-10.svg") as &[u8]),
        "classind-12" => Some(include_bytes!("../assets/ratings/classind-12.svg") as &[u8]),
        "classind-14" => Some(include_bytes!("../assets/ratings/classind-14.svg") as &[u8]),
        "classind-16" => Some(include_bytes!("../assets/ratings/classind-16.svg") as &[u8]),
        "classind-18" => Some(include_bytes!("../assets/ratings/classind-18.svg") as &[u8]),
        "classind-6" => Some(include_bytes!("../assets/ratings/classind-6.svg") as &[u8]),
        "classind-l" => Some(include_bytes!("../assets/ratings/classind-l.svg") as &[u8]),
        "classinda-10" => Some(include_bytes!("../assets/ratings/classinda-10.svg") as &[u8]),
        "classinda-12" => Some(include_bytes!("../assets/ratings/classinda-12.svg") as &[u8]),
        "classinda-14" => Some(include_bytes!("../assets/ratings/classinda-14.svg") as &[u8]),
        "classinda-16" => Some(include_bytes!("../assets/ratings/classinda-16.svg") as &[u8]),
        "classinda-18" => Some(include_bytes!("../assets/ratings/classinda-18.svg") as &[u8]),
        "classinda-6" => Some(include_bytes!("../assets/ratings/classinda-6.svg") as &[u8]),
        "classinda-al" => Some(include_bytes!("../assets/ratings/classinda-al.svg") as &[u8]),
        "csrr-0" => Some(include_bytes!("../assets/ratings/csrr-0.svg") as &[u8]),
        "csrr-12" => Some(include_bytes!("../assets/ratings/csrr-12.svg") as &[u8]),
        "csrr-15" => Some(include_bytes!("../assets/ratings/csrr-15.svg") as &[u8]),
        "csrr-18" => Some(include_bytes!("../assets/ratings/csrr-18.svg") as &[u8]),
        "csrr-6" => Some(include_bytes!("../assets/ratings/csrr-6.svg") as &[u8]),
        "elspa-11" => Some(include_bytes!("../assets/ratings/elspa-11.svg") as &[u8]),
        "elspa-15" => Some(include_bytes!("../assets/ratings/elspa-15.svg") as &[u8]),
        "elspa-18" => Some(include_bytes!("../assets/ratings/elspa-18.svg") as &[u8]),
        "elspa-3" => Some(include_bytes!("../assets/ratings/elspa-3.svg") as &[u8]),
        "esrb-ao" => Some(include_bytes!("../assets/ratings/esrb-ao.svg") as &[u8]),
        "esrb-e" => Some(include_bytes!("../assets/ratings/esrb-e.svg") as &[u8]),
        "esrb-e10" => Some(include_bytes!("../assets/ratings/esrb-e10.svg") as &[u8]),
        "esrb-m" => Some(include_bytes!("../assets/ratings/esrb-m.svg") as &[u8]),
        "esrb-rp" => Some(include_bytes!("../assets/ratings/esrb-rp.svg") as &[u8]),
        "esrb-t" => Some(include_bytes!("../assets/ratings/esrb-t.svg") as &[u8]),
        "grb-12" => Some(include_bytes!("../assets/ratings/grb-12.svg") as &[u8]),
        "grb-15" => Some(include_bytes!("../assets/ratings/grb-15.svg") as &[u8]),
        "grb-19" => Some(include_bytes!("../assets/ratings/grb-19.svg") as &[u8]),
        "grb-all" => Some(include_bytes!("../assets/ratings/grb-all.svg") as &[u8]),
        "igrs-13" => Some(include_bytes!("../assets/ratings/igrs-13.svg") as &[u8]),
        "igrs-15" => Some(include_bytes!("../assets/ratings/igrs-15.svg") as &[u8]),
        "igrs-18" => Some(include_bytes!("../assets/ratings/igrs-18.svg") as &[u8]),
        "igrs-3" => Some(include_bytes!("../assets/ratings/igrs-3.svg") as &[u8]),
        "igrs-7" => Some(include_bytes!("../assets/ratings/igrs-7.svg") as &[u8]),
        "igrs-rc" => Some(include_bytes!("../assets/ratings/igrs-rc.svg") as &[u8]),
        "igrs-su" => Some(include_bytes!("../assets/ratings/igrs-su.svg") as &[u8]),
        "nzoflc-g" => Some(include_bytes!("../assets/ratings/nzoflc-g.svg") as &[u8]),
        "nzoflc-m" => Some(include_bytes!("../assets/ratings/nzoflc-m.svg") as &[u8]),
        "nzoflc-pg" => Some(include_bytes!("../assets/ratings/nzoflc-pg.svg") as &[u8]),
        "nzoflc-r13" => Some(include_bytes!("../assets/ratings/nzoflc-r13.svg") as &[u8]),
        "nzoflc-r15" => Some(include_bytes!("../assets/ratings/nzoflc-r15.svg") as &[u8]),
        "nzoflc-r16" => Some(include_bytes!("../assets/ratings/nzoflc-r16.svg") as &[u8]),
        "nzoflc-r18" => Some(include_bytes!("../assets/ratings/nzoflc-r18.svg") as &[u8]),
        "pegi-12" => Some(include_bytes!("../assets/ratings/pegi-12.svg") as &[u8]),
        "pegi-16" => Some(include_bytes!("../assets/ratings/pegi-16.svg") as &[u8]),
        "pegi-18" => Some(include_bytes!("../assets/ratings/pegi-18.svg") as &[u8]),
        "pegi-3" => Some(include_bytes!("../assets/ratings/pegi-3.svg") as &[u8]),
        "pegi-7" => Some(include_bytes!("../assets/ratings/pegi-7.svg") as &[u8]),
        "usk-0" => Some(include_bytes!("../assets/ratings/usk-0.svg") as &[u8]),
        "usk-12" => Some(include_bytes!("../assets/ratings/usk-12.svg") as &[u8]),
        "usk-16" => Some(include_bytes!("../assets/ratings/usk-16.svg") as &[u8]),
        "usk-18" => Some(include_bytes!("../assets/ratings/usk-18.svg") as &[u8]),
        "usk-6" => Some(include_bytes!("../assets/ratings/usk-6.svg") as &[u8]),
        _ => None,
    }
}
