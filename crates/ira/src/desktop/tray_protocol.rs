use super::tray::IraTray;
use glib::{variant::ToVariant, Variant};
use std::collections::HashMap;

pub(super) const ITEM: &str = "org.kde.StatusNotifierItem";
pub(super) const MENU: &str = "com.canonical.dbusmenu";
pub(super) const ITEM_PATH: &str = "/StatusNotifierItem";
pub(super) const MENU_PATH: &str = "/MenuBar";

// Activation is deliberately absent: advertising it makes GNOME defer a single
// click while waiting to distinguish it from double-click application activation.
pub(super) const ITEM_XML: &str = r#"<node><interface name="org.kde.StatusNotifierItem">
<property name="Category" type="s" access="read"/>
<property name="Id" type="s" access="read"/>
<property name="Title" type="s" access="read"/>
<property name="Status" type="s" access="read"/>
<property name="WindowId" type="i" access="read"/>
<property name="IconName" type="s" access="read"/>
<property name="IconPixmap" type="a(iiay)" access="read"/>
<property name="IconThemePath" type="s" access="read"/>
<property name="OverlayIconName" type="s" access="read"/>
<property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
<property name="AttentionIconName" type="s" access="read"/>
<property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
<property name="AttentionMovieName" type="s" access="read"/>
<property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
<property name="ItemIsMenu" type="b" access="read"/>
<property name="Menu" type="o" access="read"/>
<signal name="NewIcon"/><signal name="NewTitle"/><signal name="NewToolTip"/>
<signal name="NewStatus"><arg type="s"/></signal>
</interface></node>"#;

pub(super) const MENU_XML: &str = r#"<node><interface name="com.canonical.dbusmenu">
<property name="Version" type="u" access="read"/>
<property name="TextDirection" type="s" access="read"/>
<property name="Status" type="s" access="read"/>
<property name="IconThemePath" type="as" access="read"/>
<method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
<method name="GetGroupProperties"><arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/></method>
<method name="GetProperty"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
<method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
<method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
<method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
<method name="AboutToShowGroup"><arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/></method>
<signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
<signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})"/><arg type="a(ias)"/></signal>
</interface></node>"#;

pub(super) fn item_property(tray: &IraTray, name: &str) -> Variant {
    let pixmaps = || tray.icons.clone();
    match name {
        "Id" => super::protocol::APP_ID.to_variant(),
        "Title" => crate::tr!("Ira").to_variant(),
        "Category" => "ApplicationStatus".to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconPixmap" => pixmaps().to_variant(),
        "OverlayIconPixmap" | "AttentionIconPixmap" => {
            Vec::<(i32, i32, Vec<u8>)>::new().to_variant()
        }
        "ItemIsMenu" => true.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH)
            .unwrap()
            .to_variant(),
        "ToolTip" => (
            "",
            pixmaps(),
            crate::tr!("Ira"),
            crate::tr!("Game library and launcher"),
        )
            .to_variant(),
        _ => "".to_variant(),
    }
}

pub(super) fn menu_property(name: &str) -> Variant {
    match name {
        "Version" => 3u32.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        _ => Vec::<String>::new().to_variant(),
    }
}

fn properties(tray: &IraTray, id: i32) -> Result<HashMap<String, Variant>, String> {
    if id == 0 {
        return Ok(HashMap::from([(
            "children-display".into(),
            "submenu".to_variant(),
        )]));
    }
    let item = tray.menu().into_iter().nth(
        id.checked_sub(1)
            .filter(|id| *id >= 0)
            .ok_or("Invalid menu ID")? as usize,
    );
    let Some((label, _)) = item else {
        return Err("Invalid menu ID".into());
    };
    Ok(HashMap::from([
        ("label".into(), label.to_variant()),
        ("enabled".into(), true.to_variant()),
        ("visible".into(), true.to_variant()),
    ]))
}

fn selected_properties(
    tray: &IraTray,
    id: i32,
    names: &[String],
) -> Result<HashMap<String, Variant>, String> {
    let mut values = properties(tray, id)?;
    if !names.is_empty() {
        values.retain(|name, _| names.contains(name));
    }
    Ok(values)
}

fn event(tray: &mut IraTray, id: i32, name: &str) -> Result<(), String> {
    properties(tray, id)?;
    if name == "clicked" && id > 0 {
        let Some((_, action)) = tray.menu().into_iter().nth((id - 1) as usize) else {
            return Err("Invalid menu ID".into());
        };
        tray.activate(action);
    }
    Ok(())
}

