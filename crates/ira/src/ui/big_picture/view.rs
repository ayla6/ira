//! The big-picture shell: a status rail floating top-right, one page in
//! the middle (Recent / Everything / Groups tabs over a shared header),
//! and a bottom rail (connected gamepads, button prompts). Controller and
//! keyboard input is routed to whichever tab is showing.

use super::all_games::{AllSoftwareUi, Tab};
use super::home::HomeUi;
use super::input::{NavCommand, NavMsg};
use super::status::{BottomBar, StatusBar};
use crate::ui::css::*;
use crate::ui::state::SharedState;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// Widgets of the big-picture view, kept on `AppState` behind an `Rc` so every
/// handler — refreshes, navigation, the scroll ticker — sees the same
/// selection state instead of a deep-cloned snapshot.
pub struct BigPictureUi {
    status: StatusBar,
    bottom: BottomBar,
    /// The big-picture scale the pills were last measured at, so a scale change
    /// can force their stale label layouts to re-resolve.
    ui_scale: Cell<f64>,
    /// Whether the pointer is currently hidden because a gamepad or the
    /// keyboard is driving the UI (see `big_picture_mouse`).
    pub(super) cursor_hidden: Cell<bool>,
    pub(super) home: HomeUi,
    pub(super) all: AllSoftwareUi,
    /// The per-game options menu (groups, sorting); topmost when open.
    pub(super) game_menu: super::game_menu::GameMenu,
    /// The virtual keyboard, topmost over everything while open.
    pub(super) keyboard: super::keyboard::Keyboard,
    /// The black "game running" screen, over everything while a game runs.
    pub(super) running: super::running::RunningOverlay,
    /// The centered disc picker, over everything while choosing a disc.
    pub(super) disc_picker: super::disc_picker::DiscPicker,
}

