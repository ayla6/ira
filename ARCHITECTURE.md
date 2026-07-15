# Architecture

## Workspace layout

The project is a Cargo workspace with 10 crates under `crates/`. Each crate
has its own `Cargo.toml` and `src/` directory.

```
crates/
├── models/              # ira-models (Level 0 — leaf, no ira deps)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Re-exports
│       ├── game.rs            # Game struct, sort_key, unmatched_game
│       ├── game_entry.rs      # GameEntry (DB row representation)
│       ├── achievement.rs     # MergedAchievement, AchievementStatus, StringOrMap
│       ├── message.rs         # AppMessage enum + AppSender
│       ├── launch_config.rs   # GameLaunchConfig, WineConfig, WineProfile
│       ├── variant.rs         # GameVariant
│       ├── session.rs         # PlaySession
│       ├── group.rs           # Group, GroupSelection
│       ├── sort_mode.rs       # SortMode enum
│       ├── kind.rs            # Kind constants (STEAM, PS4, GBE_STEAM, etc.)
│       ├── consoles.rs        # ConsoleDef, CONSOLES, find_console
│       └── app_details.rs     # AppDetails, DlcInfo
│
├── db/                  # ira-db (Level 1 — depends on ira-models)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # DbConn, init_db, schema migrations, game_entry_from_row
│       ├── crud.rs            # add_game, remove_game, load_all_games
│       ├── lookup.rs          # find_by_steam_id, find_by_db_id, etc.
│       ├── lutris_ops.rs      # upsert_matching, unmatch_game, set_lutris_db_id
│       ├── settings.rs        # Per-game settings updates
│       ├── game_config.rs     # Launch config + wine config persistence
│       ├── sessions.rs        # Play session recording
│       ├── profiles.rs        # Wine profiles
│       ├── variants.rs        # Game variants
│       ├── groups.rs          # Game groups/collections
│       ├── metadata.rs        # Release dates, scores, review data
│       └── migration.rs       # Schema migrations (ensure_column)
│
├── parser/              # ira-parser (Level 1 — depends on ira-models)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Re-exports
│       ├── paths.rs           # data_dir, achievements_dir, find_image_file
│       ├── icons.rs           # ICO→PNG conversion, icon resolution
│       ├── status.rs          # load_status_map (Goldberg + GOG format)
│       ├── date.rs            # Date parsing
│       └── loader.rs          # read_app_name, populate_image_paths, set_achievement_earned
│
├── config/              # ira-config (Level 1 — depends on ira-models)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Config struct + load/save
│       └── secrets.rs         # secret-tool integration
│
├── api/                 # ira-api (Level 2 — depends on ira-models, ira-parser)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # SteamClient struct + re-exports
│       ├── types.rs           # API response types (SgdbAsset, SteamGameDetails, etc.)
│       ├── util.rs            # urlencode, pick_lang
│       ├── download.rs        # download_file, fetch_image, fetch_image_fallback
│       ├── steam.rs           # Steam Web API (store, user stats, schema)
│       ├── sgdb.rs            # SteamGridDB API
│       ├── nemirtingas.rs     # Nemirtingas games-infos-datas repo
│       └── assets.rs           # Asset download orchestration
│
├── platforms/           # ira-platforms (Level 2 — depends on models, db, parser, api, config)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Re-exports
│       ├── lutris.rs          # Lutris DB reading (load_lutris_games, playtime)
│       ├── lutris_config.rs  # Lutris YAML config parsing
│       ├── lutris_watcher.rs # LutrisWatcher (file watcher for pga.db)
│       ├── steam/             # Steam local data (Goldberg saves, appdetails)
│       │   ├── mod.rs
│       │   ├── achievements.rs
│       │   ├── appinfo.rs
│       │   ├── discovery.rs
│       │   ├── paths.rs
│       │   ├── steam_setup.rs
│       │   └── vdf.rs
│       ├── gog.rs             # GOG local data (achievements, emu config)
│       ├── gog_setup.rs       # GOG game setup (detection, add)
│       ├── api_emulators.rs   # Steam/GOG API emulator installation
│       ├── consoles.rs        # Re-export from ira-models
│       ├── emulator_detect.rs # Detect emulator binaries
│       ├── rom_serial.rs      # ROM serial number extraction
│       ├── watcher_util.rs    # DebouncedFileWatcher
│       ├── ps4/               # shadPS4 (multi-file)
│       │   ├── mod.rs
│       │   ├── paths.rs
│       │   ├── psf.rs
│       │   ├── npbind.rs
│       │   ├── discovery.rs
│       │   ├── playtime.rs
│       │   ├── trophy_xml.rs
│       │   ├── game.rs
│       │   ├── watcher.rs
│       │   └── versions.rs
│       └── retroachievements/  # RetroAchievements integration
│           ├── mod.rs
│           ├── api.rs
│           ├── discovery.rs
│           └── paths.rs
│
├── images/              # ira-images (Level 2 — depends on ira-models, GTK)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # texture_for, set_image, set_picture
│       ├── cache.rs           # TextureCache (LRU)
│       └── scaled.rs          # ScaledPaintable custom widget
│
├── watcher/             # ira-watcher (Level 3 — depends on ira-config, ira-parser, ira-models)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Re-exports AchievementWatcher
│       ├── engine.rs          # AchievementWatcher + event_loop (accepts load_game closure)
│       └── notify.rs          # Desktop notifications
│
├── launcher/            # ira-launcher (depends on ira-models, ira-db)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Re-exports
│       ├── launch.rs          # launch_game (accepts LaunchContext)
│       ├── env_builder.rs     # build_env (environment variable assembly)
│       ├── wine_launch.rs     # Wine binary discovery, prefix setup
│       ├── native_launch.rs   # Native executable launching
│       └── wrapper.rs          # spawn_game, monitor_process
│
└── ira/                  # Main app (depends on all crates above)
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs             # Re-exports from ira-models for backward compat
    │   ├── main.rs            # Entry point — just main()
    │   ├── activate.rs        # activate() + pipe wakeup + background init
    │   ├── game_list.rs      # build_game_list, build_shadps4_games, auto_match
    │   ├── game_loader.rs     # load_game, load_games, read_app_details (orchestration)
    │   ├── strings.rs         # UI string constants
    │   ├── bench.rs           # Performance benchmark harness
    │   ├── ui/                # GTK UI (~28 files)
    │   │   ├── mod.rs         # Public API re-exports
    │   │   ├── state.rs       # AppState + SharedState
    │   │   ├── css.rs         # APP_CSS constant
    │   │   ├── window.rs      # build_ui, build_window
    │   │   ├── sidebar.rs     # rebuild_sidebar, build_sidebar_row
    │   │   ├── grid_view.rs   # show_grid_view, build_recent_row
    │   │   ├── grid_bin.rs    # GridBin custom widget
    │   │   ├── game_item.rs   # GameItem glib wrapper
    │   │   ├── game_display.rs # display_game, build_game_header
    │   │   ├── achievement_rows.rs # create_achievement_row, build_global_tab
    │   │   ├── image_budget.rs # ImageLoadBudget
    │   │   ├── play_button.rs # Launch/stop button
    │   │   ├── message_handler.rs # handle_app_message dispatch
    │   │   ├── helpers.rs     # merge_game_enrichment, refresh_playtime, clear_children
    │   │   ├── enrichment.rs  # enrich_game_async
    │   │   ├── background.rs  # hide_to_background, restore_content
    │   │   ├── dialogs.rs     # Settings, game settings, SGDB picker
    │   │   ├── context_menu.rs # Right-click menu
    │   │   ├── mass_match_dialog.rs # Match unmatched games
    │   │   ├── add_game.rs   # Add game flow
    │   │   ├── add_game_dialog.rs # Add game dialog
    │   │   ├── edit_game_dialog.rs # Edit game dialog
    │   │   ├── play_history.rs # Play history chart
    │   │   ├── wine_config_widget.rs # Wine config UI
    │   │   ├── profile_dialog.rs # Wine profile management
    │   │   ├── matching.rs   # match_game_to_steam/sgdb
    │   │   ├── filter.rs     # Game filtering
    │   │   └── group_dialog.rs # Collection management
    │   └── bin/
    │       └── test_main.rs  # Test binary
    └── tests/
        ├── trophy_parsing.rs
        ├── psf_parsing.rs
        ├── npbind_parsing.rs
        ├── playtime_parsing.rs
        └── game_discovery.rs
```

