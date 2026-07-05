# AGENTS.md

## Project overview

Rust GTK4/libadwaita achievement viewer for emulator games (Goldberg Steam emulator and Nemirtingas GOG Galaxy emulator). Tracks, displays, and live-watches achievement unlocks across Steam and GOG games.

## Tech stack

- **Language**: Rust (nightly, edition 2021)
- **UI**: gtk4 0.11 (v4_14 feature), libadwaita 0.9 (v1_6 feature, aliased as `adw`), glib 0.22, gio 0.22, gdk4 0.11, pango 0.22
- **HTTP**: reqwest 0.12 (blocking client)
- **File watching**: notify 9.0.0-rc.4
- **Database**: rusqlite 0.32 (bundled SQLite)
- **Hashing**: xxhash-rust 0.8 (xxh3, for GOG DLL checksum to detect old vs new emulator config format)
- **Image**: image 0.25 (png + ico features, for ICO→PNG conversion and grid texture pre-scaling)
- **Serialization**: serde 1, serde_json 1
- **Time**: chrono 0.4 (std only)

## Build & run

```sh
cargo build --release          # release build
cargo build                    # debug build (use this for debugging!)
DISPLAY=:0 ./target/release/achievement-viewer
DISPLAY=:0 AV_BENCH=1 ./target/release/achievement-viewer   # bench mode: auto-switches 10×, logs RSS
```

**Always use `cargo build` (debug) when debugging. Only build release when verifying release behavior.**

## Architecture

### Module map

| File | Responsibility |
|------|---------------|
| `main.rs` | Entry point, `AppMessage` enum, app activation, DB init, DB population from existing dirs, enrichment loop startup |
| `db.rs` | SQLite database (`gse.db` at GSE root). `games` table: `kind` (steam/gog), `steam_id`, `platform_id`, `title`, `lutris_id`. Functions: `init_db`, `add_game`, `update_game`, `load_all_games`, `remove_game`, `find_by_steam_id` |
| `parser.rs` | `Game` and `MergedAchievement` structs, `load_games` (from DB), `load_game` (single game from `GameEntry`), `load_status_map` (Goldberg + GOG format), path helpers (`data_dir`, `achievements_dir`, `unlock_status_path`), `set_achievement_earned`, `convert_ico_to_png` (deletes .ico after conversion) |
| `steam.rs` | `SteamClient` (Mutex-protected API keys), `fetch_game_details`, `fetch_global_achievements` (handles `percent` as string OR number), `ensure_assets` (icon + hero), `ensure_grids` (vertical 600x900, header 460x215, logo), `generate_steam_settings` (writes achievement definitions to `data/<id>/achievements/`) |
| `gamesetup.rs` | `detect_app_id`, `is_gog_game`, `find_gog_info` (walks up dirs for `.info` files), GOG emulator config generation (`generate_galaxy_emu_config` with XXH3 checksum of Galaxy DLL), `add_game_from_folder` (Steam), `add_gog_game_from_folder` (GOG) |
| `watcher.rs` | `AchievementWatcher` (notify-based file watcher), watches `achievements.json` for unlock changes, sends `WatcherGameUpdated` messages, desktop notifications via `notify-send` |
| `images.rs` | Thread-local texture cache (`TEXTURE_CACHE` for full-size, `SCALED_CACHE` for pre-scaled textures). `scaled_texture` resizes images to exact target dimensions using `Triangle` filter so `Picture`'s natural size matches the card size (fixes FlowBox layout). `clear_texture_cache` clears both caches. |
| `config.rs` | `Config` struct: `steam_api_key`, `steam_griddb_api_key` (stored in secret-tool), `notifications_enabled`, `close_to_background`, `grid_scale_step` (0-4). Saved to `~/.config/achievement-viewer/config.json`. |
| `strings.rs` | Centralized UI string constants (sentence case). Import as `use crate::strings as S;` then use `S::SETTINGS`, `S::CANCEL`, etc. |
| `ui.rs` | All GTK UI: `AppState`, `build_ui`, `build_window`, `rebuild_sidebar` (with "All games" row at index 0), `show_grid_view`, `build_grid_card`, `display_game`, `create_achievement_row`, `create_global_stats_row` (hidden achievements use gray icon for both spoiler and revealed views, full-row slide animation), `hide_to_background` (destroys window + clears caches + malloc_trim), `restore_content`, game settings dialog (right-click sidebar row), hamburger menu (settings + zoom slider), `enrich_game_async` |
| `bench.rs` | `run_bench` — auto-switch 10× between first two games, hide-to-background, malloc_trim, RSS logging. Triggered by `AV_BENCH=1` env var. |

