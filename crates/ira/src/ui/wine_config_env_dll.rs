use super::css::*;
use adw::prelude::*;

pub(super) fn collect_env_vars(box_: &gtk4::ListBox) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut child = box_.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<gtk4::ListBoxRow>() {
            if let Some(hbox) = row.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                let children: Vec<gtk4::Widget> = {
                    let mut v = Vec::new();
                    let mut ch = hbox.first_child();
                    while let Some(c) = ch.clone() {
                        v.push(c.clone());
                        ch = c.next_sibling();
                    }
                    v
                };
                if children.len() >= 2 {
                    if let (Some(name_entry), Some(value_entry)) = (
                        children[0].downcast_ref::<gtk4::Entry>(),
                        children[1].downcast_ref::<gtk4::Entry>(),
                    ) {
                        let name = name_entry.text().to_string();
                        if !name.is_empty() {
                            result.push((name, value_entry.text().to_string()));
                        }
                    }
                }
            }
        }
        child = w.next_sibling();
    }
    result
}

pub(super) fn build_dll_override_row(name: &str, value: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some(&crate::tr!("DLL name (e.g. d3d11)")));
    name_entry.set_text(name);
    name_entry.set_hexpand(true);
    hbox.append(&name_entry);

    let model = gtk4::StringList::new(&[
        "native,builtin",
        "builtin,native",
        "native",
        "builtin",
        "disabled",
    ]);
    let value_combo = gtk4::DropDown::new(Some(model), None::<&gtk4::PropertyExpression>);
    {
        let idx = match value {
            "n,b" | "native,builtin" => 0,
            "b,n" | "builtin,native" => 1,
            "n" | "native" => 2,
            "b" | "builtin" => 3,
            "" | "d" | "disabled" => 4,
            _ => 0,
        };
        value_combo.set_selected(idx as u32);
    }
    hbox.append(&value_combo);

    let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove_btn.add_css_class(CSS_FLAT);
    remove_btn.add_css_class(CSS_CIRCULAR);
    let row_clone = row.clone();
    remove_btn.connect_clicked(move |_| {
        if let Some(list) = row_clone
            .parent()
            .and_then(|p| p.downcast::<gtk4::ListBox>().ok())
        {
            row_clone.unparent();
            list.remove(&row_clone);
        }
    });
    hbox.append(&remove_btn);

    row.set_child(Some(&hbox));
    row
}

pub(super) fn collect_dll_overrides(box_: &gtk4::ListBox) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut child = box_.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<gtk4::ListBoxRow>() {
            if let Some(hbox) = row.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                let children: Vec<gtk4::Widget> = {
                    let mut v = Vec::new();
                    let mut ch = hbox.first_child();
                    while let Some(c) = ch.clone() {
                        v.push(c.clone());
                        ch = c.next_sibling();
                    }
                    v
                };
                if children.len() >= 2 {
                    let name_entry = children[0].clone();
                    let value_combo = children[1].clone();
                    if let Some(entry) = name_entry.downcast_ref::<gtk4::Entry>() {
                        if let Some(combo) = value_combo.downcast_ref::<gtk4::DropDown>() {
                            let name = entry.text().to_string();
                            if !name.is_empty() {
                                let labels = [
                                    "native,builtin",
                                    "builtin,native",
                                    "native",
                                    "builtin",
                                    "disabled",
                                ];
                                let idx = combo.selected() as usize;
                                let value =
                                    labels.get(idx).unwrap_or(&"native,builtin").to_string();
                                result.push((name, value));
                            }
                        }
                    }
                }
            }
        }
        child = w.next_sibling();
    }
    result
}
