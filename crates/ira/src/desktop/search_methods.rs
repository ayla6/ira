use super::{
    paths,
    protocol::*,
    requests::positive_id,
    search_index::{load_index, matching_ids, SearchItem},
};
use glib::prelude::*;
use std::{
    collections::HashMap,
    process::{Command, Stdio},
};

pub(super) enum Reply {
    Ids(Vec<String>),
    Metas(Vec<SearchItem>),
    Done,
}

fn terms(parameters: &glib::Variant, index: usize) -> Result<Vec<String>, String> {
    let terms = parameters
        .child_value(index)
        .get::<Vec<String>>()
        .ok_or_else(|| "Invalid search terms".to_string())?;
    if terms.len() > 32 || terms.iter().map(String::len).sum::<usize>() > 4096 {
        return Err("Search text is too long".into());
    }
    Ok(terms)
}

fn launch(option: &str, value: &str) -> Result<(), String> {
    let mut child = Command::new(paths::executable()?)
        .arg(format!("--{option}"))
        .arg(value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Launch Ira: {error}"))?;
    std::thread::spawn(move || {
        if let Err(error) = child.wait() {
            eprintln!("Wait for desktop activation: {error}");
        }
    });
    Ok(())
}

pub(super) fn handle(method: &str, parameters: &glib::Variant) -> Result<Reply, String> {
    match method {
        "GetInitialResultSet" | "GetSubsearchResultSet" => {
            let query = terms(parameters, usize::from(method == "GetSubsearchResultSet"))?;
            Ok(Reply::Ids(matching_ids(&load_index()?, &query)))
        }
        "GetResultMetas" => {
            let ids = terms(parameters, 0)?;
            let ids = ids
                .iter()
                .map(|id| positive_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            let index = load_index()?;
            Ok(Reply::Metas(
                ids.iter()
                    .filter_map(|id| index.iter().find(|item| item.id == *id).cloned())
                    .collect(),
            ))
        }
        "ActivateResult" => {
            let id = parameters
                .child_value(0)
                .get::<String>()
                .ok_or_else(|| "Invalid result ID".to_string())?;
            let id = positive_id(&id)?;
            if !load_index()?.iter().any(|item| item.id == id) {
                return Err("This game is no longer available in desktop search".into());
            }
            launch(LAUNCH_OPTION, &id.to_string())?;
            Ok(Reply::Done)
        }
        "LaunchSearch" => {
            launch(SEARCH_OPTION, &terms(parameters, 0)?.join(" "))?;
            Ok(Reply::Done)
        }
        _ => Err("Unknown search-provider method".into()),
    }
}

pub(super) fn reply_value(reply: Reply) -> glib::Variant {
    match reply {
        Reply::Ids(ids) => (ids,).to_variant(),
        Reply::Metas(items) => {
            let values: Vec<HashMap<String, glib::Variant>> = items
                .into_iter()
                .map(|item| {
                    let icon: gio::Icon = match item.icon {
                        Some(path) => gio::FileIcon::new(&gio::File::for_path(path)).upcast(),
                        None => {
                            gio::ThemedIcon::from_names(&[APP_ID, "applications-games-symbolic"])
                                .upcast()
                        }
                    };
                    let mut meta = HashMap::from([
                        ("id".into(), item.id.to_string().to_variant()),
                        ("name".into(), item.name.to_variant()),
                        (
                            "description".into(),
                            crate::tr!("Launch with Ira").to_variant(),
                        ),
                    ]);
                    if let Some(icon) = gio::prelude::IconExt::serialize(&icon) {
                        meta.insert("icon".into(), icon);
                    }
                    meta
                })
                .collect();
            (values,).to_variant()
        }
        Reply::Done => ().to_variant(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reply_value_has_search_provider_signatures() {
        assert_eq!(
            reply_value(Reply::Ids(vec!["1".into()])).type_().as_str(),
            "(as)"
        );
        assert_eq!(
            reply_value(Reply::Metas(vec![])).type_().as_str(),
            "(aa{sv})"
        );
        assert_eq!(reply_value(Reply::Done).type_().as_str(), "()");
        let item = SearchItem {
            id: 1,
            name: "Game".into(),
            platform: "steam".into(),
            icon: None,
            last_played: 0,
        };
        let data = reply_value(Reply::Metas(vec![item]));
        let metas = data
            .child_value(0)
            .get::<Vec<HashMap<String, glib::Variant>>>()
            .unwrap();
        assert_eq!(metas[0]["id"].get::<String>().unwrap(), "1");
        assert!(metas[0].contains_key("icon"));
    }

    #[test]
    fn test_terms_enforces_type_and_request_limits() {
        assert!(terms(&(vec!["x".repeat(4097)],).to_variant(), 0).is_err());
        assert!(terms(&(vec!["x"; 33],).to_variant(), 0).is_err());
        assert!(terms(&(42,).to_variant(), 0).is_err());
    }
}
