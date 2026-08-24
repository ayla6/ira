//! Tolerant Valve KeyValues ("VDF") parser for Steam controller configs.
//!
//! Real workshop files lean on quirks the strict parsers reject: duplicate
//! keys everywhere (activator kinds, groups, presets), `//` comments,
//! escaped quotes inside binding strings, and mixed quoting. Everything is
//! kept in document order so imports preserve author intent.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Obj(Vec<Node>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub value: Value,
}

impl Node {
    /// First child object under `key`, if any.
    pub fn obj(&self, key: &str) -> Option<&Vec<Node>> {
        match self.get(key) {
            Some(Value::Obj(children)) => Some(children),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.children()
            .iter()
            .find(|node| node.key == key)
            .map(|node| &node.value)
    }

    /// This node's own string value, when it is not an object.
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(text) => Some(text),
            Value::Obj(_) => None,
        }
    }

    /// Numeric convenience for settings values.
    pub fn as_f32(&self) -> Option<f32> {
        self.as_str().and_then(|text| text.parse().ok())
    }

    /// Child nodes when this node holds an object; empty otherwise.
    pub(crate) fn children(&self) -> &[Node] {
        static EMPTY: [Node; 0] = [];
        match &self.value {
            Value::Obj(children) => children,
            Value::Str(_) => &EMPTY,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Str(text)) => Some(text),
            _ => None,
        }
    }
}

/// First node named `key` anywhere in the tree, document order.
pub fn find<'a>(nodes: &'a [Node], key: &str) -> Option<&'a Node> {
    for node in nodes {
        if node.key == key {
            return Some(node);
        }
        if let Value::Obj(children) = &node.value {
            if let Some(found) = find(children, key) {
                return Some(found);
            }
        }
    }
    None
}

/// Every direct child named `key`, in order — duplicates are meaningful.
pub fn find_all<'a>(nodes: &'a [Node], key: &str) -> Vec<&'a Node> {
    nodes.iter().filter(|node| node.key == key).collect()
}

/// Parse one KeyValues document into its top-level nodes.
pub fn parse_vdf(text: &str) -> Result<Vec<Node>, String> {
    let mut parser = Parser {
        chars: text.chars().peekable(),
        line: 1,
    };
    parser.skip_ws();
    let nodes = parser.parse_block()?;
    parser.skip_ws();
    if parser.peek().is_some() {
        return Err(format!(
            "line {}: trailing content after top level",
            parser.line
        ));
    }
    Ok(nodes)
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        let next = self.chars.next();
        if next == Some('\n') {
            self.line += 1;
        }
        next
    }

    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/')
                    if {
                        // Comment runs to end of line; peek the second slash
                        // without consuming the first.
                        let mut clone = self.chars.clone();
                        clone.next();
                        clone.next() == Some('/')
                    } =>
                {
                    while let Some(c) = self.bump() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some('}') => {
                    self.bump();
                    return Ok(nodes);
                }
                None => {
                    if nodes.is_empty() {
                        return Err(format!("line {}: unexpected end of file", self.line));
                    }
                    return Ok(nodes);
                }
                Some('"') => {
                    let key = self.parse_quoted()?;
                    self.skip_ws();
                    let value = match self.peek() {
                        Some('{') => {
                            self.bump();
                            Value::Obj(self.parse_block()?)
                        }
                        Some('"') => Value::Str(self.parse_quoted()?),
                        other => {
                            return Err(format!(
                                "line {}: expected {{ or quoted value for \"{key}\", found {other:?}",
                                self.line
                            ));
                        }
                    };
                    nodes.push(Node { key, value });
                }
                unexpected => {
                    return Err(format!(
                        "line {}: expected quoted key, found {unexpected:?}",
                        self.line
                    ));
                }
            }
        }
    }

    fn parse_quoted(&mut self) -> Result<String, String> {
        if self.bump() != Some('"') {
            return Err(format!("line {}: expected quote", self.line));
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(format!("line {}: unterminated string", self.line)),
                Some('"') => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(other) => {
                        let _ = write!(out, "\\{other}");
                    }
                    None => return Err(format!("line {}: dangling escape", self.line)),
                },
                Some(c) => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{find, find_all, parse_vdf};

    #[test]
    fn test_parse_vdf_basic_and_duplicates() {
        let doc = r#"
// leading comment
"controller_mappings"
{
	"version"		"3"
	"group"
	{
		"id"		"0"
		"mode"		"four_buttons"
	}
	"group"
	{
		"id"		"1"
	}
}
"#;
        let nodes = parse_vdf(doc).unwrap();
        assert_eq!(nodes.len(), 1);
        let root = &nodes[0];
        assert_eq!(root.str("version"), Some("3"));
        let groups = find_all(root.children(), "group");
        assert_eq!(groups.len(), 2, "duplicate keys stay ordered");
        assert_eq!(groups[0].str("mode"), Some("four_buttons"));
    }

    #[test]
    fn test_parse_vdf_escapes_and_comments() {
        let doc = "\"a\"\n{\n\t\"b\" \"say \\\"hi\\\"\" // trailing\n\t\"c\" \"line\\nbreak\"\n}\n";
        let nodes = parse_vdf(doc).unwrap();
        assert_eq!(nodes[0].str("b"), Some("say \"hi\""));
        assert_eq!(nodes[0].str("c"), Some("line\nbreak"));
    }

    #[test]
    fn test_find_walks_nested_nodes() {
        let doc = "\"outer\"\n{\n\t\"inner\"\n{\n\t\t\"target\" \"yes\"\n}\n}\n";
        let nodes = parse_vdf(doc).unwrap();
        assert_eq!(
            find(&nodes, "target").and_then(|node| node.as_str()),
            Some("yes")
        );
    }

    #[test]
    fn test_parse_vdf_rejects_garbage() {
        assert!(parse_vdf("\"a\" {oops}").is_err());
        assert!(parse_vdf("\"unterminated").is_err());
    }
}
