use adw::prelude::*;

fn main() -> glib::ExitCode {
    ira::ui::init_desktop_identity();
    let application = adw::Application::new(
        Some(ira::desktop::protocol::APP_ID),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    application.connect_startup(|_| ira::ui::apply_desktop_program_class());
    application.connect_activate(|application| {
        let window = adw::ApplicationWindow::new(application);
        ira::ui::bind_desktop_window_identity(&window);
        // A buffer-less surface emits the identity protocol without showing a
        // window or changing the user's current application instance.
        gtk4::prelude::WidgetExt::realize(&window);
        window.set_visible(true);
        window.set_visible(false);
        window.set_visible(true);
        gtk4::prelude::WidgetExt::display(&window).flush();
        window.destroy();
        application.quit();
    });
    application.run_with_args(&["ira-desktop-identity-probe"])
}