pub(super) fn menu_call(
    tray: &mut IraTray,
    revision: u32,
    method: &str,
    params: &Variant,
) -> Result<Variant, String> {
    match method {
        "GetLayout" => {
            let (id, depth, names) = params
                .get::<(i32, i32, Vec<String>)>()
                .ok_or("Invalid layout arguments")?;
            let values = selected_properties(tray, id, &names)?;
            let children = if id == 0 && depth != 0 {
                (1..=tray.menu().len() as i32)
                    .map(|id| {
                        Ok((
                            id,
                            selected_properties(tray, id, &names)?,
                            Vec::<Variant>::new(),
                        )
                            .to_variant())
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                Vec::new()
            };
            Ok(Variant::tuple_from_iter([
                revision.to_variant(),
                (id, values, children).to_variant(),
            ]))
        }
        "GetGroupProperties" => {
            let (ids, names) = params
                .get::<(Vec<i32>, Vec<String>)>()
                .ok_or("Invalid property arguments")?;
            let ids = if ids.is_empty() {
                (0..=tray.menu().len() as i32).collect()
            } else {
                ids
            };
            let values = ids
                .into_iter()
                .filter_map(|id| {
                    selected_properties(tray, id, &names)
                        .ok()
                        .map(|values| (id, values))
                })
                .collect::<Vec<_>>();
            Ok((values,).to_variant())
        }
        "GetProperty" => {
            let (id, name) = params
                .get::<(i32, String)>()
                .ok_or("Invalid property arguments")?;
            let value = properties(tray, id)?
                .remove(&name)
                .ok_or("Unknown menu property")?;
            Ok((value,).to_variant())
        }
        "Event" => {
            let (id, name, _, _) = params
                .get::<(i32, String, Variant, u32)>()
                .ok_or("Invalid event arguments")?;
            event(tray, id, &name)?;
            Ok(().to_variant())
        }
        "EventGroup" => {
            let (events,) = params
                .get::<(Vec<(i32, String, Variant, u32)>,)>()
                .ok_or("Invalid event arguments")?;
            let errors = events
                .into_iter()
                .filter_map(|(id, name, _, _)| event(tray, id, &name).err().map(|_| id))
                .collect::<Vec<_>>();
            Ok((errors,).to_variant())
        }
        "AboutToShow" => {
            let (id,) = params.get::<(i32,)>().ok_or("Invalid menu ID")?;
            properties(tray, id)?;
            Ok((false,).to_variant())
        }
        "AboutToShowGroup" => {
            let (ids,) = params.get::<(Vec<i32>,)>().ok_or("Invalid menu IDs")?;
            let errors = ids
                .into_iter()
                .filter(|id| properties(tray, *id).is_err())
                .collect::<Vec<_>>();
            Ok((Vec::<i32>::new(), errors).to_variant())
        }
        _ => Err("Unknown tray menu method".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_properties_select_requested_names_and_reject_invalid_ids() {
        let (sender, _) = async_channel::bounded(8);
        let tray = IraTray::new(sender);
        assert_eq!(
            properties(&tray, 0).unwrap()["children-display"]
                .get::<String>()
                .unwrap(),
            "submenu"
        );
        assert_eq!(
            properties(&tray, 1).unwrap()["label"]
                .get::<String>()
                .unwrap(),
            crate::tr!("Hide to background")
        );
        assert!(properties(&tray, i32::MIN).is_err());
        assert_eq!(
            properties(&tray, 2).unwrap()["label"]
                .get::<String>()
                .unwrap(),
            crate::tr!("Big Picture Mode")
        );
        assert!(properties(&tray, 4).is_err());
        let selected = selected_properties(&tray, 2, &["label".into()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("label"));
    }

    #[test]
    fn test_item_and_menu_properties_match_declared_types() {
        let (sender, _) = async_channel::bounded(8);
        let tray = IraTray::new(sender);
        for (name, signature) in [
            ("Menu", "o"),
            ("ItemIsMenu", "b"),
            ("IconPixmap", "a(iiay)"),
            ("ToolTip", "(sa(iiay)ss)"),
            ("Id", "s"),
            ("WindowId", "i"),
        ] {
            assert_eq!(
                item_property(&tray, name).type_().as_str(),
                signature,
                "{name}"
            );
        }
        for (name, signature) in [
            ("Version", "u"),
            ("TextDirection", "s"),
            ("Status", "s"),
            ("IconThemePath", "as"),
        ] {
            assert_eq!(menu_property(name).type_().as_str(), signature, "{name}");
        }
        assert!(item_property(&tray, "ItemIsMenu").get::<bool>().unwrap());
    }

    #[test]
    fn test_menu_call_signatures_and_event_dispatch() {
        let (sender, receiver) = async_channel::bounded(8);
        let mut tray = IraTray::new(sender);
        let layout = menu_call(
            &mut tray,
            7,
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()).to_variant(),
        )
        .unwrap();
        assert_eq!(layout.type_().as_str(), "(u(ia{sv}av))");
        assert_eq!(layout.child_value(0).get::<u32>(), Some(7));
        let group = menu_call(
            &mut tray,
            7,
            "GetGroupProperties",
            &(vec![0i32, 1, 2, 3, 4], Vec::<String>::new()).to_variant(),
        )
        .unwrap();
        assert_eq!(group.type_().as_str(), "(a(ia{sv}))");
        assert_eq!(group.child_value(0).n_children(), 4);
        let property =
            menu_call(&mut tray, 7, "GetProperty", &(1i32, "label").to_variant()).unwrap();
        assert_eq!(property.type_().as_str(), "(v)");
        assert!(menu_call(&mut tray, 7, "GetLayout", &().to_variant()).is_err());
        event(&mut tray, 1, "opened").unwrap();
        assert!(receiver.try_recv().is_err());
        event(&mut tray, 2, "clicked").unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            super::super::tray::TrayAction::ToggleBigPicture
        );
        event(&mut tray, 3, "clicked").unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            super::super::tray::TrayAction::Quit
        );
        assert!(event(&mut tray, 4, "clicked").is_err());
    }
}
