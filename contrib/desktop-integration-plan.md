# Desktop integration proposal

## Objective and acceptance criteria

Ira is a game library and launcher. Integrate the installed library into the
desktop without requiring one application-menu entry per game.

1. **Desktop search:** implement GNOME Shell's `SearchProvider2` protocol.
   Search saved, installed, non-hidden games locally without opening the Ira
   window, scanning emulator libraries, contacting services, or reading API keys.
   Selecting a result launches that game through Ira's existing launch flow,
   including its configured runner, controller settings, default variant, and
   disc picker. Selecting the provider opens Ira with the search text.
   Reuse the running Ira instance; handle requests arriving before library load.
   Do not claim native search support for desktops using a different protocol.
2. **Optional game shortcuts:** add a per-game action to create/remove an
   application-menu shortcut in the user's XDG applications directory. Search
   integration works independently of these shortcuts. Use game artwork where
   available and Ira's icon as a fallback. Never overwrite unrelated entries.
3. **Window state:** restore maximized state and the last normal window size,
   including after hide/restore. Fullscreen big-picture windows must not
   overwrite desktop-window state.
4. **Tray:** provide a menu-only StatusNotifierItem with Show Ira or
   Hide to background according to visibility, plus Quit. Preserve the
   existing close-to-background preference; closing hides directly without a
   confirmation dialog. Missing
   tray-host support must not crash Ira or prevent reopening it. GNOME Shell
   requires an AppIndicator/StatusNotifier extension to display tray icons.
5. **Settings:** enable and disable desktop integration from General settings.
   Disabling removes Ira-owned host registrations, preserves optional per-game
   shortcuts, and disables the GNOME search provider for the current user.

## Packaging and host registration

Include the search-provider definition and D-Bus activation metadata in native
package staging. Offer explicit per-user desktop integration registration for
AppImages: host registrations must point to the persistent AppImage path,
never its temporary mount directory. Keep integration registration separate
from optional per-game shortcuts. Document enabling the provider in GNOME's
Search settings and the tray-host limitation.

GNOME's provider discovery scans session `XDG_DATA_DIRS` with user-data scanning
disabled. User registration adds the data home through an Ira-owned user-service
drop-in for `org.gnome.Shell@.service`, which overrides GNOME's imported login
environment and synchronizes D-Bus activation before Shell starts. It reloads
user unit definitions, reports when the current session cannot discover the
provider, and removes the drop-in on disable. No system-wide or Flatpak export
files are changed. Provider interfaces must be exported in GApplication's `dbus_register`
before acquiring the bus name, so the very first D-Bus-activated search succeeds.

## Implementation plan

1. Persist desktop-window state with serialization/default tests.
2. Add a validated game-launch command and search command, forwarded through
   GApplication to the existing instance. Test cold-start request queuing.
3. Implement the local search index and headless D-Bus provider. Test matching,
   filtering, metadata, malformed result IDs, and calls in a private D-Bus session.
4. Implement opt-in menu shortcuts and user integration registration. Test
   desktop-entry escaping, executable paths containing spaces, AppImage paths,
   and safe removal of Ira-owned files.
5. Export a cross-desktop tray through GIO on its own GLib thread, routing
   its callbacks to GTK's main thread. Test registration, menu actions, shutdown,
   and operation without a tray watcher in a private D-Bus session.
   Omit `Activate` from introspection so GNOME treats a primary click as a menu
   click immediately rather than waiting to disambiguate double-click activation.
6. Update package contents, translations, and documentation; rebuild and
   smoke-test the AppImage. Commit independent, verified slices.

## Commands

All Cargo commands run inside `rust-dev`:

```sh
distrobox enter rust-dev -- cargo build
distrobox enter rust-dev -- cargo test
distrobox enter rust-dev -- cargo clippy --all-targets -- -D warnings
python3 contrib/test_packaging.py
bash po/update-pot.sh
distrobox enter rust-dev -- env LINUXDEPLOY=/path/to/linuxdeploy.AppImage bash contrib/appimage.sh
```

Run the full suite with isolated XDG config/data/cache directories: the
existing Vita3K missing-library test otherwise discovers the user's real games.

## Structure and style

- `crates/ira/src/desktop/`: desktop protocols, registration, shortcut generation,
  and incoming launch requests, split by responsibility; module roots only
  declare and re-export modules.
- `crates/ira/src/ui/`: window-state and context-menu wiring; keep protocol and
  filesystem work outside widget construction.
- `crates/config/src/`: serde-defaulted window state and preferences-only reads.
- `crates/db/src/`: read-only search queries if needed; no schema migration.
- `contrib/`: desktop/provider/service metadata, installation, and packaging tests.
- `po/ira.pot`: regenerate for new translated UI and tray strings.

Use existing `Result<T, String>`, `pub(crate)`, and dependency-direction
conventions. For example, desktop actions delegate to the launch flow rather
than duplicating runner configuration:

```rust
super::play_button::launch_game(state, db_id, variant_id)
```

## Verification and boundaries

- Test pure parsing, search, state serialization, and desktop-entry generation
  without GTK; integration tests use temporary directories and a private D-Bus.
- Verify window/tray lifecycle with Xvfb. Document any desktop-shell or hardware
  behavior that cannot be verified headlessly.
- Always validate incoming IDs, escape desktop-entry values, do I/O off the GTK
  thread, preserve unrelated user work, and review staged diffs before commits.
- Ask before adding the proposed `ksni` dependency or changing the behavior above.
- Never create per-game shortcuts automatically, install system-wide files
  without explicit permission, overwrite unrelated desktop entries, add database
  migrations, or push commits without a request.

## References

- [GNOME search-provider contract](https://developer.gnome.org/documentation/tutorials/search-provider.html)
- [ksni StatusNotifierItem implementation](https://docs.rs/ksni/0.3.7/ksni/)

## Status

Approved by the user, including the `ksni` dependency. Window state uses a
separate cache file so saving it cannot overwrite concurrently edited preferences.
