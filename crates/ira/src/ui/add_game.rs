use super::state::SharedState;
use adw::prelude::*;

pub fn prompt_for_steam_id(
    state: &SharedState,
    title: &str,
    body: &str,
    on_add: impl Fn(&str) + 'static,
) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some(title), Some(body));

    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("e.g. 1687950"));
    entry.set_input_purpose(gtk4::InputPurpose::Digits);
    entry.set_margin_top(8);
    entry.set_margin_bottom(8);
    entry.set_margin_start(8);
    entry.set_margin_end(8);
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("add", &crate::tr!("Add game"));
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        on_add(&entry.text());
    });
    dialog.present(Some(&window));
}
