# Packaging Ira

Ira is a game library and launcher spanning PC games and console emulators,
with controller support, artwork, playtime, and achievement tracking.

## AppImage

Run from the repository root, inside the development container:

```sh
distrobox enter rust-dev -- env LINUXDEPLOY=/absolute/path/to/linuxdeploy-x86_64.AppImage bash contrib/appimage.sh
```

Obtain linuxdeploy from its [official releases](https://github.com/linuxdeploy/linuxdeploy/releases).
The build needs `file`, `glib-compile-schemas`, `readelf`, `secret-tool`, and
the normal Rust/GTK development dependencies. GTK builds using Glycin also
need its loaders and `bwrap`. Install `gettext` to compile any `po/*.po` catalogs.
The linuxdeploy output plugin downloads an AppImage runtime on first use.

Output: `target/appimage/Ira-0.1.0-x86_64.AppImage` and a SHA-256 checksum.
The version and architecture follow the build. Each run stages a new AppDir
under `target/appimage/`, leaving source binaries untouched. Set `SKIP_BUILD=1`
to package existing release binaries, or `IRA_BUILD_DIR` to select their directory.

The bundle contains GTK/libadwaita, private input/overlay helpers, artwork,
schemas, MIME data, image loaders, keyring tooling, and installed translations.
There are currently no translated `.po` catalogs in the repository.
ELF RPATH locates private libraries without adding `LD_LIBRARY_PATH` to games'
environments. Glycin's executable paths are relocated at launch while retaining
its sandbox. User settings, libraries, saves, and keys remain outside the image.

### Compatibility and host integration

- The current Fedora 44 build uses **glibc 2.43**. It targets systems with glibc
  2.43 or newer; it is **not** a universal binary for older Linux distributions.
  For a broader release, build against the oldest supported userspace that
  supplies GTK >= 4.14 and libadwaita >= 1.7, then test that distro explicitly.
- GPU drivers, Wine/Proton, emulators, a Secret Service keyring, desktop portals,
  and device permissions are host integrations, not bundled game runtimes.
- Normal execution uses FUSE. Without FUSE, run
  `APPIMAGE_EXTRACT_AND_RUN=1 ./Ira-0.1.0-x86_64.AppImage` (extraction costs startup time).
- The image carries `70-ira-input.rules` and `71-ira-uhid.rules`, but cannot
  install host udev rules. An administrator must install the rules from
  `contrib/` into the host's `/etc/udev/rules.d/` for input remapping when needed.
- Build-platform details are stored in `usr/share/ira/build-platform.txt`.
  Distribution license texts are included when `/usr/share/licenses` exists;
  review dependencies' source-distribution obligations before publishing.

## Native package staging

After building the release targets listed in `appimage.sh`:

```sh
DESTDIR=/path/to/staging PREFIX=/usr bash contrib/install.sh
```

The entry point is `/usr/bin/ira`; implementation binaries and overlay libraries
live in `/usr/lib/ira/`. Desktop, icon, AppStream, license, udev, and locale files
use the standard prefix-relative directories. This is suitable input for a
distribution package; GTK and other shared dependencies remain package-manager
dependencies. Custom prefixes can set `IRA_LOCALEDIR` when launching translations.

## Desktop search, shortcuts, and tray

Open **Settings → General → Desktop integration** to enable or disable desktop
search using the Ira launcher already installed by your AppImage manager or
native package. Ira never creates, changes, or removes that launcher. The switch
applies independently of the settings dialog's Save button. Disabling removes
Ira's search registration and session setup; optional game shortcuts remain.

Command-line equivalents:

```sh
./Ira.AppImage --install-desktop-integration
./Ira.AppImage --uninstall-desktop-integration
```

GNOME Shell searches session `XDG_DATA_DIRS` rather than `XDG_DATA_HOME` for
providers. User registration installs a user-service override at
`$XDG_CONFIG_HOME/systemd/user/org.gnome.Shell@.service.d/70-ira-desktop.conf`.
It prepends the user's data directory to the host's existing roots for GNOME
Shell and propagates that environment to D-Bus-activated apps before Shell
starts. A service-level environment setting takes precedence over GNOME's login
environment import, which can overwrite environment.d settings.

When the user's Flatpak export root is already in the host data paths, Ira also
registers its provider at
`$XDG_DATA_HOME/flatpak/exports/share/gnome-shell/search-providers/com.github.ira.ini`.
Enabling refreshes GNOME's provider settings so the running Shell rescans that
existing root. Only Ira's ownership-marked provider file is written there;
Flatpak applications and launchers are untouched. Disabling removes that file.

This setup is **user-local**: no administrator privileges, system directory
writes, or application-launcher changes. The service override remains a fallback
for sessions without that export root; inheriting it requires a new session.
Settings reports when the running session still cannot discover either provider
location. Native packages carry the provider
in their system data directory and keep it opt-in. Non-systemd GNOME session
startup is outside this user-session setup's support.

Discovery status reads the running GNOME Shell process's data roots rather than
Ira's environment. A successful D-Bus search request proves the provider answers;
it does not prove that Shell can discover the registration.
GLib caches that root list and does not support changing it at runtime. Reloading
provider files cannot add a directory absent from the running Shell's list.

Registration uses the persistent AppImage filename, never its temporary mount.
If you move the AppImage, disable and re-enable integration at its new location.
No application or per-game shortcuts are created as part of enabling desktop
search. Register an AppImage with your app manager first; search registration
uses its desktop ID, including manager-provided filenames and `env` wrappers.
Ira windows use that launcher's filename stem as their Wayland app ID and X11
program class, while keeping the stable GApplication bus name for forwarding.
Game shortcuts are launch actions: they disable startup notification and carry
no `StartupWMClass`, so they cannot claim Ira's windows.

Search is local and headless: no GUI startup, network access, source scan, or
keyring lookup occurs until selecting a result. It searches the saved installed
library, excludes hidden/vanished entries and missing known ROM/install paths,
and launches with Ira's normal configuration. PC install membership is the
cached library maintained by Ira's source scans. From a game's context menu,
**Create app menu shortcut** or **Remove app menu shortcut** manages its optional
desktop entry, including a separately listed variant. The context menu shows
only the applicable action, based on the existing Ira-owned shortcut file.
Shortcut icons are full-resolution PNG snapshots (bounded to 256 pixels) with
content-based filenames. Artwork changes refresh existing shortcuts with a new
icon identity, avoiding desktop texture caches keyed by the original thumbnail
path. Refreshing artwork preserves each shortcut's launch command and never
creates a shortcut the user has removed.

Ira also accepts `--launch-game ID`, optional `--variant ID`, and `--search TEXT`.
Requests reuse the running instance and wait for its initial library load.

The StatusNotifier tray provides **Hide to background** while visible or
**Show Ira** while hidden, plus **Quit**. GNOME needs an AppIndicator/StatusNotifier extension to display tray
items; compatible KDE and other desktops can display them directly. Enabling
**Close to background** hides directly without a confirmation dialog. Reopening
Ira or choosing **Show Ira** restores its content and saved window state.

The tray is menu-only: `ItemIsMenu=true` and its D-Bus interface deliberately
omits `Activate`. Advertising activation makes GNOME distinguish a single menu
click from a double-click window activation, delaying the menu. Ira exports the
small notifier and menu interfaces through GIO on a dedicated GLib thread, so
menu requests are independent of GTK rendering and library cleanup. Tests check
the advertised interface, late watcher registration, layout updates, and actions.

AppImage packaging also starts the **finished AppImage** on a private D-Bus bus
and Xvfb display. It rejects artifacts that advertise `Activate`, report
`ItemIsMenu=false`, or expose the wrong menu actions. Run the same check manually
with `python3 contrib/check-desktop-protocol.py /path/to/Ira.AppImage`.
This also checks window association against a manager-named launcher. In a
Wayland session, build the `desktop_identity_probe` example and run
`python3 contrib/check-wayland-identity.py` to check native identity across
hide/show cycles without submitting visible buffers.

## Checks

```sh
python3 contrib/test_packaging.py
bash -n contrib/AppRun contrib/install.sh contrib/appimage.sh
./target/appimage/Ira-0.1.0-x86_64.AppImage --help
```

The AppImage has been smoke-tested with a private D-Bus session, Xvfb, Cairo
rendering, and isolated XDG directories. Game launches, controller hardware,
Wayland/GPU rendering, and cross-distribution compatibility still need testing
on their target systems.

Desktop protocol integration tests use a private D-Bus daemon to test actual
service activation without a display, search results and metadata, and dispatch
to Ira. Session setup tests validate the GNOME user-service override with
systemd-analyze and check its preserved host data roots. A live user-manager
probe verifies that a service-level setting wins over imported login paths.

## Performance findings

The regression test in `crates/images/src/async_load.rs` exercises 40 decoded
2048x1024 hero images without widget references:

| Experiment | Result | Decision |
| --- | --- | --- |
| Cache pixbuf wrappers only for actual pixbuf consumers | Retained pixels: 128 MiB -> 64 MiB; the hero texture budget now governs texture-only loads | Kept, regression-tested |
| Bound completed decodes waiting for GTK | Queue capacity is four results, plus at most four workers' in-flight buffers | Kept to prevent an unbounded decoded-image backlog |
| Consume decoded RGBA buffers instead of copying during conversion | Repeated 20-image WebP runs: copying 419-439 ms, consuming 438-483 ms | Reverted; no demonstrated speed gain |

These are image-pipeline results, **not whole-application RAM claims**. Initial
window observations under Xvfb varied from 323 to 2653 ms with portal/font/cache
warmup and an isolated copy of the library missing its asset caches. That is
not a reliable startup-speed comparison, so no end-to-end startup improvement
is claimed. Measure a fixed, fully cached library and consistent desktop session
before making further startup changes.