## Crate dependency graph

```
ira-models (L0)
├── ira-db (L1)
├── ira-parser (L1)
├── ira-config (L1)
├── ira-api (L2) ← ira-parser
├── ira-platforms (L2) ← ira-db, ira-parser, ira-api, ira-config
├── ira-images (L2)
├── ira-watcher (L3) ← ira-config, ira-parser
├── ira-launcher ← ira-db
└── ira (main) ← everything above
```

## Key principles

- **`ira-models` is a dependency leaf** — pure types, no ira-* deps, breaks all
  circular dependency risks
- **`ira-api` = HTTP** — everything that makes network requests
- **`ira-platforms` = game sources** — each platform has its own file/folder
- **`lib.rs` re-exports** — each crate's public API is defined in its `lib.rs`
- **`game_loader.rs` in the main crate** — orchestrates data from multiple
  crates (db, parser, platforms). Lives in the main crate because it depends
  on everything. Lower-level crates that need `load_game` accept it as a
  closure parameter (dependency injection).

## Dependency injection pattern

The `game_loader` module in the main `ira` crate orchestrates data from
multiple crates. It cannot be a separate crate because it depends on both
`ira-platforms` (for Steam native achievement reading) and `ira-db` (for
loading game entries).

Crates that need `load_game` but can't depend on the main crate accept it as
a closure:

