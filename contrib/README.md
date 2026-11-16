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
