# Architecture

## Directory layout

```
src/
├── main.rs                  # Entry point — just main()
├── activate.rs              # activate() + pipe wakeup + background init
├── game_list.rs             # build_game_list, build_shadps4_games, auto_match
├── migration.rs             # migrate_data_dir, populate_db_from_dirs
├── app.rs                   # AppSender (pipe-based wakeup channel)
├── strings.rs               # UI string constants
├── bench.rs                 # Performance benchmark harness
│
├── models/                  # Shared domain types — dependency leaf
│   ├── game.rs              # Game struct, sort_key, unmatched_game
│   ├── game_entry.rs        # GameEntry (DB row representation)
│   ├── achievement.rs       # MergedAchievement, AchievementStatus, StringOrMap
│   └── message.rs           # AppMessage enum + AppSender
│
├── config/                  # Configuration management
│   ├── mod.rs               # Config struct + load/save
│   └── secrets.rs           # secret-tool integration
│
├── db/                      # Database layer
│   ├── mod.rs               # DbConn, init_db, schema migrations
│   ├── crud.rs              # add_game, remove_game, load_all_games
│   ├── lookup.rs            # find_by_steam_id, find_by_kind_platform, etc.
│   ├── lutris_ops.rs        # upsert_matching, unmatch_game, set_lutris_db_id
│   └── settings.rs          # Per-game settings updates
│
├── api/                     # Online data sources (HTTP)
│   ├── mod.rs               # Re-exports
│   ├── types.rs             # Shared API response types
│   ├── util.rs              # urlencode, pick_lang
│   ├── download.rs          # download_file, fetch_image, fetch_image_fallback
│   ├── steam.rs             # Steam Web API (store, user stats, schema)
│   ├── sgdb.rs              # SteamGridDB API
│   ├── nemirtingas.rs       # Nemirtingas games-infos-datas repo
│   └── assets.rs            # Asset download orchestration
│
├── platforms/               # Game/achievement sources
│   ├── mod.rs               # Re-exports
│   ├── lutris.rs            # Lutris DB reading (load_lutris_games, playtime)
│   ├── lutris_watcher.rs    # LutrisWatcher (file watcher for pga.db)
│   ├── steam.rs             # Steam local data (Goldberg saves, appdetails)
│   ├── steam_setup.rs       # Steam game setup (detect, add)
│   ├── gog.rs               # GOG local data (achievements, emu config)
│   ├── gog_setup.rs         # GOG game setup (detection, add)
│   └── ps4/                 # shadPS4 (multi-file)
│       ├── mod.rs           # Re-exports
│       ├── paths.rs         # shadps4_user_dir, play_time_path, trophy_dir
│       ├── psf.rs           # param.sfo binary parsing
│       ├── npbind.rs        # npbind.dat binary parsing
│       ├── discovery.rs     # Game discovery
│       ├── playtime.rs      # Playtime parsing
│       ├── trophy_xml.rs    # TROP.XML + user trophy XML
│       ├── game.rs          # load_shadps4_game, serial_to_lutris_id
│       ├── watcher.rs       # ShadPS4Watcher
│       └── versions.rs      # Qt Launcher version management
│
├── parser/                  # File parsing utilities
│   ├── mod.rs               # load_games, load_game (orchestrator)
│   ├── paths.rs             # data_dir, achievements_dir, find_image_file
│   ├── icons.rs             # ICO→PNG conversion, icon resolution
│   └── status.rs            # load_status_map (Goldberg + GOG format)
│
├── images/                  # Texture loading and caching
│   ├── mod.rs               # texture_for, set_image, set_picture
│   ├── cache.rs             # TextureCache (LRU)
│   └── scaled.rs            # ScaledPaintable custom widget
│
├── watcher/                 # Live achievement file monitoring
│   ├── mod.rs               # AchievementWatcher + event_loop
│   └── notify.rs            # Desktop notifications
│
├── ui/                      # GTK UI (~22 files)
│   ├── mod.rs               # Public API re-exports
│   ├── state.rs             # AppState + SharedState
│   ├── css.rs               # APP_CSS constant
│   ├── window.rs            # build_ui, build_window
│   ├── sidebar.rs           # rebuild_sidebar, build_sidebar_row
│   ├── grid_view.rs         # show_grid_view, build_recent_row
│   ├── grid_bin.rs          # GridBin custom widget
│   ├── game_item.rs         # GameItem glib wrapper
│   ├── game_display.rs      # display_game, build_game_header
│   ├── achievement_rows.rs  # create_achievement_row, build_global_tab
│   ├── image_budget.rs      # ImageLoadBudget
│   ├── play_button.rs       # Launch/stop button
│   ├── message_handler.rs   # handle_app_message dispatch
│   ├── helpers.rs           # merge_game_enrichment, refresh_playtime
│   ├── enrichment.rs        # enrich_game_async
│   ├── background.rs        # hide_to_background, restore_content
│   ├── dialogs.rs           # Settings, game settings, SGDB picker
│   ├── context_menu.rs      # Right-click menu
│   ├── mass_match_dialog.rs # Match unmatched games
│   ├── add_game.rs          # Add game flow
│   └── matching.rs          # match_game_to_steam/sgdb
│
├── bin/
│   └── test_main.rs         # Test binary
│
└── lib.rs                   # Library crate root (for tests)

tests/                       # Integration tests
```

## Key principles

- **`models/` is a dependency leaf** — imports only serde/std, breaks all circular dep risks
- **`api/` = HTTP** — everything that makes network requests
- **`platforms/` = game sources** — each platform has its own file/folder
- **`mod.rs` re-exports** — all existing `crate::foo::*` import paths stay valid
- **Flat for <80 lines** — tiny files stay flat (app.rs, strings.rs, bench.rs)

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

- `clear_children()` in `ui/helpers.rs` — replaces inline `while let Some(child)` loops
- `monitor_running_game()` in `ui/helpers.rs` — spawns child-process poll thread
- `game_entry_from_row()` + `GAME_COLUMNS` in `db/mod.rs` — shared row mapping
- `populate_image_paths()` in `parser/mod.rs` — shared 5-asset path probing
- `sgdb_get_json()` / `sgdb_endpoint()` in `api/sgdb.rs` — shared SGDB request boilerplate