- **`ira-watcher`**: `AchievementWatcher::new()` accepts
  `Arc<dyn Fn(&GameEntry, &str) -> Result<Game, String> + Send + Sync>`
- **`ira-platforms` (retroachievements)**: `build_ra_games()` accepts
  `impl Fn(&GameEntry, &str) -> Result<Game, String>`

The main crate provides `game_loader::load_game` when calling these functions.

## Game kind values

The `kind` field in the DB distinguishes emulator-specific game sources:

| Kind       | Meaning                                  | Achievements from       |
|------------|------------------------------------------|-------------------------|
| `gbe_steam`| Goldberg Steam Emulator                  | `steam/{app_id}/`       |
| `ne_gog`   | Nemirtingas GOG Emulator                 | `gog/{GALAXY_ID}/{pid}/`|
| `sgdb`     | SteamGridDB-only (images, no achievements)| —                      |
| `ps4`      | shadPS4 emulator                         | trophy XML + npbind     |

**Filesystem paths use `"steam"` and `"gog"` directories** — these are NOT the
same as the DB `kind` values. Only the `kind` column was renamed; on-disk paths
are unchanged for backward compatibility.

## Shared helpers

- `clear_children()` in `crates/ira/src/ui/helpers.rs` — replaces inline `while let Some(child)` loops
- `monitor_running_game()` in `crates/ira/src/ui/helpers.rs` — spawns child-process poll thread
- `game_entry_from_row()` + `GAME_COLUMNS` in `crates/db/src/lib.rs` — shared row mapping
- `populate_image_paths()` in `crates/parser/src/loader.rs` — shared 5-asset path probing
- `sgdb_get_json()` / `sgdb_endpoint()` in `crates/api/src/sgdb.rs` — shared SGDB request boilerplate
