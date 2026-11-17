use super::{protocol::*, search_methods};
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

mod imp {
    use super::*;
    use gio::subclass::prelude::*;

    #[derive(Default)]
    pub struct SearchApplication {
        registration: RefCell<Option<gio::RegistrationId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchApplication {
        const NAME: &'static str = "IraSearchApplication";
        type Type = super::SearchApplication;
        type ParentType = gio::Application;
    }

    impl ObjectImpl for SearchApplication {}

    impl ApplicationImpl for SearchApplication {
        fn dbus_register(
            &self,
            connection: &gio::DBusConnection,
            path: &str,
        ) -> Result<(), glib::Error> {
            self.parent_dbus_register(connection, path)?;
            // Export before GApplication owns its name, so the first activated
            // request cannot reach an object that only has the base interfaces.
            let registration = super::register(connection, self.obj().upcast_ref())
                .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error))?;
            *self.registration.borrow_mut() = Some(registration);
            Ok(())
        }

        fn dbus_unregister(&self, connection: &gio::DBusConnection, path: &str) {
            if let Some(registration) = self.registration.borrow_mut().take() {
                if let Err(error) = connection.unregister_object(registration) {
                    eprintln!("Unregister search provider: {error}");
                }
            }
            self.parent_dbus_unregister(connection, path);
        }
    }
}

glib::wrapper! {
    pub struct SearchApplication(ObjectSubclass<imp::SearchApplication>)
        @extends gio::Application, @implements gio::ActionGroup, gio::ActionMap;
}

pub fn run() -> glib::ExitCode {
    crate::i18n::init();
    let application: SearchApplication = glib::Object::builder()
        .property("application-id", PROVIDER_ID)
        .property("flags", gio::ApplicationFlags::IS_SERVICE)
        .build();
    application.set_inactivity_timeout(30_000);
    application.run_with_args(&["ira-search-provider"])
}

fn register(
    connection: &gio::DBusConnection,
    application: &gio::Application,
) -> Result<gio::RegistrationId, String> {
    let node = gio::DBusNodeInfo::for_xml(include_str!("search-provider.xml"))
        .map_err(|error| error.to_string())?;
    let interface = node
        .lookup_interface(PROVIDER_INTERFACE)
        .ok_or_else(|| "Missing search-provider interface".to_string())?;
    let weak = application.downgrade();
    let active = Rc::new(Cell::new(0));
    connection
        .register_object(PROVIDER_PATH, &interface)
        .method_call(move |_, _, _, _, method, parameters, invocation| {
            let Some(application) = weak.upgrade() else {
                return;
            };
            if active.get() >= 16 {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.LimitsExceeded",
                    "Too many active searches",
                );
                return;
            }
            active.set(active.get() + 1);
            let active = active.clone();
            let hold = application.hold();
            let method = method.to_string();
            glib::MainContext::default().spawn_local(async move {
                let result =
                    gio::spawn_blocking(move || search_methods::handle(&method, &parameters))
                        .await
                        .map_err(|_| "Search worker panicked".to_string())
                        .and_then(|result| result);
                match result {
                    Ok(reply) => invocation.return_value(Some(&search_methods::reply_value(reply))),
                    Err(error) => {
                        eprintln!("Desktop search: {error}");
                        invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", &error);
                    }
                }
                active.set(active.get() - 1);
                drop(hold);
            });
        })
        .build()
        .map_err(|error| error.to_string())
}
