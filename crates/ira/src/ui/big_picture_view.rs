//! The big-picture couch shell: status rail on top (avatar, date, clock,
//! battery), a two-page stack in the middle (home carousel, All Software
//! grid), and a bottom rail (connected gamepads, button prompts).
//! Controller and keyboard input is routed to whichever page is showing.

use super::big_picture_all::AllSoftwareUi;
use super::big_picture_home::HomeUi;
use super::big_picture_input::{NavCommand, NavMsg};
use super::big_picture_status::{BottomBar, StatusBar};
use super::css::*;
use super::state::SharedState;
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Which page of the couch shell is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    AllSoftware,
}

/// Widgets of the couch view, kept on `AppState` behind an `Rc` so every
/// handler — refreshes, navigation, the scroll ticker — sees the same
/// selection state instead of a deep-cloned snapshot.
pub struct BigPictureUi {
    stack: gtk4::Stack,
    home_page: gtk4::Box,
    all_page: gtk4::Box,
    bottom: BottomBar,
    page: Cell<Page>,
    pub(super) home: HomeUi,
    pub(super) all: AllSoftwareUi,
}

/// Build the couch window (fullscreen is applied by main.rs) and take over
/// the shared state's window reference.
pub(super) fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(&crate::tr!("Ira")));
    window.set_size_request(900, 650);

    super::css::init_styles();

    let square_mode = state.borrow().cfg.big_picture_square_capsules;
    let (root, ui) = build_root(state, square_mode);
    // Key events land on the toplevel whenever nothing else holds focus, so
    // the keyboard handler lives there rather than on a child widget.
    wire_keyboard(state, &window);
    // The desktop window's close wiring never runs in this mode, so honor
    // the close-to-background setting here: without it a compositor close
    // would destroy the window and leave the process running headless.
    {
        let close_state = state.clone();
        window.connect_close_request(move |_| {
            let close_to_background = close_state.borrow().cfg.close_to_background;
            if close_to_background {
                super::background::show_close_choice_dialog(&close_state);
                glib::Propagation::Stop
            } else {
                if let Some(app) = close_state.borrow().window.application() {
                    app.quit();
                }
                glib::Propagation::Proceed
            }
        });
    }

    {
        let mut s = state.borrow_mut();
        s.window = window.clone();
        s.big_picture = Some(Rc::new(ui));
    }
    window.set_content(Some(&root));
    window.present();

    refresh(state);
    super::big_picture_input::start(state);
}

fn build_root(state: &SharedState, square_mode: bool) -> (gtk4::Overlay, BigPictureUi) {
    let overlay = gtk4::Overlay::new();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class(CSS_BP_ROOT);

    let status = StatusBar::build();
    root.append(status.widget());

    let stack = gtk4::Stack::new();
    stack.set_vexpand(true);
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(150);
    let (home_page, home) = super::big_picture_home::build(state, square_mode);
    let (all_page, all) = super::big_picture_all::build(state);
    stack.add_named(&home_page, Some("home"));
    stack.add_named(&all_page, Some("all"));
    root.append(&stack);

    let bottom = BottomBar::build();
    bottom.set_prompts(&[("A", &crate::tr!("Play"))]);
    root.append(bottom.widget());

    root.set_hexpand(true);
    root.set_valign(gtk4::Align::Fill);
    overlay.add_overlay(&root);

    let ui = BigPictureUi {
        stack,
        home_page,
        all_page,
        bottom,
        page: Cell::new(Page::Home),
        home,
        all,
    };
    (overlay, ui)
}

fn wire_keyboard(state: &SharedState, window: &adw::ApplicationWindow) {
    let state = state.clone();
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, modifiers| {
        match key {
            gdk4::Key::Left => route(&state, NavCommand::Left),
            gdk4::Key::Right => route(&state, NavCommand::Right),
            gdk4::Key::Up => route(&state, NavCommand::Up),
            gdk4::Key::Down => route(&state, NavCommand::Down),
            gdk4::Key::Return | gdk4::Key::KP_Enter | gdk4::Key::space => {
                route(&state, NavCommand::Confirm)
            }
            gdk4::Key::Escape => {
                if showing_all(&state) {
                    show_home(&state);
                } else {
                    quit_app(&state);
                }
            }
            gdk4::Key::q if modifiers.contains(gdk4::ModifierType::CONTROL_MASK) => {
                quit_app(&state)
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(key);
}

/// Messages from the controller reader: navigation for the showing page,
/// and the pad status for the bottom rail's dots and battery.
pub(super) fn handle_msg(state: &SharedState, msg: NavMsg) {
    match msg {
        NavMsg::Nav(command) => route(state, command),
        NavMsg::Pads(status) => {
            if let Some(big) = state.borrow().big_picture.clone() {
                big.bottom
                    .set_pad_status(status.count, status.battery);
            }
        }
    }
}

fn route(state: &SharedState, command: NavCommand) {
    if showing_all(state) {
        match command {
            NavCommand::Left => grid_move(state, -1, 0),
            NavCommand::Right => grid_move(state, 1, 0),
            NavCommand::Up => grid_move(state, 0, -1),
            NavCommand::Down => grid_move(state, 0, 1),
            NavCommand::Confirm => {
                let game = state
                    .borrow()
                    .big_picture
                    .as_ref()
                    .and_then(|big| big.all.selected_game());
                if let Some(game) = game {
                    if let Err(error) =
                        super::play_button::launch_game(state, game.db_id, game.variant_id)
                    {
                        eprintln!("Failed to launch game: {error}");
                        let _ = state
                            .borrow()
                            .sender
                            .send(crate::AppMessage::AddGameError(error));
                    }
                }
            }
            NavCommand::Back => show_home(state),
        }
    } else {
        match command {
            NavCommand::Left => super::big_picture_home::move_selection(state, -1),
            NavCommand::Right => super::big_picture_home::move_selection(state, 1),
            NavCommand::Confirm => confirm(state),
            NavCommand::Up | NavCommand::Down | NavCommand::Back => {}
        }
    }
}

fn showing_all(state: &SharedState) -> bool {
    state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| big.page.get() == Page::AllSoftware)
}

fn grid_move(state: &SharedState, dx: i32, dy: i32) {
    if let Some(big) = state.borrow().big_picture.clone() {
        big.all.move_selection(dx, dy);
    }
}

/// The home screen's Confirm: launch the selected game, or open All
/// Software when the grid tile is selected.
pub(super) fn confirm(state: &SharedState) {
    let is_tile = state
        .borrow()
        .big_picture
        .as_ref()
        .map(super::big_picture_home::selection_is_tile);
    if is_tile == Some(true) {
        open_all(state);
    } else {
        super::big_picture_home::launch_selected(state);
    }
}

pub(super) fn open_all(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    big.page.set(Page::AllSoftware);
    big.stack.set_visible_child(&big.all_page);
    big.bottom.set_prompts(&[("B", &crate::tr!("Back")), ("A", &crate::tr!("Play"))]);
    big.all.ensure_opened();
}

pub(super) fn show_home(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    big.page.set(Page::Home);
    big.stack.set_visible_child(&big.home_page);
    big.bottom.set_prompts(&[("A", &crate::tr!("Play"))]);
}

fn quit_app(state: &SharedState) {
    let window = state.borrow().window.clone();
    if let Some(app) = window.application() {
        app.quit();
    }
}

/// Repopulate both pages from the shared game list. Cheap no-op outside
/// big-picture mode; message handlers call it whenever the game list or its
/// artwork changes.
pub(super) fn refresh(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    super::big_picture_home::refresh(state);
    big.all.refresh(state);
}
