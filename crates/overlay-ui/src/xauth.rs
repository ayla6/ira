//! Xauthority cookie reading + desktop display plumbing.
//!
//! The process environment belongs to the gamescope session (wrong DISPLAY,
//! wrong XAUTHORITY), so the desktop thread parses the display string and
//! reads the MIT cookie itself instead of inheriting either. Pure helpers
//! are unit-tested; `connect_desktop` logs each failure distinctly so a
//! failure names itself.

/// Parses a display string into (number, screen): ":0" -> (0, 0),
/// "host:2.1" -> (2, 1). Local Unix-socket displays only; anything else
/// (TCP hosts, abstract paths) is out of scope and refused.
pub(super) fn parse_display_number(display: &str) -> Option<(u32, usize)> {
    let after_colon = display.rsplit(':').next()?;
    let mut parts = after_colon.split('.');
    let num: u32 = parts.next()?.parse().ok()?;
    let screen: usize = match parts.next() {
        None => 0,
        Some(s) => s.parse().ok()?,
    };
    parts.next().is_none().then_some((num, screen))
}

fn take_field(data: &[u8], off: usize) -> Option<(Vec<u8>, usize)> {
    let len = u16::from_be_bytes(data.get(off..off.checked_add(2)?)?.try_into().ok()?) as usize;
    let end = off.checked_add(2)?.checked_add(len)?;
    Some((data.get(off + 2..end)?.to_vec(), end))
}

/// MIT cookie for a display number out of an Xauthority file's bytes.
/// Scores candidates: exact display beats the blank-display wildcard
/// desktop compositors leave behind, local family beats the rest.
/// Pure for testability.
pub(super) fn read_cookie(data: &[u8], display_no: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    const FAMILY_LOCAL: u16 = 256;
    let mut best: Option<(i32, Vec<u8>, Vec<u8>)> = None;
    let mut off = 0;
    while off < data.len() {
        let family = u16::from_be_bytes(data.get(off..off.checked_add(2)?)?.try_into().ok()?);
        off += 2;
        let (_, next) = take_field(data, off)?;
        off = next;
        let (disp, next) = take_field(data, off)?;
        off = next;
        let (name, next) = take_field(data, off)?;
        off = next;
        let (cookie, next) = take_field(data, off)?;
        off = next;
        if name != b"MIT-MAGIC-COOKIE-1" {
            continue;
        }
        let disp_score = if disp == display_no.as_bytes() {
            2
        } else if disp.is_empty() {
            1
        } else {
            continue;
        };
        // Exact display always outranks the wildcard; family breaks ties.
        let score = disp_score * 2 + i32::from(family == FAMILY_LOCAL);
        if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
            best = Some((score, name, cookie));
        }
    }
    best.map(|(_, name, cookie)| (name, cookie))
}

/// Connects to the desktop X server with an explicit authority cookie:
/// the process environment belongs to the gamescope session (wrong
/// DISPLAY, wrong XAUTHORITY), so neither may be read here. Each stage
/// logs distinctly so a failure names itself.
pub(super) fn connect_desktop(
    display: &str,
    auth_path: Option<&str>,
) -> Option<x11rb::rust_connection::RustConnection> {
    use std::os::unix::net::UnixStream;
    use x11rb::rust_connection::{DefaultStream, RustConnection};
    let (num, screen) = parse_display_number(display).or_else(|| {
        eprintln!("ira-overlay-ui: desktop input disabled, bad display '{display}'");
        None
    })?;
    let stream = UnixStream::connect(format!("/tmp/.X11-unix/X{num}"))
        .ok()
        .or_else(|| {
            eprintln!("ira-overlay-ui: desktop input disabled, no socket for :{num}");
            None
        })?;
    let (defstream, _) = DefaultStream::from_unix_stream(stream).ok().or_else(|| {
        eprintln!("ira-overlay-ui: desktop input disabled, stream wrap failed");
        None
    })?;
    let cookie = auth_path
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|data| read_cookie(&data, &num.to_string()));
    if auth_path.is_some() && cookie.is_none() {
        eprintln!("ira-overlay-ui: no usable cookie, trying {display} without auth");
    }
    let (auth_name, auth_data) = cookie.unwrap_or_default();
    RustConnection::connect_to_stream_with_auth_info(defstream, screen, auth_name, auth_data)
        .map_err(|e| {
            eprintln!("ira-overlay-ui: desktop input disabled, X setup refused: {e}");
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_display_number_local_only() {
        assert_eq!(parse_display_number(":0"), Some((0, 0)));
        assert_eq!(parse_display_number(":2"), Some((2, 0)));
        assert_eq!(parse_display_number(":0.0"), Some((0, 0)));
        assert_eq!(parse_display_number("host:1.2"), Some((1, 2)));
        assert_eq!(parse_display_number(""), None);
        assert_eq!(parse_display_number(":abc"), None);
        assert_eq!(parse_display_number("no-colon"), None);
        assert_eq!(parse_display_number(":0.1.2"), None);
    }

    /// One Xauthority record: family + address/display/name/cookie fields.
    fn xauth_record(family: u16, addr: &[u8], disp: &str, name: &[u8], cookie: &[u8]) -> Vec<u8> {
        fn field(bytes: &[u8]) -> Vec<u8> {
            let mut v = (bytes.len() as u16).to_be_bytes().to_vec();
            v.extend_from_slice(bytes);
            v
        }
        let mut v = family.to_be_bytes().to_vec();
        v.extend(field(addr));
        v.extend(field(disp.as_bytes()));
        v.extend(field(name));
        v.extend(field(cookie));
        v
    }

    #[test]
    fn test_read_cookie_picks_local_mit_record() {
        const MIT: &[u8] = b"MIT-MAGIC-COOKIE-1";
        let mut blob = Vec::new();
        // Wild-family record for :0 sorts before the local one but loses.
        blob.extend(xauth_record(0, b"x", "0", MIT, b"AAAA"));
        blob.extend(xauth_record(256, b"host", "0", MIT, b"BBBB"));
        // Another display never matches.
        blob.extend(xauth_record(256, b"host", "1", MIT, b"CCCC"));
        // Wrong mechanism never matches.
        blob.extend(xauth_record(
            256,
            b"host",
            "0",
            b"XDM-AUTHORIZATION-1",
            b"DDDD",
        ));
        assert_eq!(
            read_cookie(&blob, "0"),
            Some((MIT.to_vec(), b"BBBB".to_vec()))
        );
        assert_eq!(
            read_cookie(&blob, "1"),
            Some((MIT.to_vec(), b"CCCC".to_vec()))
        );
        assert_eq!(read_cookie(&blob, "2"), None);
        assert_eq!(read_cookie(&[], "0"), None);
        assert_eq!(read_cookie(b"truncated", "0"), None);
    }

    #[test]
    fn test_read_cookie_accepts_blank_display_wildcard() {
        const MIT: &[u8] = b"MIT-MAGIC-COOKIE-1";
        // Desktop compositors leave the display field blank: matches any
        // number, behind exact matches.
        let mut blob = Vec::new();
        blob.extend(xauth_record(256, b"host", "", MIT, b"WILD"));
        blob.extend(xauth_record(0, b"x", "1", MIT, b"OTHER"));
        assert_eq!(
            read_cookie(&blob, "0"),
            Some((MIT.to_vec(), b"WILD".to_vec()))
        );
        // An exact display match outranks the wildcard.
        blob.extend(xauth_record(0, b"x", "0", MIT, b"EXACT"));
        assert_eq!(
            read_cookie(&blob, "0"),
            Some((MIT.to_vec(), b"EXACT".to_vec()))
        );
    }
}
