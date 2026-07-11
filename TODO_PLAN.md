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

> **Not implemented:**
> - Signal handling (SIGTERM/SIGINT forwarding to child processes)
> - Return code file (AV_RETURN_CODE_FILE)
> - `run_as_subreaper()` as a separate function (merged into `spawn_game` + `monitor_process`)

### `src/launcher/env_builder.rs` ✅
- `build_env()` — merges user env, Wine env, shader cache ✅
- NVIDIA shader cache ✅
- Gamemode wrapper ✅
- MangoHud wrapper ✅
- Gamescope wrapper ✅

> **Not implemented:**
> - `AV_GAME_UUID` env var
> - `AV_RETURN_CODE_FILE` env var

### `src/launcher/wine_launch.rs` ✅
- `build_wine_env()` ✅ (WINEDEBUG, WINEARCH, WINEPREFIX, WINEESYNC, WINEFSYNC, etc.)
- `find_wine_binary()` ✅ (system, custom, winehq-devel/staging, wine-development, Lutris runners)
- `detect_wine_versions()` ✅
- `build_wine_command()` ✅
- `detect_arch()` ✅
- `format_dll_overrides()` ✅ (with unit tests)
- `is_esync_limit_set()` ❌ NOT implemented
- `is_fsync_supported()` ❌ NOT implemented

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

> **Not implemented:**
> - Clicking a date in daily history expands to show individual sessions

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

> **NOT implemented:**
> - `show_edit_game_dialog(state, db_id)` — edit existing game configs (pre-filled dialog)
> - Context menu "Edit Game Settings" still opens old `dialogs::show_game_settings_dialog`, not the new add_game_dialog
> - Environment variables UI (dynamic key-value list, same as DLL overrides pattern)
> - Browse buttons for file picker on exe path and working dir
> - Validation: exe exists on filesystem (only checks non-empty)
> - Page-based navigation (AdwNavigationView) — simplified to single scrolling page

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

> **Not implemented:**
> - `to_launch_config_extras()` — env vars not on this page
> - Browse buttons for prefix path and custom wine path
> - Visibility toggles for gamescope_flags, virtual_desktop_res, dpi (fields exist but always/never visible)
> - Proton-specific options (PROTONPATH, etc.)
> - Sensitive-only-if-found checks for gamemode/mangohud/gamescope binaries
> - Integration with `dialogs.rs` (game settings dialog)
