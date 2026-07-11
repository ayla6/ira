# Feature Plan: Game Launching, Wine Config, Play Sessions

## Overview
Four major features, analyzed from the Lutris source code at ~/Downloads/lutris:
1. Add UI to add new games (native + Wine) with full config
2. Add Wine config UI (prefix, version, env, DLL overrides, esync/fsync, DXVK, etc.)
3. Add game launcher wrapper (subreaper pattern, env construction, process tracking, playtime sessions)
4. Add play session tracking (per-session DB table for "what you played in a day" metadata)

## Architecture (dependency order)
- Phase 1: Config models + DB schema (no dependencies) ✅
- Phase 2: Play session DB schema + CRUD (no dependencies) ✅
- Phase 3: Game launcher - subreaper + env construction + process tracking (depends on Phase 1) ✅
- Phase 4: Play session tracking integration (depends on Phase 2 + Phase 3) ✅
- Phase 5: Add Game UI - full config dialog (depends on Phase 1) ⚠️
- Phase 6: Wine config UI widget (depends on Phase 1, used by Phase 5) ✅
- Phase 0: Enable Add Game button (bug fix, no dependencies) ❌
- Phase 7: Complete Add Game dialog - env vars, browse, validation (depends on Phase 5) ❌
- Phase 8: Unified Edit Game dialog - replaces old settings dialog (depends on Phase 7) ❌
- Phase 9: Apply WineConfig fields at launch - wire ~13 ignored fields (depends on Phase 6) ❌
- Phase 10: Launcher process management fixes - process groups, unify monitor (depends on Phase 3) ❌
- Phase 11: Wire ld_preload / ld_library_path in build_env (depends on Phase 1) ❌
- Phase 12: Play session improvements - expandable history, refresh, crash handling (depends on Phase 4) ❌
- Phase 13: Code cleanup - dead code, formatters, GE-Proton check (no dependencies) ❌

---

## PHASE 1: Config Models and DB Schema ✅

### Create `src/models/launch_config.rs` ✅