/// Build the big-picture window, fullscreened before its first present, and
/// take over the shared state's window reference.
pub(crate) fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    super::super::desktop_identity::bind(&window);
    super::super::tray::watch_window(&window, state);
    window.set_title(Some(&crate::tr!("Ira")));
    window.set_size_request(900, 650);

    // The window has no width yet here, and a 0-scale sheet would floor
    // every big-picture font to 1px — start from the 1080p reference instead;
    // the first refresh swaps in the real scale.
    crate::ui::css::init_styles(big_picture_scale(state).max(1.0));
    ira_widgets::font::pin_unhinted(true);

    let square_mode = state.borrow().cfg.big_picture_square_capsules;
    let (root, ui) = build_root(state, square_mode);
    // Key events land on the toplevel whenever nothing else holds focus, so
    // the keyboard handler lives there rather than on a child widget.
    wire_keyboard(state, &window);
    // The pointer returns on mouse motion and hides on gamepad/keyboard
    // navigation; hovering a cover selects it.
    super::mouse::attach(state, root.upcast_ref());
    // The desktop window's close wiring never runs in this mode, so honor
    // the close-to-background setting here: without it a compositor close
    // would destroy the window and leave the process running headless.
    {
        let close_state = state.clone();
        window.connect_close_request(move |_| {
            let close_to_background = close_state.borrow().cfg.close_to_background;
            if close_to_background {
                crate::ui::background::hide_to_background(&close_state);
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
    // Fullscreen BEFORE the first present: presenting at the default size
    // first would lay the whole shell out small and resize it a frame
    // later — a transitional stage the floats can anchor to and then sit
    // at until something moves. Requested while unmapped, the fullscreen
    // state applies at the very first layout instead.
    window.fullscreen();
    window.present();

    // Rails' icons, prompts and fonts scale with the viewport, and the
    // real size only exists a frame or two after present. Scale then —
    // not when the first games load, or a 4K screen spends the whole
    // library load with 1080p-reference icons.
    let tick_state = state.clone();
    window.add_tick_callback(move |_, _| {
        if tick_state.borrow().window.width() == 0 {
            return glib::ControlFlow::Continue;
        }
        refresh(&tick_state);
        glib::ControlFlow::Break
    });
    refresh(state);
    // The page is up from the start now — apply the tab's prompts and
    // header labels before the first frame.
    if let Some(big) = state.borrow().big_picture.clone() {
        big.all.apply_mode(state);
    }
    super::input::start(state);
}

fn build_root(state: &SharedState, square_mode: bool) -> (gtk4::Overlay, BigPictureUi) {
    let overlay = gtk4::Overlay::new();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class(CSS_BP_ROOT);

    let status = StatusBar::build();

    let (home_page, home) = super::home::build(state, square_mode);
    let (all_page, all) = super::all_games::build(state, &home_page);
    all_page.set_vexpand(true);
    root.append(&all_page);

    let bottom = BottomBar::build();
    bottom.set_prompts(&[(ira_input::GamepadButton::A, &crate::tr!("Play"))]);
    root.append(bottom.widget());

    root.set_hexpand(true);
    root.set_valign(gtk4::Align::Fill);
    overlay.add_overlay(&root);

    // The status rail floats over the top-right on every page: on All
    // Software it shares the header's line (that side stays empty), on
    // home it hangs over the carousel. It is a sibling of the bp-root
    // Box, so it carries that class itself for the big-picture font. It
    // must be under the menus, so it is added before them.
    let status_widget = status.widget();
    status_widget.add_css_class(CSS_BP_ROOT);
    status_widget.set_halign(gtk4::Align::End);
    status_widget.set_valign(gtk4::Align::Start);
    overlay.add_overlay(status_widget);
    overlay.set_measure_overlay(status_widget, false);

    let game_menu = super::game_menu::GameMenu::new(state);
    overlay.add_overlay(game_menu.root());
    overlay.set_measure_overlay(game_menu.root(), false);
    let keyboard = super::keyboard::Keyboard::new(state);
    overlay.add_overlay(keyboard.root());
    overlay.set_measure_overlay(keyboard.root(), false);
    // The running screen sits over everything, menus included: while a
    // game runs there is nothing else to reach for.
    let running = super::running::RunningOverlay::new();
    overlay.add_overlay(running.root());
    overlay.set_measure_overlay(running.root(), false);
    // The disc picker sits over everything but the running screen: a
    // launch never overlaps a running game.
    let disc_picker = super::disc_picker::DiscPicker::build();
    overlay.add_overlay(disc_picker.root());
    overlay.set_measure_overlay(disc_picker.root(), false);

    let ui = BigPictureUi {
        status,
        bottom,
        ui_scale: Cell::new(0.0),
        cursor_hidden: Cell::new(false),
        home,
        all,
        game_menu,
        keyboard,
        running,
        disc_picker,
    };
    (overlay, ui)
}

impl BigPictureUi {
    /// Swap the bottom rail's button prompts (kept here so page modules
    /// don't reach into the rail's fields).
    pub(super) fn set_prompts(&self, items: &[(ira_input::GamepadButton, &str)]) {
        self.bottom.set_prompts(items);
    }
}

fn wire_keyboard(state: &SharedState, window: &adw::ApplicationWindow) {
    let state = state.clone();
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    // GDK reports auto-repeat as more presses; a key already down is a
    // hold, which must behave like a held stick (no edge wraps).
    let held: Rc<RefCell<HashSet<gdk4::Key>>> = Rc::default();
    {
        let held = held.clone();
        key.connect_key_released(move |_, key, _, _| {
            held.borrow_mut().remove(&key);
        });
    }
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let fresh = held.borrow_mut().insert(key);
        // While the virtual keyboard shows, physical typing goes straight
        // into it; navigation keys still fall through to the router.
        let keyboard_open = state
            .borrow()
            .big_picture
            .as_ref()
            .is_some_and(|big| big.keyboard.is_open());
        if keyboard_open {
            if key == gdk4::Key::BackSpace {
                if let Some(big) = state.borrow().big_picture.clone() {
                    big.keyboard.backspace();
                }
                return glib::Propagation::Stop;
            }
            if let Some(ch) = key.to_unicode() {
                if !ch.is_control() {
                    if let Some(big) = state.borrow().big_picture.clone() {
                        big.keyboard.type_char(ch);
                    }
                    return glib::Propagation::Stop;
                }
            }
        }
        match key {
            gdk4::Key::Left => route(&state, NavCommand::Left, fresh),
            gdk4::Key::Right => route(&state, NavCommand::Right, fresh),
            gdk4::Key::Up => route(&state, NavCommand::Up, fresh),
            gdk4::Key::Down => route(&state, NavCommand::Down, fresh),
            // Held Enter or Escape repeats nothing: one launch, one peel.
            gdk4::Key::Return | gdk4::Key::KP_Enter | gdk4::Key::space | gdk4::Key::Escape
                if !fresh => {}
            gdk4::Key::Return | gdk4::Key::KP_Enter | gdk4::Key::space => {
                route(&state, NavCommand::Confirm, true)
            }
            gdk4::Key::Escape => {
                // Escape peels overlays first: keyboard, then menu, then
                // the disc picker; with nothing open it opens the Home
                // menu. It never quits the app by itself.
                let keyboard_open = state
                    .borrow()
                    .big_picture
                    .as_ref()
                    .is_some_and(|big| big.keyboard.is_open());
                let menu_open = state
                    .borrow()
                    .big_picture
                    .as_ref()
                    .is_some_and(|big| big.game_menu.is_open());
                let picker_open = state
                    .borrow()
                    .big_picture
                    .as_ref()
                    .is_some_and(|big| big.disc_picker.is_open());
                if keyboard_open {
                    if let Some(big) = state.borrow().big_picture.clone() {
                        big.keyboard.close(&state);
                    }
                } else if menu_open {
                    if let Some(big) = state.borrow().big_picture.clone() {
                        big.game_menu.close();
                    }
                } else if picker_open {
                    if let Some(big) = state.borrow().big_picture.clone() {
                        big.disc_picker.close();
                    }
                } else {
                    // Nothing to peel: the keyboard's Home button.
                    route(&state, NavCommand::Home, true);
                }
            }
            // The keyboard's B and Start (the virtual keyboard took its
            // own Backspace above).
            gdk4::Key::BackSpace if fresh => route(&state, NavCommand::Back, true),
            gdk4::Key::Menu if fresh => route(&state, NavCommand::Options, true),
            // The keyboard's Y: New Group on the Groups tiles.
            gdk4::Key::n | gdk4::Key::N if fresh => route(&state, NavCommand::Quaternary, true),
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
/// and the pad count for the bottom rail's dots.
pub(super) fn handle_msg(state: &SharedState, msg: NavMsg) {
    match msg {
        NavMsg::Nav(command, engage) => route(state, command, engage),
        NavMsg::Pads(status) => {
            if let Some(big) = state.borrow().big_picture.clone() {
                big.bottom.set_pad_status(status.count, status.family);
                big.all.set_shoulder_family(status.family);
                big.keyboard.set_pad_family(status.family);
            }
        }
    }
}

fn route(state: &SharedState, command: NavCommand, engage: bool) {
    let mouse_drove = super::mouse::note_controller_use(state);
    // While the black running screen covers the shell the library is
    // not navigable: B cancels the launch (stops the starting game)
    // and every other input waits for the session to exit.
    if state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| big.is_running_showing())
    {
        if command == NavCommand::Back && engage {
            cancel_running(state);
        }
        return;
    }
    if command == NavCommand::Home {
        toggle_home_menu(state);
        return;
    }
    // The virtual keyboard swallows navigation while it is open: arrows
    // walk the keys, Confirm types, B deletes, Options cancels.
    if route_keyboard(state, command) {
        return;
    }
    // A held B must not pop page after page (or quit the app): only the
    // keyboard's backspace consumes Back repeats, and that arm has
    // already returned above.
    if command == NavCommand::Back && !engage {
        return;
    }
    // The options menu swallows navigation while it is open.
    {
        let menu_open = state
            .borrow()
            .big_picture
            .as_ref()
            .is_some_and(|big| big.game_menu.is_open());
        if menu_open {
            let Some(big) = state.borrow().big_picture.clone() else {
                return;
            };
            match command {
                NavCommand::Up => big.game_menu.move_selection(state, -1),
                NavCommand::Down => big.game_menu.move_selection(state, 1),
                NavCommand::Confirm => big.game_menu.activate(state),
                NavCommand::Secondary => big.game_menu.activate_secondary(state),
                NavCommand::Back | NavCommand::Options => big.game_menu.close(),
                _ => {}
            }
            return;
        }
    }
    // The disc picker swallows navigation while it is open.
    {
        let picker_open = state
            .borrow()
            .big_picture
            .as_ref()
            .is_some_and(|big| big.disc_picker.is_open());
        if picker_open {
            let Some(big) = state.borrow().big_picture.clone() else {
                return;
            };
            match command {
                NavCommand::Left | NavCommand::Up => big.disc_picker.move_selection(-1),
                NavCommand::Right | NavCommand::Down => big.disc_picker.move_selection(1),
                NavCommand::Confirm => big.disc_picker.activate(state),
                NavCommand::Back => big.disc_picker.close(),
                _ => {}
            }
            return;
        }
    }
    // Grab the shell without holding the state borrow: tab switches
    // below refresh auto membership mutably, and GTK handlers routinely
    // run reentrantly under an outstanding borrow.
    let big = state.borrow().big_picture.clone();
    let Some(big) = big else {
        return;
    };
    // The shoulders change tabs from every tab, Recent included — on
    // fresh presses only: a held shoulder sweeping tabs is strobe food.
    match command {
        NavCommand::PrevTab if engage => big.all.switch_tab(state, -1),
        NavCommand::NextTab if engage => big.all.switch_tab(state, 1),
        _ => {}
    }
    if big.all.tab() == Tab::Recent {
        // The recent carousel: left/right walk the covers, A plays, and
        // the arrow tile at the end jumps to the Everything tab. Only a
        // fresh press on an edge tile wraps around; held repeats hit a
        // wall there instead of cycling forever.
        match command {
            NavCommand::Left => super::home::move_selection(state, -1, engage),
            NavCommand::Right => super::home::move_selection(state, 1, engage),
            NavCommand::Confirm => {
                if super::home::selection_is_tile(&big) {
                    big.all.set_tab(state, Tab::Everything);
                } else {
                    super::home::launch_selected(state);
                }
            }
            NavCommand::Up
            | NavCommand::Down
            | NavCommand::Back
            | NavCommand::Options
            | NavCommand::Secondary
            | NavCommand::Tertiary
            | NavCommand::PrevTab
            | NavCommand::NextTab
            | NavCommand::Quaternary
            | NavCommand::Home => {}
        }
        return;
    }
    // First directional press while the pointer was still in charge:
    // its focused tile yields entirely, and the arrows re-acquire
    // from whatever is on screen (see `AllSoftwareUi::move_selection`).
    if mouse_drove
        && matches!(
            command,
            NavCommand::Left | NavCommand::Right | NavCommand::Up | NavCommand::Down
        )
    {
        big.all.clear_selection();
    }
    match command {
        NavCommand::Back => {
            // Only a group's game view has somewhere to go back to; the
            // root tabs show no back affordance at all.
            big.all.on_back(state);
        }
        _ if big.all.in_groups_tiles() => match command {
            NavCommand::Quaternary => big.all.groups_new(state),
            NavCommand::Secondary => big
                .game_menu
                .open(state, super::game_menu::MenuKind::GroupOrder),
            NavCommand::Options => big.all.groups_settings_selected(state),
            NavCommand::Left => big.all.groups_move(state, -1, 0, engage),
            NavCommand::Right => big.all.groups_move(state, 1, 0, engage),
            NavCommand::Up => big.all.groups_move(state, 0, -1, engage),
            NavCommand::Down => big.all.groups_move(state, 0, 1, engage),
            NavCommand::Confirm => big.all.groups_open_selected(state),
            _ => {}
        },
        NavCommand::Options => {
            // Options on a focused game opens its group menu; with
            // nothing focused it opens the sort picker.
            let game = big.all.selected_game();
            match game {
                Some(game) => big
                    .game_menu
                    .open(state, super::game_menu::MenuKind::Groups(Box::new(game))),
                None => big.game_menu.open(state, super::game_menu::MenuKind::Sort),
            }
        }
        // X opens the sort picker everywhere (the groups tiles get their
        // own tile order): Options alone can't reach sorting once a game
        // has focus.
        NavCommand::Secondary => {
            big.game_menu.open(state, super::game_menu::MenuKind::Sort);
        }
        NavCommand::Left => big.all.move_selection(state, -1, 0, engage),
        NavCommand::Right => big.all.move_selection(state, 1, 0, engage),
        NavCommand::Up => big.all.move_selection(state, 0, -1, engage),
        NavCommand::Down => big.all.move_selection(state, 0, 1, engage),
        NavCommand::Confirm => {
            let game = big.all.selected_game();
            if let Some(game) = game {
                if let Err(error) =
                    crate::ui::play_button::launch_game(state, game.db_id, game.variant_id)
                {
                    eprintln!("Failed to launch game: {error}");
                    let _ = state
                        .borrow()
                        .sender
                        .send(crate::AppMessage::GameLaunchFailed {
                            db_id: game.db_id,
                            error,
                        });
                }
            }
        }
        _ => {}
    }
}

/// Cancel a launch still behind the black running screen: B while the
/// session is starting drops it back to the library. Only starting
/// sessions (no pid yet) cancel — once the game owns its pid, B
/// belongs to the game, not the shell.
fn cancel_running(state: &SharedState) {
    let starting: Vec<i64> = state
        .borrow()
        .running_games
        .lock()
        .map(|games| {
            games
                .iter()
                .filter(|(_, pid)| **pid <= 0)
                .map(|(db_id, _)| *db_id)
                .collect()
        })
        .unwrap_or_default();
    if starting.is_empty() {
        return;
    }
    for db_id in starting {
        crate::ui::play_button::stop_game(state, db_id);
    }
    if let Some(big) = state.borrow().big_picture.clone() {
        big.hide_running();
    }
}

/// Route one nav command to the virtual keyboard while it is open:
/// arrows walk the keys, Confirm types, B deletes, Options cancels.
/// True when the keyboard consumed the command. Stays ahead of the
/// Back-repeat guard — the keyboard's own backspace consumes Back
/// repeats.
fn route_keyboard(state: &SharedState, command: NavCommand) -> bool {
    if !state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| big.keyboard.is_open())
    {
        return false;
    }
    let Some(big) = state.borrow().big_picture.clone() else {
        return true;
    };
    match command {
        NavCommand::Up => big.keyboard.move_cursor(0, -1),
        NavCommand::Down => big.keyboard.move_cursor(0, 1),
        NavCommand::Left => big.keyboard.move_cursor(-1, 0),
        NavCommand::Right => big.keyboard.move_cursor(1, 0),
        NavCommand::Confirm => big.keyboard.press_selected(state),
        NavCommand::Back => big.keyboard.backspace(),
        // The shoulders move the text caret itself.
        NavCommand::PrevTab => big.keyboard.move_caret(-1),
        NavCommand::NextTab => big.keyboard.move_caret(1),
        // Start commits, X cancels, L3 shifts the next letter.
        NavCommand::Options => big.keyboard.press_ok(state),
        NavCommand::Secondary => big.keyboard.close(state),
        NavCommand::Tertiary => big.keyboard.bump_shift(state),
        // Routed before the keyboard ever sees it.
        NavCommand::Home => {}
        NavCommand::Quaternary => {}
    }
    true
}

/// The Home button's menu (leave big picture, quit): opens over whatever
/// is showing — closing the keyboard and disc picker it would sit on —
/// and a second press closes it.
fn toggle_home_menu(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    if big.game_menu.is_home_open() {
        big.game_menu.close();
        return;
    }
    if big.keyboard.is_open() {
        big.keyboard.close(state);
    }
    if big.disc_picker.is_open() {
        big.disc_picker.close();
    }
    big.game_menu.open(state, super::game_menu::MenuKind::Home);
}

/// The big-picture UI's viewport scale (1.0 = 1920 wide); 0 before the window
/// has a size.
fn big_picture_scale(state: &SharedState) -> f64 {
    state.borrow().window.width() as f64 / 1920.0
}

pub(super) fn quit_app(state: &SharedState) {
    let window = state.borrow().window.clone();
    if let Some(app) = window.application() {
        app.quit();
    }
}

/// Open the virtual keyboard to name a new group. With `game`, the fresh
/// group takes the game in as its first member.
pub(super) fn name_new_group(state: &SharedState, game: Option<crate::Game>) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    big.keyboard.open(
        state,
        &crate::tr!("Name the group"),
        "",
        false,
        Box::new(move |state, name| {
            let db = state.borrow().db.clone();
            let Ok(id) = ira_db::create_group(&db, name) else {
                return;
            };
            if let Some(game) = &game {
                if let Err(e) = ira_db::add_game_to_group(&db, game.db_id, id) {
                    eprintln!("Failed to add game to group: {e}");
                }
            }
            finish_group_change(state, id);
        }),
    );
}

/// Open the virtual keyboard to rename an existing group.
pub(super) fn rename_group(state: &SharedState, group_id: i64, current: &str) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    big.keyboard.open(
        state,
        &crate::tr!("Rename the group"),
        current,
        false,
        Box::new(move |state, name| {
            if let Err(e) = write_group_name(state, group_id, name) {
                eprintln!("Failed to rename group: {e}");
            }
            finish_group_change(state, group_id);
        }),
    );
}

/// Rename a manual group, or an auto group (negated id) with its rules
/// kept as they are.
fn write_group_name(state: &SharedState, group_id: i64, name: &str) -> Result<(), String> {
    let db = state.borrow().db.clone();
    if group_id >= 0 {
        return ira_db::rename_group(&db, group_id, name);
    }
    let root = state
        .borrow()
        .auto_groups
        .iter()
        .find(|g| g.id == group_id)
        .map(|g| g.root.clone())
        .ok_or_else(|| format!("auto group {group_id} is gone"))?;
    let db_id = super::super::auto_groups::to_db_id(group_id);
    ira_db::update_auto_group(&db, db_id, name, &root)
}

/// Sync the shared groups and redraw the Groups tiles after a change,
/// landing the selection on the touched group.
fn finish_group_change(state: &SharedState, group_id: i64) {
    // Clone out of the borrow: sync_and_focus_group mutably borrows the
    // state to store the refreshed lists, and an if-let keeps its
    // scrutinee borrow for the whole block.
    let big = state.borrow().big_picture.clone();
    if let Some(big) = big {
        big.all.sync_and_focus_group(state, group_id);
    }
}

/// Repopulate both pages from the shared game list. Cheap no-op outside
/// big-picture mode; message handlers call it whenever the game list or its
/// artwork changes.
pub(crate) fn refresh(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    // Big-picture sizes scale with the viewport (1.0 = 1920 wide), which also
    // cancels desktop display scaling: logical pixels shrink as the scale
    // grows, and everything follows. The stylesheet only reloads when the
    // scale really moved: refresh runs on every game-list update — and from
    // the carousel's adjustment, mid-layout — and reloading the CSS would
    // re-style the whole widget tree there, leaving widgets measured and
    // allocated against half-applied styles.
    let scale = big_picture_scale(state);
    if scale > 0.01 && (big.ui_scale.get() - scale).abs() > 0.001 {
        big.ui_scale.set(scale);
        crate::ui::css::init_styles(scale);
        big.status.set_icon_scale(scale);
        big.bottom.set_icon_scale(scale);
        big.all.set_icon_scale(scale);
        // A pill whose text was set at the old font keeps measuring its
        // stale layout; force a re-measure when the scale actually moved.
        big.all.revalidate_tooltip();
        super::home::revalidate_pill(&big);
    }
    if big.all.tab() == Tab::Recent {
        super::home::refresh(state);
    }
    big.all.refresh(state);
}
