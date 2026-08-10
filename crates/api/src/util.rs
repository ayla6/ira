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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_pick_lang_empty() {
        let m = HashMap::new();
        assert_eq!(pick_lang(&m), "");
    }

    #[test]
    fn test_pick_lang_single() {
        let mut m = HashMap::new();
        m.insert("english".to_string(), "Hello".to_string());
        assert_eq!(pick_lang(&m), "Hello");
    }

    #[test]
    fn test_pick_lang_multiple() {
        let mut m = HashMap::new();
        m.insert("french".to_string(), "Bonjour".to_string());
        m.insert("english".to_string(), "Hello".to_string());
        m.insert("german".to_string(), "Hallo".to_string());
        assert_eq!(pick_lang(&m), "Hello");
    }

    #[test]
    fn test_pick_lang_fallback() {
        let mut m = HashMap::new();
        m.insert("schlonk".to_string(), "glorp".to_string());
        assert_eq!(pick_lang(&m), "glorp");
    }

    #[test]
    fn test_pick_lang_all_empty() {
        let mut m = HashMap::new();
        m.insert("english".to_string(), "".to_string());
        m.insert("german".to_string(), "".to_string());
        assert_eq!(pick_lang(&m), "");
    }

    #[test]
    fn test_urlencode_empty() {
        assert_eq!(urlencode(""), "");
    }

    #[test]
    fn test_urlencode_spaces() {
        assert_eq!(urlencode("hello world"), "hello%20world");
    }

    #[test]
    fn test_urlencode_special_chars() {
        assert_eq!(urlencode("a/b?c"), "a%2Fb%3Fc");
    }

    #[test]
    fn test_urlencode_unicode_alphanumeric_passthrough() {
        assert_eq!(urlencode("café"), "café");
    }

    #[test]
    fn test_urlencode_safe_chars() {
        assert_eq!(
            urlencode("hello-world_foo.bar~baz"),
            "hello-world_foo.bar~baz"
        );
    }
}
