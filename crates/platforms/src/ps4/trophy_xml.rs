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
pub fn parse_trop_xml(path: &Path) -> Vec<TrophyDef> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut trophies = Vec::new();
    let mut current_id = String::new();
    let mut current_name = String::new();
    let mut current_detail = String::new();
    let mut current_ttype = 'B';
    let mut current_hidden = false;
    let mut in_trophy = false;
    let mut in_name = false;
    let mut in_detail = false;
    let mut tag_buf = String::new();

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            // Start of a tag
            tag_buf.clear();
            let mut closing = false;
            if chars.peek() == Some(&'/') {
                chars.next();
                closing = true;
            }
            while let Some(&ch) = chars.peek() {
                if ch == '>' {
                    chars.next();
                    break;
                }
                tag_buf.push(ch);
                chars.next();
            }

            let tag = tag_buf.trim();
            let tag_name = tag.split_whitespace().next().unwrap_or("");

            if closing {
                if tag_name == "trophy" && in_trophy {
                    trophies.push(TrophyDef {
                        id: current_id.clone(),
                        name: current_name.clone(),
                        detail: current_detail.clone(),
                        ttype: current_ttype,
                        hidden: current_hidden,
                    });
                    in_trophy = false;
                } else if tag_name == "name" {
                    in_name = false;
                } else if tag_name == "detail" {
                    in_detail = false;
                }
            } else {
                if tag_name == "trophy" {
                    in_trophy = true;
                    current_name.clear();
                    current_detail.clear();
                    current_ttype = 'B';
                    current_hidden = false;
                    // Parse attributes
                    for attr in tag.split_whitespace() {
                        if let Some(eq_pos) = attr.find('=') {
                            let key = &attr[..eq_pos];
                            let val = attr[eq_pos + 1..].trim_matches('"');
                            match key {
                                "id" => current_id = val.to_string(),
                                "ttype" => {
                                    current_ttype = val.chars().next().unwrap_or('B');
                                }
                                "hidden" => current_hidden = val == "yes",
                                _ => {}
                            }
                        }
                    }
                } else if tag_name == "name" {
                    in_name = true;
                    current_name.clear();
                } else if tag_name == "detail" {
                    in_detail = true;
                    current_detail.clear();
                }
            }
        } else if in_name {
            current_name.push(c);
        } else if in_detail {
            current_detail.push(c);
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
        Err(_) => return result,
    };

    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closing = false;
            if chars.peek() == Some(&'/') {
                chars.next();
                closing = true;
            }
            while let Some(&ch) = chars.peek() {
                if ch == '>' {
                    chars.next();
                    break;
                }
                tag.push(ch);
                chars.next();
            }

            if !closing && tag.trim().starts_with("trophy") {
                let mut id = String::new();
                let mut earned = false;
                let mut timestamp: i64 = 0;

                for attr in tag.split_whitespace() {
                    if let Some(eq_pos) = attr.find('=') {
                        let key = &attr[..eq_pos];
                        let val = attr[eq_pos + 1..].trim_matches('"');
                        match key {
                            "id" => id = val.to_string(),
                            "unlockstate" => earned = val == "true",
                            "timestamp" => timestamp = val.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }

                if !id.is_empty() {
                    result.insert(id, (earned, timestamp));
                }
            }
        }
    }

    result
}
