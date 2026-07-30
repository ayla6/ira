use adw::prelude::*;
use super::state::SharedState;
use super::css::*;
use super::add_game_dialog::show_add_game_dialog;
use super::auto_add_dialog::show_auto_add_dialog;
use super::installer_add_dialog::show_installer_add_dialog;

/// Small window shown when the user clicks the "+" button, letting them
/// choose how to add a game: manually or via the auto-add wizard.
pub fn show_add_game_method(state: &SharedState) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_title(Some("Add Game"));
    win.set_default_size(420, 260);
    win.set_modal(true);
    win.set_transient_for(Some(&parent));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    outer.append(&header);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.set_margin_top(12);
    body.set_margin_bottom(24);
    body.set_valign(gtk4::Align::Center);

    let title = gtk4::Label::new(Some("How do you want to add this game?"));
    title.add_css_class("title");
    body.append(&title);

    let manual_btn = gtk4::Button::with_label("Manual add");
    manual_btn.add_css_class(CSS_SUGGESTED_ACTION);
    manual_btn.set_size_request(-1, 44);
    let sc = state.clone();
    let win_c = win.clone();
    manual_btn.connect_clicked(move |_| {
        win_c.close();
        show_add_game_dialog(&sc);
    });
    body.append(&manual_btn);

    let auto_btn = gtk4::Button::with_label("Auto add game");
    auto_btn.set_tooltip_text(Some("Pick a folder and let Ira identify the game, download assets and set up everything"));
    auto_btn.set_size_request(-1, 44);
    let sc2 = state.clone();
    let win_c2 = win.clone();
    auto_btn.connect_clicked(move |_| {
        win_c2.close();
        show_auto_add_dialog(&sc2);
    });
    body.append(&auto_btn);

    let installer_btn = gtk4::Button::with_label("Install from installer");
    installer_btn.set_tooltip_text(Some("Run installer files (exe, sh, GOG) and let Ira detect the installed game"));
    installer_btn.set_size_request(-1, 44);
    let sc3 = state.clone();
    let win_c3 = win.clone();
    installer_btn.connect_clicked(move |_| {
        win_c3.close();
        show_installer_add_dialog(&sc3);
    });
    body.append(&installer_btn);

    outer.append(&body);
    win.set_content(Some(&outer));
    win.present();
}
