use std::collections::HashMap;
use std::path::Path;

/// A trophy definition from TROP.XML
#[derive(Debug, Clone)]
pub struct TrophyDef {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub ttype: char,
    pub hidden: bool,
}

/// Parse TROP.XML to get trophy definitions (name, detail, type, hidden).
/// Decode the five XML built-in entities. The scanners here are
/// hand-rolled and don't decode, so an escaped name would show up raw
/// ("Killer &amp; Survivor"). `&amp;` decodes last.
fn decode_xml_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn attr_value<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    tag.split_whitespace().find_map(|attr| {
        let (k, v) = attr.split_once('=')?;
        (k == key).then(|| v.trim_matches('"'))
    })
}

fn tag_name(tag: &str) -> &str {
    tag.split_whitespace().next().unwrap_or("")
}

/// Reads `<...>` starting after `<` was consumed.
/// Returns (tag content, is_closing).
fn read_tag(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<(String, bool)> {
    let mut buf = String::new();
    let closing = chars.peek() == Some(&'/');
    if closing {
        chars.next();
    }
    for ch in chars.by_ref() {
        if ch == '>' {
            return Some((buf, closing));
        }
        buf.push(ch);
    }
    None
}

pub fn parse_trop_xml(path: &Path) -> Vec<TrophyDef> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("trophy definitions unreadable ({}): {e}", path.display());
            return Vec::new();
        }
    };
    parse_trop_defs(&data)
}

struct TrophyBuilder {
    id: String,
    name: String,
    detail: String,
    ttype: char,
    hidden: bool,
}

fn parse_trop_defs(data: &str) -> Vec<TrophyDef> {
    let mut trophies = Vec::new();
    let mut current = TrophyBuilder {
        id: String::new(),
        name: String::new(),
        detail: String::new(),
        ttype: 'B',
        hidden: false,
    };
    let mut in_trophy = false;
    let mut in_name = false;
    let mut in_detail = false;

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            if in_name {
                current.name.push(c);
            } else if in_detail {
                current.detail.push(c);
            }
            continue;
        }
        let Some((tag, closing)) = read_tag(&mut chars) else {
            break;
        };
        let tag = tag.trim();
        let name = tag_name(tag);

        if closing {
            if name == "trophy" && in_trophy {
                trophies.push(TrophyDef {
                    id: std::mem::take(&mut current.id),
                    name: decode_xml_entities(&current.name),
                    detail: decode_xml_entities(&current.detail),
                    ttype: current.ttype,
                    hidden: current.hidden,
                });
                in_trophy = false;
            } else if name == "name" {
                in_name = false;
            } else if name == "detail" {
                in_detail = false;
            }
            continue;
        }
        if name == "trophy" {
            in_trophy = true;
            current.name.clear();
            current.detail.clear();
            current.ttype = 'B';
            current.hidden = false;
            if let Some(id) = attr_value(tag, "id") {
                current.id = id.to_string();
            }
            current.ttype = attr_value(tag, "ttype")
                .and_then(|v| v.chars().next())
                .unwrap_or('B');
            current.hidden = attr_value(tag, "hidden") == Some("yes");
        } else if name == "name" {
            in_name = true;
            current.name.clear();
        } else if name == "detail" {
            in_detail = true;
            current.detail.clear();
        }
    }

    trophies
}

/// Parse user trophy XML to get unlock states.
/// Returns map of trophy_id → (earned, timestamp)
pub fn parse_user_trophies(path: &Path) -> HashMap<String, (bool, i64)> {
    let mut result = HashMap::new();
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("trophy unlock states unreadable ({}): {e}", path.display());
            return result;
        }
    };

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            continue;
        }
        let Some((tag, closing)) = read_tag(&mut chars) else {
            break;
        };
        if closing || !tag.trim().starts_with("trophy") {
            continue;
        }
        let id = attr_value(&tag, "id").unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        let earned = attr_value(&tag, "unlockstate") == Some("true");
        let timestamp = attr_value(&tag, "timestamp")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        result.insert(id, (earned, timestamp));
    }

    result
}