### Data flow

1. On startup: DB initialized → populated from existing `steam/` and `gog/` directories if empty → `load_games` reads all games from DB → `build_ui` creates the main window with grid view
2. Each game loaded → watcher starts watching its `achievements.json` → `enrich_game_async` spawns a thread to fetch Steam details, download assets (icon, hero, grids), and global achievement percentages
3. Enrichment sends `EnrichedGame` message → `apply_game_update` merges data and refreshes display if the game is currently viewed
4. File watcher detects `achievements.json` change → reloads game → sends `WatcherGameUpdated` → shows desktop notification for newly unlocked achievements

### Directory structure

```
/data/Games/Saves/GSE/
├── gse.db                          # SQLite database
├── data/
│   └── <steam_id>/
│       ├── achievements/           # achievement definitions (symlink to game's steam_settings for Steam games, real folder for GOG)
│       │   ├── achievements.json
│       │   └── achievement_images/
│       ├── appdetails.json         # cached Steam store data
│       ├── global_achievements.json # cached global percentages
│       ├── icon.png
│       ├── library_hero.jpg
│       ├── library_600x900.jpg     # vertical grid (for grid view)
│       ├── header.jpg              # horizontal grid
│       └── logo.png                # transparent logo for hero overlay
├── steam/
│   └── <appid>/
│       └── achievements.json       # unlock status (symlinked to Wine prefixes)
├── gog/
│   └── 100000000000000000/         # fixed galaxyid (we generate the config ourselves)
│       └── <product_id>/
│           └── achievements.json   # GOG unlock status
```

### Key design decisions

- **No `game_dir` field on `Game`**: Games are identified by `(kind, steam_id, platform_id)` from the DB. Paths are computed via helper functions.
- **Pre-scaled textures**: `scaled_texture()` in `images.rs` resizes images to exact card dimensions so `Picture`'s natural size is correct, fixing FlowBox layout. Uses `Triangle` filter (fast). Cached per `(path, width)` key.
- **GOG config generation**: XXH3 checksum of `Galaxy.dll`/`Galaxy64.dll` determines old flat format vs new nested `GalaxyEmu` format. We write the config ourselves with a fixed `galaxyid: 100000000000000000`.
- **Grid view**: Default state when no game is selected. "All games" is the first sidebar row (index 0). FlowBox with `set_homogeneous(true)` for space-between distribution.
- **Hidden achievements**: Gray icon (`icon_gray_path`) shown for both spoiler and revealed views. Full-row slide animation using `gtk4::Stack` with `SlideLeft` transition. Only colored icon shown when actually earned.
- **ICO cleanup**: `convert_ico_to_png` deletes the original `.ico` after conversion.
- **Strings**: All UI text centralized in `strings.rs` with sentence case ("Add game", not "Add Game").
- **Config**: `grid_scale_step` (0-4) saved in config.json. `GRID_SIZES = [80, 140, 200, 260, 320]`, default is step 2 (200px).
- **`RefCell` borrow patterns**: Never hold a `state.borrow()` across a `state.borrow_mut()`. Extract owned values first, drop the `Ref`, then borrow mutably.

### GOG emulator config formats

**Old flat format** (when DLL XXH3 matches known hashes):
```json
{"galaxyid": 100000000000000000, "productid": N, "api_version": "1.152.1.0", ...}
```

**New nested format** (all other DLLs):
```json
{"GalaxyEmu": {"Application": {"AppId": N}, "User": {"GalaxyId": 100000000000000000}}}
```

### GOG achievements.json format

```json
{"ACHIEVEMENT_NAME": {"unlock_time": 1783168359}}
```
Only earned achievements listed. `null` or `{}` when nothing earned. `load_status_map` handles Goldberg, GOG, and null formats.

## Coding conventions

- **No comments** unless explicitly requested
- **No emojis** unless explicitly requested
- **Sentence case** for all UI strings (via `strings.rs`)
- **Zero warnings** policy: fix all unused imports/variables/consts
- **Debug builds** for debugging, release builds for final verification
- Use `cargo build` not `cargo build --release` when iterating

## Maintenance

After making changes to the codebase, update this file to reflect:
- New modules or changed module responsibilities
- Changed directory structure or data flow
- New design decisions or changed conventions
- New config fields or DB schema changes

Do NOT add bug reports, future plans, or TODO items here — those go in `TODO.md`.
