use super::state::SharedState;
use crate::desktop::{
    protocol::*,
    requests::{parse_request, DesktopRequest},
};
use adw::prelude::*;
use glib::translate::ToGlibPtr;
use std::{cell::RefCell, rc::Rc};

pub fn connect(app: &adw::Application, holder: Rc<RefCell<Option<SharedState>>>) {
    for (name, description) in [
        (
            LAUNCH_OPTION,
            crate::tr!("Launch a saved game by its numeric ID"),
        ),
        (VARIANT_OPTION, crate::tr!("Launch a specific game variant")),
        (SEARCH_OPTION, crate::tr!("Search the game library")),
    ] {
        app.add_main_option(
            name,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::String,
            &description,
            None,
        );
    }
    for (name, description) in [
        (
            PROVIDER_OPTION,
            crate::tr!("Run the headless GNOME search provider"),
        ),
        (
            REGISTER_OPTION,
            crate::tr!("Enable desktop integration for this user"),
        ),
        (
            UNREGISTER_OPTION,
            crate::tr!("Disable desktop integration for this user"),
        ),
    ] {
        app.add_main_option(
            name,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            &description,
            None,
        );
    }
    app.connect_command_line(move |app, command| {
        let options = command.options_dict();
        let request = (|| {
            let game = options
                .lookup::<String>(LAUNCH_OPTION)
                .map_err(|e| e.to_string())?;
            let variant = options
                .lookup::<String>(VARIANT_OPTION)
                .map_err(|e| e.to_string())?;
            let search = options
                .lookup::<String>(SEARCH_OPTION)
                .map_err(|e| e.to_string())?;
            parse_request(game.as_deref(), variant.as_deref(), search.as_deref())
        })();
        match request {
            Ok(request) => {
                let launch = matches!(request, Some(DesktopRequest::Launch { .. }));
                if launch {
                    if holder.borrow().is_none() {
                        let state = crate::activate::activate(app);
                        state.borrow_mut().content_unloaded = true;
                        *holder.borrow_mut() = Some(state);
                    }
                } else {
                    app.activate();
                }
                let state = holder.borrow().clone();
                if let (Some(state), Some(request)) = (state, request) {
                    submit(&state, request);
                }
                glib::ExitCode::SUCCESS
            }
            Err(error) => {
                let message = glib::GString::from(format!("Ira: {error}\n"));
                // A fixed format forwards errors to the invoking process on older GLib too.
                unsafe {
                    gio::ffi::g_application_command_line_printerr(
                        command.to_glib_none().0,
                        c"%s".as_ptr(),
                        message.as_ptr(),
                    );
                }
                glib::ExitCode::FAILURE
            }
        }
    });
}

pub(super) fn submit(state: &SharedState, request: DesktopRequest) {
    let requests = state.borrow_mut().desktop_requests.submit(request);
    match requests {
        Ok(requests) => requests
            .into_iter()
            .for_each(|request| dispatch(state, request)),
        Err(error) => show_error(state, &error),
    }
}

pub(super) fn library_ready(state: &SharedState) {
    let requests = state.borrow_mut().desktop_requests.library_ready();
    requests
        .into_iter()
        .for_each(|request| dispatch(state, request));
}

fn dispatch(state: &SharedState, request: DesktopRequest) {
    match request {
        DesktopRequest::Search(text) => {
            let entry = state.borrow().search_entry.clone();
            entry.set_text(&text);
        }
        DesktopRequest::Launch {
            game_id,
            variant_id,
        } => {
            let db = state.borrow().db.clone();
            let weak = Rc::downgrade(state);
            glib::MainContext::default().spawn_local(async move {
                let result = gio::spawn_blocking(move || {
                    if let Some(id) = variant_id {
                        let variants = ira_db::get_variants(&db, game_id)?;
                        if !variants.iter().any(|variant| variant.id == id) {
                            return Err("Variant does not belong to this game".to_string());
                        }
                        Ok(Some(id))
                    } else {
                        ira_db::get_default_variant(&db, game_id)
                    }
                })
                .await;
                let Some(state) = weak.upgrade() else { return };
                let result = result
                    .map_err(|_| "Desktop launch worker panicked".to_string())
                    .and_then(|result| result)
                    .and_then(|variant| super::play_button::launch_game(&state, game_id, variant));
                if let Err(error) = result {
                    show_error(&state, &error);
                }
            });
        }
    }
}

pub(super) fn show_error(state: &SharedState, error: &str) {
    eprintln!("Desktop action failed: {error}");
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some(&crate::tr!("Desktop action failed")), Some(error));
    dialog.add_response("close", &crate::tr!("Close"));
    dialog.present(Some(&window));
}
