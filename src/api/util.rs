use std::collections::HashMap;

pub fn pick_lang(m: &HashMap<String, String>) -> String {
    if let Some(v) = m.get("english") {
        if !v.is_empty() {
            return v.clone();
        }
    }
    for v in m.values() {
        if !v.is_empty() {
            return v.clone();
        }
    }
    String::new()
}

pub const NEMIRTINGAS_BASE_URL: &str =
    "https://raw.githubusercontent.com/Nemirtingas/games-infos-datas/refs/heads/main/steam";

pub const MIN_IMAGE_BYTES: u64 = 200;

pub fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect()
}
