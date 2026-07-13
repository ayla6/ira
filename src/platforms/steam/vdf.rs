use std::collections::HashMap;

/// A parsed VDF value — either a string or a nested object.
#[derive(Debug, PartialEq)]
pub enum VdfValue {
    Str(String),
    Obj(HashMap<String, VdfValue>),
}

/// Parse VDF text. Returns the inner value (the top-level key name is discarded).
/// VDF format: `"key" "value"` or `"key" { "subkey" "subvalue" }`
pub fn parse_vdf(text: &str) -> Option<VdfValue> {
    let mut p = VdfParser::new(text);
    p.skip_ws();
    let _top_key = p.parse_string()?;
    p.parse_value()
}

pub fn get_str<'a>(value: &'a VdfValue, key: &str) -> Option<&'a str> {
    match value {
        VdfValue::Obj(obj) => match obj.get(key) {
            Some(VdfValue::Str(s)) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

pub fn get_obj<'a>(value: &'a VdfValue, key: &str) -> Option<&'a HashMap<String, VdfValue>> {
    match value {
        VdfValue::Obj(obj) => match obj.get(key) {
            Some(VdfValue::Obj(o)) => Some(o),
            _ => None,
        },
        _ => None,
    }
}

/// Like `get_obj` but returns the raw `&VdfValue` (works for both Str and Obj),
/// enabling chained navigation through nested VDF structures.
pub fn get_value<'a>(value: &'a VdfValue, key: &str) -> Option<&'a VdfValue> {
    match value {
        VdfValue::Obj(obj) => obj.get(key),
        _ => None,
    }
}

struct VdfParser<'a> {
    chars: &'a [u8],
    pos: usize,
}

impl<'a> VdfParser<'a> {
    fn new(text: &'a str) -> Self {
        Self { chars: text.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                b'/' if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == b'/' => {
                    while self.pos < self.chars.len() && self.chars[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        self.skip_ws();
        if self.pos >= self.chars.len() || self.chars[self.pos] != b'"' {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != b'"' {
            if self.chars[self.pos] == b'\\' && self.pos + 1 < self.chars.len() {
                self.pos += 2;
            } else {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.chars[start..self.pos]).ok()?.to_string();
        if self.pos < self.chars.len() {
            self.pos += 1;
        }
        Some(s)
    }

    fn parse_value(&mut self) -> Option<VdfValue> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return None;
        }
        if self.chars[self.pos] == b'{' {
            self.pos += 1;
            let mut obj = HashMap::new();
            loop {
                self.skip_ws();
                if self.pos >= self.chars.len() {
                    break;
                }
                if self.chars[self.pos] == b'}' {
                    self.pos += 1;
                    break;
                }
                let key = self.parse_string()?;
                let value = self.parse_value()?;
                obj.insert(key, value);
            }
            Some(VdfValue::Obj(obj))
        } else {
            self.parse_string().map(VdfValue::Str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_app_manifest() {
        let vdf = r#"
"AppState"
{
    "appid"      "440"
    "name"       "Team Fortress 2"
    "installdir" "Team Fortress 2"
    "StateFlags" "4"
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        assert_eq!(get_str(&parsed, "appid"), Some("440"));
        assert_eq!(get_str(&parsed, "name"), Some("Team Fortress 2"));
        assert_eq!(get_str(&parsed, "installdir"), Some("Team Fortress 2"));
        assert_eq!(get_str(&parsed, "StateFlags"), Some("4"));
    }

    #[test]
    fn test_parse_library_folders() {
        let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path" "/home/user/.local/share/Steam"
    }
    "1"
    {
        "path" "/other/drive/steam"
    }
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        let folder0 = get_obj(&parsed, "0").unwrap();
        assert_eq!(folder0.get("path"), Some(&VdfValue::Str("/home/user/.local/share/Steam".into())));
    }

    #[test]
    fn test_parse_library_folders_old_format() {
        let vdf = r#"
"LibraryFolders"
{
    "0" "/home/user/.local/share/Steam"
    "1" "/other/drive"
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        assert_eq!(get_str(&parsed, "0"), Some("/home/user/.local/share/Steam"));
        assert_eq!(get_str(&parsed, "1"), Some("/other/drive"));
    }

    #[test]
    fn test_parse_loginusers() {
        let vdf = r#"
"users"
{
    "76561198000000000"
    {
        "Account"        "username"
        "PersonaName"    "Display Name"
        "MostRecent"     "1"
        "Timestamp"      "1234567890"
    }
    "76561198000000001"
    {
        "Account"        "otheruser"
        "MostRecent"     "0"
        "Timestamp"      "1234567891"
    }
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        let user = get_obj(&parsed, "76561198000000000").unwrap();
        assert_eq!(user.get("MostRecent"), Some(&VdfValue::Str("1".into())));
        assert_eq!(user.get("PersonaName"), Some(&VdfValue::Str("Display Name".into())));
    }

    #[test]
    fn test_parse_with_comments() {
        let vdf = r#"
// this is a comment
"AppState"
{
    "appid" "440" // inline comment
    "name"  "Test"
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        assert_eq!(get_str(&parsed, "appid"), Some("440"));
        assert_eq!(get_str(&parsed, "name"), Some("Test"));
    }

    #[test]
    fn test_get_value_chained_navigation() {
        let vdf = r#"
"localconfig"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "12345"
                    {
                        "Playtime" "300"
                        "LastPlayed" "1700000000"
                    }
                }
            }
        }
    }
}
"#;
        let parsed = parse_vdf(vdf).unwrap();
        let app = get_value(&parsed, "Software")
            .and_then(|s| get_value(s, "Valve"))
            .and_then(|v| get_value(v, "Steam"))
            .and_then(|s| get_value(s, "apps"))
            .and_then(|v| get_value(v, "12345"))
            .unwrap();
        assert_eq!(get_str(app, "Playtime"), Some("300"));
        assert_eq!(get_str(app, "LastPlayed"), Some("1700000000"));
    }
}