**`GameLaunchConfig` struct** (per-game launch configuration, stored as JSON in DB): ✅
- `exe: String` -- path to game executable (e.g. /path/to/game.exe or /path/to/game.bin) ✅
- `args: String` -- command-line arguments (shlex-split at launch time) ✅
- `working_dir: String` -- working directory (empty = use exe's dir) ✅
- `env_vars: Vec<(String, String)>` -- extra environment variables ✅
- `ld_preload: String` -- for native Linux games (LD_PRELOAD) ✅
- `ld_library_path: String` -- for native Linux games ✅

**`WineConfig` struct** (per-game Wine runner configuration): ✅
- `enabled: bool` -- whether this game uses Wine (false = native Linux) ✅
- `prefix: String` -- Wine prefix path (empty = default ~/.wine) ✅
- `version: String` -- "system", "winehq-devel", "winehq-staging", "wine-development", "ge-proton", "custom", or Lutris-installed version like "wine-ge-8-26-x86_64" ✅
- `custom_wine_path: String` -- path to custom wine binary (used when version == "custom") ✅
- `arch: String` -- "auto" | "win32" | "win64" (default "auto") ✅
- `esync: bool` -- enable esync (default true) ✅
- `fsync: bool` -- enable fsync (default true) ✅
- `dxvk: bool` -- enable DXVK (default true) ✅
- `vkd3d: bool` -- enable VKD3D (default true) ✅
- `d3d_extras: bool` -- enable D3D Extras (default true) ✅
- `dxvk_nvapi: bool` -- enable DXVK-NVAPI/DLSS (default true) ✅
- `fsr: bool` -- enable AMD FSR (default true) ✅
- `battleye: bool` -- enable BattlEye anti-cheat (default true) ✅
- `eac: bool` -- enable Easy Anti-Cheat (default true) ✅
- `show_debug: String` -- WINEDEBUG value ("-all" | "" | "+fps" | "+all", default "-all") ✅
- `dll_overrides: Vec<(String, String)>` -- WINEDLLOVERRIDES entries ✅
- `audio: String` -- "auto" | "alsa" | "pulse" | "oss" (default "auto") ✅
- `graphics: String` -- "auto" | "wayland" | "x11" (default "auto") ✅
- `desktop_integration: bool` (default false) ✅
- `show_crash_dialogs: bool` (default false) ✅
- `mouse_warp_override: String` -- "enable" | "disable" | "force" (default "enable") ✅
- `virtual_desktop: bool` (default false) ✅
- `virtual_desktop_res: String` -- e.g. "1920x1080" ✅
- `dpi_enabled: bool` (default false) ✅
- `dpi: i32` (default 96) ✅
- `gamemode: bool` (default false) ✅
- `mangohud: bool` (default false) ✅
- `gamescope: bool` (default false) ✅
- `gamescope_flags: String` ✅

**`LaunchConfig` convenience struct** wrapping `GameLaunchConfig + Option<WineConfig>`. ✅

- Implement `Default` for both structs (matching defaults above). ✅
- Add `serde::{Serialize, Deserialize}` to all. ✅
- Re-export from `src/models/mod.rs` and `src/lib.rs`. ✅
- Add unit tests for `Default` impls. ✅

### Create DB table `game_configs` ✅
- Table created in init_db ✅ (without FOREIGN KEY — intentional per AGENTS.md)

### Create `src/db/game_config.rs` ✅
- `get_game_config` ✅
- `save_game_config` (UPSERT) ✅
- `delete_game_config` ✅
- Re-export from `src/db/mod.rs` ✅
- Tests with tempfile DB ✅

---

## PHASE 2: Play Session DB Schema and CRUD ✅

### Create DB table `play_sessions` ✅
- Table + indexes created in init_db ✅ (without FOREIGN KEY)

### Create `src/models/session.rs` ✅
- `PlaySession` struct ✅
- `Default` impl ✅
- Re-exported ✅

### Create `src/db/sessions.rs` ✅
- `record_session` ✅
- `get_sessions_for_game` ✅
- `get_sessions_for_date` ✅
- `get_sessions_range` ✅
- `get_total_playtime_for_game` ✅
- `get_playtime_by_day` ✅
- `delete_sessions_for_game` ✅
- Re-exported ✅
- Tests ✅

---

## PHASE 3: Game Launcher -- Subreaper Pattern and Process Tracking ✅

### Create `src/launcher/` module ✅

### `src/launcher/mod.rs` ✅
- `launch_game()` entry point ✅ (simplified: returns PID, not LaunchedGame)
- Decides Wine vs native, builds env, calls wrapper ✅

> **Deviation from plan:** The `LaunchedGame` struct with `kill()`, `is_running()`, `Drop` impl was
> not implemented. Instead, a simpler PID-based approach is used: `running_games` is
> `HashMap<i64, i32>` (lutris_id → pid), and `stop_game()` sends SIGTERM via `libc::kill()`.

### `src/launcher/wrapper.rs` ✅
- `spawn_game()` — sets PR_SET_CHILD_SUBREAPER, spawns child ✅
- `monitor_process()` — polls try_wait every 2s, reaps zombies, records session, sends messages ✅
- `reap_zombies()` / `reap_zombies_once()` ✅
- Crash detection (duration < 5s → warning) ✅

> **Not implemented (see Phase 10):**
> - Signal handling (SIGTERM/SIGINT forwarding to child processes) — Phase 10
> - Return code file (AV_RETURN_CODE_FILE) — deferred (no consumer)
> - `run_as_subreaper()` as a separate function (merged into `spawn_game` + `monitor_process`) — by design
> - Process group / `setsid()` — `stop_game()` only kills top-level PID, not descendants — Phase 10
> - `reap_zombies()` uses `waitpid(-1)` which reaps ANY child — Phase 10
> - Duplicated monitor logic (`wrapper::monitor_process` vs `helpers::monitor_running_game`) — Phase 10

### `src/launcher/env_builder.rs` ✅
- `build_env()` — merges user env, Wine env, shader cache ✅
- NVIDIA shader cache ✅
- Gamemode wrapper ✅
- MangoHud wrapper ✅
- Gamescope wrapper ✅

> **Not implemented (see Phase 11):**
> - `AV_GAME_UUID` env var — deferred (no consumer)
> - `AV_RETURN_CODE_FILE` env var — deferred (no consumer)
> - `ld_preload` / `ld_library_path` fields exist in `GameLaunchConfig` but `build_env()` never reads them — Phase 11

### `src/launcher/wine_launch.rs` ✅
- `build_wine_env()` ✅ (WINEDEBUG, WINEARCH, WINEPREFIX, WINEESYNC, WINEFSYNC, etc.)
- `find_wine_binary()` ✅ (system, custom, winehq-devel/staging, wine-development, Lutris runners)
- `detect_wine_versions()` ✅
- `build_wine_command()` ✅
- `detect_arch()` ✅
- `format_dll_overrides()` ✅ (with unit tests)
- `is_esync_limit_set()` ✅ implemented (line 209) but never called — dead code
- `is_fsync_supported()` ✅ implemented (line 218) but never called — dead code

### `src/launcher/native_launch.rs` ✅
- `build_native_command()` ✅ (with tests)
- `validate_executable()` ✅ (with tests)

### Wire into UI ✅
- `play_button.rs` uses `launcher::launch_game()` for games with stored config ✅
- Lutris fallback (`lutris:rungameid/<id>`) for games without config ✅
- PS4 games keep shadps4 launch ✅
- `stop_game()` sends SIGTERM ✅
- Context menu Play uses shared `launch_game()`/`stop_game()` ✅
- `GameStarted` message refreshes display after context menu launch ✅

---

## PHASE 4: Play Session Tracking Integration ✅

### Session recording ✅
- `wrapper.rs:monitor_process()` records session on exit ✅
- `helpers.rs:monitor_running_game()` records session on exit ✅ (PS4/Lutris path)
- `SessionRecorded` message sent BEFORE `GameStopped` (correct order for display refresh) ✅
- `set_last_played` called once on launch (in `launch_game()`) for all paths ✅
- Crash detection (duration < 5s → warning) in both paths ✅

### `src/ui/play_history.rs` ✅
- `show_play_history_dialog()` — per-game session list with total playtime ✅
- `show_daily_history_dialog()` — last 30 days playtime by day ✅

> **Not implemented (see Phase 12):**
> - Clicking a date in daily history expands to show individual sessions — Phase 12
> - `SessionRecorded` handler doesn't refresh displayed game — Phase 12
> - Crash sessions (< 5s) are still recorded, polluting history — Phase 12

### UI integration ✅
- Context menu: "View Play History" item ✅
- Window menu: "Play History" item ✅
- `message_handler.rs`: handles `SessionRecorded` (updates in-memory playtime) ✅
- `message_handler.rs`: handles `GameStarted` (refreshes display) ✅
- `game_display.rs`: already shows playtime (uses `game.playtime` updated by `SessionRecorded`) ✅

---

## PHASE 5: Add Game UI -- Full Config Dialog ⚠️ PARTIALLY DONE

### Create `src/ui/add_game_dialog.rs` ⚠️

`pub fn show_add_game_dialog(state: &SharedState)` ✅

**Game Info section** ✅
- Name (EntryRow) ✅
- Game kind selector (ComboRow): Native Linux / Wine (Windows) ✅
- Executable path (EntryRow) ✅
- Arguments (EntryRow) ✅
- Working directory (EntryRow) ✅

> **Not implemented:**
> - Browse buttons for executable path and working directory
> - Environment variables (dynamic key-value list)

**Wine Configuration section** ✅
- Embedded `wine_config_widget` shown when kind == Wine ✅

**Achievement Source section** ✅
- Steam App ID (EntryRow) ✅
- GOG Product ID (EntryRow) ✅
- Auto-detect button: scans folder for steam_appid.txt / GOG galaxy files ✅

**On "Add Game":** ✅
- Validate: name non-empty, exe non-empty ✅
- Determine kind/trophy_source/platform_id ✅
- `db::add_game()` + `db::save_game_config()` ✅
- Send `AppMessage::NewGame(game)` ✅
- Start enrichment ✅

**Auto-detection shortcut** ✅
- Browse for folder → auto-detect Steam/GOG, pre-fill fields ✅
- Auto-detect executable in folder (.exe, .x86_64, AppRun, .sh) ✅

> **NOT implemented (see Phase 0, 7, 8):**
> - Add Game button is permanently disabled (`window.rs:118` `set_sensitive(false)`, never enabled) — Phase 0
> - `show_edit_game_dialog(state, db_id)` — edit existing game configs (pre-filled dialog) — Phase 8
> - Context menu "Edit Game Settings" still opens old `dialogs::show_game_settings_dialog`, not the new dialog — Phase 8
> - Environment variables UI (dynamic key-value list, same as DLL overrides pattern) — Phase 7
> - Browse buttons for file picker on exe path and working dir — Phase 7
> - Browse buttons for prefix path and custom wine path in wine_config_widget — Phase 7
> - Validation: exe exists on filesystem (only checks non-empty) — Phase 7
> - Page-based navigation (AdwNavigationView) — simplified to single scrolling page (deferred)

---

## PHASE 6: Wine Config UI Widget ✅

### Create `src/ui/wine_config_widget.rs` ✅

`pub fn build_wine_config_page(wine: &WineConfig) -> (gtk4::Box, WineConfigWidgets)` ✅

**Section: "Wine Version"** ✅
- Wine version ComboRow with `detect_wine_versions()` ✅
- Custom Wine path EntryRow (visible only when version == "custom") ✅
- Prefix architecture ComboRow (Auto/32-bit/64-bit) ✅

**Section: "Wine Prefix"** ✅
- Prefix path EntryRow ✅
- Desktop integration SwitchRow ✅

**Section: "Performance"** ✅
- Esync, Fsync, FSR, Gamemode, MangoHud, Gamescope SwitchRows ✅
- Gamescope flags EntryRow ✅

**Section: "Graphics"** ✅
- DXVK, VKD3D, D3D Extras, DXVK-NVAPI SwitchRows ✅
- Graphics backend ComboRow ✅
- Virtual desktop SwitchRow + resolution EntryRow ✅
- DPI scaling SwitchRow + SpinButton ✅
- Mouse warp override ComboRow ✅
- Audio driver ComboRow ✅

**Section: "Anti-Cheat"** ✅
- BattlEye, Easy Anti-Cheat SwitchRows ✅

**Section: "Debugging"** ✅
- Output debugging info ComboRow ✅
- Show crash dialogs SwitchRow ✅

**Section: "DLL Overrides"** ✅
- Dynamic list of DLL override rows (name entry + type dropdown + remove button) ✅
- "Add override" button ✅
- `collect_dll_overrides()` reads all rows ✅

**`WineConfigWidgets` struct** ✅ (all widget fields)
**`to_wine_config()`** ✅

**Visibility logic** ✅ (partial)
- Proton: hides arch, VKD3D, virtual desktop ✅
- Custom: shows custom_wine_path ✅

> **Not implemented (see Phase 7, 9):**
> - `to_launch_config_extras()` — env vars not on this page — Phase 7
> - Browse buttons for prefix path and custom wine path — Phase 7
> - Visibility toggles for gamescope_flags, virtual_desktop_res, dpi (fields exist but always/never visible) — Phase 7
> - Proton-specific options (PROTONPATH, etc.) — Phase 9
> - Sensitive-only-if-found checks for gamemode/mangohud/gamescope binaries — Phase 9
> - Integration with `dialogs.rs` (game settings dialog) — Phase 8 (replaces old dialog)

---

## PHASE 0: Enable Add Game Button (bug fix) ❌

The Add Game button is created with `set_sensitive(false)` at `window.rs:118` and never
enabled anywhere. The click handler is wired (`window.rs:302-304`) but the button stays
disabled, making the entire add game feature inaccessible.

### Fix
Remove `add_btn.set_sensitive(false)` at `window.rs:118`. The dialog only needs `db`,
`sender`, `steam`, `watcher`, `save_dir` — all available from startup. No need to wait
for `GamesLoaded`.

---

## PHASE 7: Complete the Add Game Dialog ❌

Finish the remaining items from Phase 5. These are also prerequisites for Phase 8 since
the edit dialog will reuse the same section-builder patterns.

### 7a. Environment Variables UI ❌
- Create `build_env_var_row(key: &str, value: &str) -> gtk4::ListBoxRow` — two `gtk4::Entry`
  fields (key + value) + remove button. Copy the `build_dll_override_row` pattern from
  `wine_config_widget.rs:291-334`.
- Create `collect_env_vars(box: &gtk4::ListBox) -> Vec<(String, String)>` — copy
  `collect_dll_overrides` pattern from `wine_config_widget.rs:336-371`.
- Add "Add variable" button.
- Wire collected env_vars into `GameLaunchConfig.env_vars` at `add_game_dialog.rs:173-178`.

### 7b. Browse Buttons ❌
- File picker on `exe_entry` — `gtk4::FileDialog::open()` with executable filter.
  Pattern at `dialogs.rs:189-205`.
- Folder picker on `wd_entry` — `gtk4::FileDialog::select_folder()`.
  Pattern at `add_game_dialog.rs:104-138`.
- File picker on `custom_wine_path` in `wine_config_widget.rs`.
- Folder picker on `prefix` in `wine_config_widget.rs`.

### 7c. Validation Feedback ❌
- Replace silent `return` at `add_game_dialog.rs:156-162` with visible error
  (`add_css_class("error")` on the EntryRow, or inline `gtk4::Label`).
- Optionally check `Path::new(&exe).is_file()`.

### 7d. Extract Shared Section Builders ❌
- Pull game info, env vars, achievement source sections into reusable functions that both
  add and edit dialogs can call. Avoids duplication (AGENTS.md: extract on 3rd occurrence).
- `build_game_info_section(config: Option<&GameLaunchConfig>) -> (PreferencesGroup, widgets...)`
- `build_env_vars_section(vars: &[(String, String)]) -> (PreferencesGroup, ListBox)`
- `build_achievement_source_section(steam_id: &str, gog_id: &str) -> (PreferencesGroup, ...)`

### 7e. Visibility Toggles in Wine Config Widget ❌
- Show `gamescope_flags` EntryRow only when `gamescope` switch is active.
- Show `virtual_desktop_res` EntryRow only when `virtual_desktop` switch is active.
- Show `dpi` SpinButton only when `dpi_enabled` switch is active.

---

## PHASE 8: Unified Edit Game Dialog ❌

Replace `dialogs::show_game_settings_dialog` with a new `show_edit_game_dialog(state, db_id)`
that combines all settings in a sidebar+stack layout (like the old dialog at `dialogs.rs:655`).

### 8a. Dialog Structure ❌
Use `adw::Window` with horizontal split: sidebar `ListBox` (`navigation-sidebar` CSS) +
`gtk4::Stack` (crossfade). Matches the existing pattern at `dialogs.rs:667-697`.

### 8b. Pages (conditionally shown) ❌

| Page | Condition |
|------|-----------|
| **General** | Always — title, sort title, shadPS4 version (PS4 only), unmatch button (Lutris-matched only) |
| **Launch Config** | Only if game has config in DB (`db::get_game_config` returns `Some`) |
| **Wine Config** | Only if config exists AND `wine.enabled` |
| **Achievement Source** | Always (or if `!app_id.is_empty()`) |
| **Images** | If `!app_id.is_empty()` |
| **Logo** | If `!logo_path.is_empty()` |
| **DLC** | If game has DLCs |

Decision: Lutris-managed games do NOT get a Launch Config page. Only manually-added games
(kind=wine/linux) that already have a config in the DB get it.

### 8c. Reuse Existing Page Builders ❌
- General: `build_game_general_page` from `dialogs.rs:364-509`
- Wine Config: `build_wine_config_page` from `wine_config_widget.rs:65`
- Images: `build_image_manager_content_with_drafts` from `dialogs.rs:948-1021`
- Logo: `build_game_logo_page` from `dialogs.rs:511-653`
- DLC: inline builder from `dialogs.rs:720-750`
- Launch Config + Achievement Source: new shared builders from Phase 7d

### 8d. Save Handler ❌
- `db::save_game_config(db, db_id, &launch, &wine)` for launch/wine config
- `db::update_game_title` / `update_sort_title` / `set_shadps4_version` for identity
- Pending image copies (same as `dialogs.rs:816-924`)
- Update `state.games` in-memory, rebuild sidebar, re-display if selected
- Close window

### 8e. Wire Context Menu ❌
Change `context_menu.rs:82` from `show_game_settings_dialog(&sc, &game)` to
`show_edit_game_dialog(&sc, game.db_id)`. Look up game by `db_id` (not `lutris_id`),
since manually-added games have `lutris_id: 0`.

### 8f. Remove Old Dialog ❌
Delete `show_game_settings_dialog` and its helpers from `dialogs.rs` after confirming
no other callers exist (grep shows only `context_menu.rs:82` calls it).

---

## PHASE 9: Apply WineConfig Fields at Launch ❌

Wire the ~13 ignored `WineConfig` fields into `wine_launch.rs` and `env_builder.rs`.
Currently these fields are exposed in the UI but silently ignored at launch time.

### 9a. Env Var Fields (in `build_wine_env`) ❌
- `vkd3d` → `PROTON_ENABLE_VKD3D=1` / `PROTON_DISABLE_VKD3D=1`
- `d3d_extras` → DLL overrides for d3dcompiler_*
- `battleye` → `PROTON_BATTLEYE_LAUNCHER=1`
- `eac` → `PROTON_EAC_LAUNCHER=1`
- `audio` → `AUDIODEV` or PulseAudio env vars
- `graphics` → Wayland/X11 selection

### 9b. Wine Registry Key Fields (pre-launch `wine reg add` commands) ❌
- `mouse_warp_override` → `HKCU\Software\Wine\X11 Driver\MouseWarpOverride`
- `virtual_desktop` + `virtual_desktop_res` → `HKCU\Software\Wine\Explorer\Desktops`
- `dpi_enabled` + `dpi` → `HKCU\Software\Wine\Fonts\LogPixels`
- `desktop_integration` → `HKCU\Software\Wine\... Links`
- `show_crash_dialogs` → `HKCU\Software\Wine\WineDbg\ShowCrashDialog`

Create `build_wine_reg_commands(wine: &WineConfig, wine_exe: &str) -> Vec<Vec<String>>`
and run before game launch in `launcher/mod.rs`.

### 9c. Command Modification Fields ❌
- `virtual_desktop` → wrap exe in `explorer.exe /desktop=Name,res`
- `arch` → respect user's arch choice instead of always auto-detecting
  (currently `detect_arch` overrides at `wine_launch.rs:15`)

### 9d. Sensitive-Only-If-Found Checks ❌
- Disable `gamemode` switch if `gamemoderun` not found in PATH
- Disable `mangohud` switch if `mangohud` not found in PATH
- Disable `gamescope` switch if `gamescope` not found in PATH
- Use `has_exec()` helper from `env_builder.rs:14-20`

---

## PHASE 10: Launcher Process Management Fixes ❌

### 10a. Process Group + setsid ❌
- `wrapper.rs:spawn_game()` → call `setsid()` or `Command::process_group(0)` in child
- `play_button.rs:stop_game()` → `kill(-pid, SIGTERM)` to kill the process group
- Add SIGKILL fallback after ~5s timeout

### 10b. Fix reap_zombies Scope ❌
- `wrapper.rs:reap_zombies()` currently uses `waitpid(-1, ...)` which reaps ANY child
- Change to `waitpid(-pgid, ...)` scoped to the game's process group

### 10c. Unify Monitor Logic ❌
- Two near-identical implementations exist:
  - `wrapper.rs:monitor_process()` (lines 40-95) — used by `launcher::launch_game`
  - `helpers.rs:monitor_running_game()` (lines 140-183) — used by PS4/Lutris path
- Extract shared `wait_and_record(child, sender, lutris_id, db, game_id, started_at, running_games)`
- Both callers use the shared function; zombie reaping always on
- PS4/Lutris path currently misses the "possible crash" warning — unify fixes this

---

## PHASE 11: Wire ld_preload / ld_library_path ❌

In `env_builder.rs:build_env()`, add ~6 lines to push `LD_PRELOAD` and `LD_LIBRARY_PATH`
from `GameLaunchConfig` into the env vec. These fields exist in the model and will have
UI from Phase 7d/8d but are currently silently ignored at launch.

---

## PHASE 12: Play Session Improvements ❌

### 12a. Expandable Daily History ❌
- Replace `ListBoxRow` with `adw::ExpanderRow` in `play_history.rs:131-150`
- Call `get_sessions_for_date()` (already exists at `db/sessions.rs:45`, currently unused)
  for each day
- Show game name (resolve via `HashMap<i64, String>` built once), start time (`%H:%M`),
  duration in child rows
- Eager loading is fine for 30-day window

### 12b. SessionRecorded Refresh ❌
- In `message_handler.rs:70-76`, after updating `playtime`, check if the game is currently
  displayed and call `display_game` if so. Currently `playtime` updates in memory but the
  UI doesn't refresh until `GameStopped` fires.

### 12c. Crash Session Handling ❌
- Sessions < 5s are warned about but still recorded, polluting history
- Skip recording for sessions < 5s (or mark them with a flag)

---

## PHASE 13: Code Cleanup ❌

### 13a. Call is_esync_limit_set() / is_fsync_supported() ❌
- Functions exist at `wine_launch.rs:209-231` but are dead code
- Wire into `build_wine_env()` to `eprintln!` a warning when user enables esync/fsync
  on an unsupported kernel

### 13b. Fix detect_wine_versions() GE-Proton Check ❌
- `wine_launch.rs:168` unconditionally pushes `"GE-Proton (Latest)"` without checking
  if it's installed. Verify before listing.

### 13c. Remove Dead init_subreaper() ❌
- `env_builder.rs:10-12` — never called. `wrapper.rs` already calls `set_subreaper()`
  at spawn time. Remove the duplicate.

### 13d. Unify Duration Formatters ❌
- `game_display.rs:482` → `format_playtime(hours: f64) -> String` → `"5h30min"`
- `play_history.rs:6-14` → `format_duration(secs: i64) -> String` → `"5 h 30 min"`
- Extract to shared helper in `ui/helpers.rs`

### 13e. Remove Unused Strings ❌
- `strings.rs`: `PLAY_SESSION`, `SESSIONS`, `DURATION`, `DATE` are defined but never used
- Either use them in expanded daily history rows (Phase 12a) or remove them

---

## Deferred / Speculative

- `AV_GAME_UUID` env var — no consumer exists yet
- `AV_RETURN_CODE_FILE` env var — no consumer exists yet
- Return code file — current 2s-poll `try_wait` is sufficient for session tracking
- SIGINT forwarding — not applicable for GTK app (no terminal)
- Page-based navigation (AdwNavigationView) — single scrolling page works fine
- Proton-specific options (PROTONPATH) — partially handled already in `build_wine_env`
