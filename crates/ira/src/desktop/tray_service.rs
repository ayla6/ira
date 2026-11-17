use super::{tray::IraTray, tray_protocol as protocol};
use glib::variant::ToVariant;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc,
    },
};

pub(super) struct Handle {
    context: glib::MainContext,
    main_loop: glib::MainLoop,
    connection: gio::DBusConnection,
    revision: Arc<AtomicU32>,
}

impl Handle {
    pub(super) fn update(&self) {
        let connection = self.connection.clone();
        let revision = self
            .revision
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.context.invoke(move || {
            if let Err(error) = connection.emit_signal(
                None,
                protocol::MENU_PATH,
                protocol::MENU,
                "LayoutUpdated",
                Some(&(revision, 0i32).to_variant()),
            ) {
                eprintln!("Update tray menu: {error}");
            }
        });
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let main_loop = self.main_loop.clone();
        self.context.invoke(move || main_loop.quit());
    }
}

pub(super) fn start(tray: IraTray) -> Result<Handle, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ira-tray".into())
        .spawn(move || {
            let context = glib::MainContext::new();
            let result = context.with_thread_default(|| run(&context, tray, &sender));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("Tray service: {error}");
                    let _ = sender.send(Err(error));
                }
                Err(error) => {
                    eprintln!("Tray context: {error}");
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| error.to_string())?
}

fn connection() -> Result<gio::DBusConnection, String> {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| error.to_string())?;
    gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .map_err(|error| error.to_string())
}

fn register(
    connection: &gio::DBusConnection,
    tray: IraTray,
    revision: Arc<AtomicU32>,
) -> Result<(), String> {
    let tray = Rc::new(RefCell::new(tray));
    let item = gio::DBusNodeInfo::for_xml(protocol::ITEM_XML)
        .map_err(|error| error.to_string())?
        .lookup_interface(protocol::ITEM)
        .ok_or("Missing tray interface")?;
    connection
        .register_object(protocol::ITEM_PATH, &item)
        .property({
            let tray = tray.clone();
            move |_, _, _, _, name| protocol::item_property(&tray.borrow(), name)
        })
        .build()
        .map_err(|error| error.to_string())?;
    let menu = gio::DBusNodeInfo::for_xml(protocol::MENU_XML)
        .map_err(|error| error.to_string())?
        .lookup_interface(protocol::MENU)
        .ok_or("Missing tray menu interface")?;
    connection
        .register_object(protocol::MENU_PATH, &menu)
        .property(|_, _, _, _, name| protocol::menu_property(name))
        .method_call(move |_, _, _, _, method, params, invocation| {
            match protocol::menu_call(
                &mut tray.borrow_mut(),
                revision.load(Ordering::Relaxed),
                method,
                &params,
            ) {
                Ok(reply) => invocation.return_value(Some(&reply)),
                Err(error) => invocation.return_dbus_error("com.canonical.dbusmenu.Error", &error),
            }
        })
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn run(
    context: &glib::MainContext,
    tray: IraTray,
    ready: &mpsc::SyncSender<Result<Handle, String>>,
) -> Result<(), String> {
    let connection = connection()?;
    let revision = Arc::new(AtomicU32::new(1));
    register(&connection, tray, revision.clone())?;
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let reply = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name.clone(), 4u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| error.to_string())?;
    if reply.get::<(u32,)>() != Some((1,)) {
        return Err("Tray bus name is already owned".into());
    }
    let watcher = gio::bus_watch_name_on_connection(
        &connection,
        "org.kde.StatusNotifierWatcher",
        gio::BusNameWatcherFlags::NONE,
        move |connection, _, _| {
            connection.call(
                Some("org.kde.StatusNotifierWatcher"),
                "/StatusNotifierWatcher",
                "org.kde.StatusNotifierWatcher",
                "RegisterStatusNotifierItem",
                Some(&(name.clone(),).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
                |result| {
                    if let Err(error) = result {
                        eprintln!("Register tray: {error}");
                    }
                },
            );
        },
        |_, _| {},
    );
    let main_loop = glib::MainLoop::new(Some(context), false);
    ready
        .send(Ok(Handle {
            context: context.clone(),
            main_loop: main_loop.clone(),
            connection: connection.clone(),
            revision,
        }))
        .map_err(|error| error.to_string())?;
    main_loop.run();
    gio::bus_unwatch_name(watcher);
    connection
        .close_sync(gio::Cancellable::NONE)
        .map_err(|error| error.to_string())
}
