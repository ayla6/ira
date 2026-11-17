use glib::variant::ToVariant;
use ira::desktop::tray::{self, TrayAction};
use std::time::{Duration, Instant};

#[path = "support/desktop_bus.rs"]
mod desktop_bus;

fn check_late_watcher(connection: &gio::DBusConnection, name: &str) {
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let registrations = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let node = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.kde.StatusNotifierWatcher"><method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method></interface></node>"#).unwrap();
        let interface = node.lookup_interface("org.kde.StatusNotifierWatcher").unwrap();
        let registration = connection.register_object("/StatusNotifierWatcher", &interface).method_call({
            let registrations = registrations.clone();
            move |_, _, _, _, _, params, invocation| {
                let (name,) = params.get::<(String,)>().unwrap();
                registrations.borrow_mut().push(name);
                invocation.return_value(Some(&().to_variant()));
            }
        }).build().unwrap();
        let own = connection.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
            Some(&("org.kde.StatusNotifierWatcher", 4u32).to_variant()), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE).unwrap();
        assert_eq!(own.get::<(u32,)>(), Some((1,)));
        let deadline = Instant::now() + Duration::from_secs(2);
        while registrations.borrow().is_empty() && Instant::now() < deadline {
            while context.iteration(false) {}
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(*registrations.borrow(), vec![name.to_string()]);
        connection.unregister_object(registration).unwrap();
    }).unwrap();
}

#[test]
fn test_tray_menu_protocol_is_responsive_and_tracks_visibility() {
    let root = tempfile::tempdir().unwrap();
    let bus = desktop_bus::PrivateBus::start(root.path(), &[]);
    // This integration binary has one test; the tray's worker inherits its private bus.
    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus.address);
    let (sender, receiver) = async_channel::bounded(8);
    let handle = tray::start(sender).unwrap();
    let connection = bus.connect();
    let response = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    let (names,) = response.get::<(Vec<String>,)>().unwrap();
    let name = names
        .iter()
        .find(|name| name.starts_with("org.kde.StatusNotifierItem-"))
        .unwrap();
    let introspection = connection
        .call_sync(
            Some(name),
            "/StatusNotifierItem",
            "org.freedesktop.DBus.Introspectable",
            "Introspect",
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    let (xml,) = introspection.get::<(String,)>().unwrap();
    let node = gio::DBusNodeInfo::for_xml(&xml).unwrap();
    let interface = node.lookup_interface("org.kde.StatusNotifierItem").unwrap();
    assert!(interface.lookup_method("Activate").is_none(), "Menu-only trays must not advertise activation: GNOME waits for a double click when they do");
    let properties = connection
        .call_sync(
            Some(name),
            "/StatusNotifierItem",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&("org.kde.StatusNotifierItem",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    let (values,) = properties
        .get::<(std::collections::HashMap<String, glib::Variant>,)>()
        .unwrap();
    assert_eq!(values["ItemIsMenu"].get::<bool>(), Some(true));
    assert_eq!(values["IconPixmap"].n_children(), 3);
    check_late_watcher(&connection, name);
    let call = |method: &str, parameters: glib::Variant| {
        connection
            .call_sync(
                Some(name),
                "/MenuBar",
                "com.canonical.dbusmenu",
                method,
                Some(&parameters),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
            .unwrap()
    };
    let layout = || {
        call(
            "GetLayout",
            (0i32, -1i32, Vec::<String>::new()).to_variant(),
        )
    };
    let started = Instant::now();
    for _ in 0..10 {
        call("AboutToShow", (0i32,).to_variant());
        let text = layout().print(true).to_string();
        assert!(text.contains("Hide to background"));
        assert!(!text.contains("Show Ira"));
        assert!(!text.contains("icon-name"));
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "Static menu requests took {:?}",
        started.elapsed()
    );
    handle.set_visible(false);
    let text = layout().print(true).to_string();
    assert!(text.contains("Show Ira"));
    assert!(!text.contains("Hide to background"));
    // The first menu item retains its ID as its label and action change.
    let response = layout();
    let children = response.child_value(1).child_value(2);
    let first = children.child_value(0).as_variant().unwrap();
    let id = first.child_value(0).get::<i32>().unwrap();
    call(
        "Event",
        (id, "clicked", 0i32.to_variant(), 0u32).to_variant(),
    );
    assert_eq!(receiver.recv_blocking().unwrap(), TrayAction::Show);
}
