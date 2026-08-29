//! Minimal Qt INI reader shared by Qt-based emulators (Azahar, Eden).

/// Decodes Qt INI percent escapes (`Data%20Storage` → `Data Storage`).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let is_hex = |b: u8| (b as char).is_ascii_hexdigit();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && bytes.len() > i + 2 && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]) {
            let value = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%');
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Section names are percent-decoded; group levels in keys are kept as
/// backslash-separated key paths (`Paths\gamedirs\1\path`).
pub struct QtIni {
    /// (section, key, value) with section and key lowercased for lookups.
    pub entries: Vec<(String, String, String)>,
}

impl QtIni {
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = percent_decode(name).trim().to_lowercase();
            } else if let Some((key, value)) = line.split_once('=') {
                entries.push((
                    section.clone(),
                    percent_decode(key).trim().to_lowercase(),
                    value.trim().to_string(),
                ));
            }
        }
        Self { entries }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let section = section.to_lowercase();
        let key = key.to_lowercase();
        self.entries
            .iter()
            .find(|(s, k, _)| *s == section && *k == key)
            .map(|(_, _, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode_unescapes_sections() {
        assert_eq!(percent_decode("Data%20Storage"), "Data Storage");
        assert_eq!(percent_decode("UI"), "UI");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
    }

    #[test]
    fn test_qt_ini_reads_sections_and_keys() {
        let ini = QtIni::parse(
            "[Data%20Storage]\nnand_directory=/tmp/nand/\nnand_directory\\default=false\n",
        );
        assert_eq!(ini.get("Data Storage", "nand_directory"), Some("/tmp/nand/"));
        // Suffixed `…\default` keys must not shadow the real value.
        assert_ne!(ini.get("Data Storage", "nand_directory"), Some("false"));
    }
}
