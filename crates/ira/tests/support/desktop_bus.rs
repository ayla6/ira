use std::{
    ffi::OsString,
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct PrivateBus {
    process: Child,
    pub address: String,
}

impl PrivateBus {
    pub fn start(root: &Path, overrides: &[(&str, OsString)]) -> Self {
        let config = root.join("bus.conf");
        let services = root.join("data/dbus-1/services");
        std::fs::create_dir_all(&services).unwrap();
        std::fs::write(&config, format!("<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><servicedir>{}</servicedir><policy context='default'><allow own='*'/><allow send_destination='*'/><allow receive_sender='*'/></policy></busconfig>", root.display(), services.display())).unwrap();
        let mut process = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--print-address=1")
            .arg(format!("--config-file={}", config.display()))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("GSETTINGS_BACKEND", "memory")
            .envs(overrides.iter().map(|(key, value)| (*key, value)))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(process.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(
            !address.trim().is_empty(),
            "Private D-Bus did not publish an address"
        );
        Self {
            process,
            address: address.trim().into(),
        }
    }

    pub fn connect(&self) -> gio::DBusConnection {
        gio::DBusConnection::for_address_sync(
            &self.address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        // The test owns this process group, including any activated providers.
        unsafe {
            libc::kill(-(self.process.id() as i32), libc::SIGTERM);
        }
        if let Err(error) = self.process.wait() {
            eprintln!("Wait for test D-Bus: {error}");
        }
    }
}
