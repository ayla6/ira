use std::path::PathBuf;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => {
            print!("null");
            return;
        }
    };

    let info = match opticaldiscs::detect::DiscImageInfo::open(&path) {
        Ok(info) => info,
        Err(_) => {
            print!("null");
            return;
        }
    };

    let serial = info
        .game
        .as_ref()
        .and_then(|g| g.serial.clone())
        .filter(|s| !s.is_empty());

    let title = info
        .game
        .as_ref()
        .and_then(|g| g.title.clone())
        .filter(|s| !s.is_empty());

    match (serial, title) {
        (Some(s), Some(t)) => print!(
            r#"{{"serial":"{}","title":"{}"}}"#,
            json_escape(&s),
            json_escape(&t)
        ),
        (Some(s), None) => print!(r#"{{"serial":"{}"}}"#, json_escape(&s)),
        (None, Some(t)) => print!(r#"{{"title":"{}"}}"#, json_escape(&t)),
        (None, None) => print!("null"),
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', r#"\""#)
}
