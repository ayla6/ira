# AGENTS.md

Guidelines for working on Ira.

## Project overview

Rust GTK4 + libadwaita desktop app for viewing game achievements across multiple
emulator platforms (Goldberg Steam Emulator, Nemirtingas GOG Emulator, shadPS4) and RetroAchievements.
It has many sources of games, having its own Lutris like system of adding games, pulling games from Lutris itself, and also console emulators.
SteamGridDB provides image assets.

## Build & test commands

Run all Cargo commands inside the `rust-dev` distrobox. From the repository
root, use `distrobox enter rust-dev -- <command>`:

```
distrobox enter rust-dev -- cargo build                         # Build the main binary
distrobox enter rust-dev -- cargo build --bin ira-test          # Build the test binary
distrobox enter rust-dev -- cargo test                          # Run all tests (all crates)
distrobox enter rust-dev -- cargo test -p ira-db                # Run tests for a specific crate
distrobox enter rust-dev -- cargo clippy --all-targets -- -D warnings   # Zero clippy warnings
```

**Always run `cargo build` and `cargo test` before committing.** Zero warnings is the baseline.

### Adding dependencies

**Always use `cargo add` from the CLI, never edit `Cargo.toml` files directly.**
This ensures the lockfile stays in sync and features are set correctly:

```bash
cargo add <crate>@<version> -p <workspace-member>            # add a dependency
cargo add <crate> -p <workspace-member> --features foo,bar   # add with features
cargo add <crate>@<version> -p <workspace-member> --features bundled  # common pattern
```

If multiple workspace members depend on the same crate, update all of them
in one `cargo add` command sequence so Cargo unifies the version.

### Dead code checks

`#[allow(...)]` suppressions are forbidden. Either use the code or delete it.
If the code is actively being implemented (WIP, will be wired up in the next
commit), bear with the warning until it's connected — do not suppress it.
Serde fields that exist only for deserialization should be prefixed with `_`
(e.g. `_success`) instead of using `#[allow(dead_code)]`.
There's No need to constantly check for it, just don't add new ones.

## Architecture: Cargo workspace

The project is a Cargo workspace with 19 crates under `crates/`. Dependencies
flow upward only: a crate may depend on lower layers, never on a consumer that
would create a cycle. The compiler enforces this.

```
Leaf:              ira-models, ira-overlay-ipc, ira-input
Foundation:        ira-db, ira-parser
                    ira-config (models, overlay-ipc)
                    ira-overlay (overlay-ipc)
Integration:       ira-api (models, parser)
                    ira-platforms (models, db, parser, api, config)
                    ira-images (models, parser; GTK)
                    ira-watcher (models, config, parser)
                    ira-launcher (models, db, overlay-ipc)
Overlay outputs:   ira-overlay-vk, ira-overlay-shim, ira-overlay-standalone
Application:       ira (main GTK/libadwaita app)
```

The main `ira` crate contains: `ui/`, `activate.rs`, `game_list.rs`,
`game_loader.rs`, `strings.rs`, `bench.rs`, `main.rs`, and integration tests.

### Crate boundaries

1. **Cross-crate imports use the `ira_X::` prefix.** For example,
   `ira_models::Game`, `ira_db::DbConn`, `ira_parser::data_dir`.
   Within a crate, use `crate::` for self-references.

2. **`lib.rs` files (crate roots) contain only re-exports and module
   declarations.** No business logic. If `lib.rs` grows past ~30 lines,
   move logic into a named file.

3. **`pub(crate)` for items shared within a crate but not externally.** If
   `api/assets.rs` needs a method defined in `api/download.rs`, that method
   is `pub(crate)` — visible to siblings in the same crate but not to other
   crates.

4. **`pub` for items needed by other crates.** Use this sparingly — if a type
   is needed by 3+ crates, it probably belongs in `ira-models`.

5. **One responsibility per file.** If the filename needs "and", split it.
   `db/crud.rs` (write operations) and `db/lookup.rs` (read operations) — not
   `db/crud_and_lookup.rs`.

6. **No circular `use` between sibling files.** If `ui/sidebar.rs` needs
   something from `ui/helpers.rs` and `ui/helpers.rs` needs something from
   `ui/sidebar.rs`, extract the shared dependency into a third file or move
   it up to `ui/mod.rs`.

7. **Prefer `super::` over `crate::` within a module.** Inside `ui/`, use
   `super::helpers::clear_children()` not `crate::ui::helpers::clear_children()`.
   This keeps the dependency local and makes refactoring easier.

### Dependency injection for circular-breaking patterns

The `game_loader` module in the main `ira` crate contains `load_game` and
`load_games`, which orchestrate data from multiple crates (db, parser,
platforms). The watcher and retroachievements modules need `load_game` but
cannot depend on the main crate. They accept it as a closure parameter:

```rust
// watcher accepts load_game as a closure
pub fn new(cfg: Arc<Config>, sender: AppSender, save_dir: String,
           load_game: Arc<dyn Fn(&GameEntry, &str) -> Result<Game, String> + Send + Sync>)
```

## Code organization

### Migration lifecycle

**Before the first release:** one-time data migrations (e.g. `UPDATE games SET
game_id = steam_id ...`) should be removed once confirmed working on the user's
database. Schema migrations (`ensure_column`) stay forever — data migrations
don't. Mark them with a comment like `// PRE-RELEASE: remove after v0.X` so
they're easy to grep for. After release, schema migrations are the only
permanent migration mechanism.

### File size
- **Soft cap: 100 lines per function.** If longer, extract sub-functions.
- **Hard cap: 200 lines per function.** No exceptions — split it.
- **File cap: 500 lines.** If longer, split into sibling files.

### Duplication
- **Extract on the 3rd occurrence** of any pattern, not the 5th.
- If you copy-paste more than 5 lines, ask: "should this be a helper?"
- Shared helpers go in the module's `helpers.rs` (for `ui/`) or `lib.rs`
  (for smaller crates).
- If you need a paragraph-long comment to justify why the workaround is OK,
  the code is wrong — fix the code.

### Types
- **Use enums for closed sets.** Game kinds (`gbe_steam`, `ne_gog`, `sgdb`, `ps4`)
  should eventually be an enum, not string literals. Asset types (`icon`, `hero`,
  `grid`, `header`, `logo`) same.
- **Always use defined constants for closed sets.** If `models/kind.rs` defines
  `pub const STEAM: &str = "steam"`, never write `"steam"` as a literal in
  comparisons or assignments. Raw string literals for closed sets are typo-prone
  and make refactoring harder.
- **Fields are named for what they store, not what they were originally used
  for.** If `steam_id` stores NPWR IDs and RA game IDs, it's misnamed. Either
  rename it or add a separate column. Field names are API contracts —
  repurposing them silently is a bug.
- **`Option` only for genuinely nullable fields.** If the DB column is
  `NOT NULL DEFAULT 0`, the Rust field is `i64`, not `Option<i64>`.
- **Implement `Default` for any struct with 10+ fields.** Construction sites
  use `..Default::default()` instead of spelling out 20 empty strings.
- **Never use `#[allow(dead_code)]` to suppress warnings.** Either use the
  field/function or remove it. Serde fields that exist only for deserialization
  should be prefixed with `_` (e.g. `_success`) if truly unused.

### Error handling
- **`Result<T, String>` for all fallible operations.** Consistent across all crates.
- **Never swallow errors silently.** At minimum `eprintln!` before returning a default.
- **DB multi-step mutations use transactions.** Wrap in `c.unchecked_transaction()`
  / `tx.commit()`.
- **DB getters return `Result`, not silent defaults.** Callers decide whether to
  fall back. (Exception: `get_ignored_lutris_ids` / `get_hidden_lutris_ids` —
  these are best-effort and returning empty on error is acceptable, but
  `eprintln!` the error first.)

### Message variants
- **Every `AppMessage` variant must be both sent and handled.** Before merging
  a new variant, verify the send site exists. During review, grep for
  `AppMessage::VariantName` — if it only appears in the enum definition and the
  match arm, it's dead code.

### Lookups
- **When you have `db_id`, use `find_by_db_id`.** Never look up a secondary key
  (steam_id, game_id, etc.) just to resolve the primary key you already have.

## Style

- **Prefer functional programming patterns when they are more readable and don't have a performance or memory drawback.**

## UI guidelines

### GTK patterns
- **Use `clear_children()` helper** from `ui/helpers.rs`. Never inline
  `while let Some(child) = w.first_child() { w.remove(&child); }`.
- **Collapse `state.borrow()` chains.** One borrow block, destructure what you need:
  ```rust
  let (steam, watcher, sender) = {
      let s = state.borrow();
      (s.steam.clone(), s.watcher.clone(), s.sender.clone())
  };
  ```
- **Background threads for any I/O.** Never block the GTK main loop. Use
  `AppSender` to send results back.
- **Separate widget construction from business logic.** A function that builds
  a dialog should not also fetch data from the network.

### Widget construction
- **Functions return the top-level widget**, not take a parent container.
- **Factory closures in `SignalListItemFactory`** should be named functions, not
  inline 40-line closures.
- **CSS classes are defined in `ui/css.rs`**, not scattered as string literals.

## Testing

### What to test
- **`ira-models`** — pure types, `Default` impls, `sort_key()`, `unmatched_game()`.
- **`ira-parser`** — `load_status_map` with Goldberg and GOG format fixtures,
  `convert_ico_to_png`, path helpers.
- **`ira-db`** — CRUD operations with a `tempfile` database, migration correctness.
- **`ira-platforms` (ps4)** — PSF parsing, npbind parsing, trophy XML, playtime parsing
  (integration tests in `crates/ira/tests/`).
- **`ira-api`** — SGDB endpoint construction, `pick_lang`, `urlencode`. Mock HTTP
  with fixture JSON files (no real network calls in tests).

### How to test
- **Integration tests** in `crates/ira/tests/` — one file per concern, each is a
  separate binary. Use `tempfile` for filesystem fixtures.
- **Unit tests** in `#[cfg(test)] mod tests` at the bottom of source files —
  for pure functions only.
- **Test names**: `test_<function>_<scenario>` — e.g. `test_parse_playtime_hms`,
  `test_serial_to_lutris_id_stable`.
- **Every new pure function gets at least one test.** If it has edge cases
  (empty input, invalid data, boundary values), test those too.

### Test structure
```rust
#[test]
fn test_load_status_map_goldberg_format() {
    // Arrange
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), r#"{"ACH_NAME":{"earned":true,"earned_time":12345}}"#).unwrap();
    // Act
    let map = load_status_map(tmp.path());
    // Assert
    assert_eq!(map.len(), 1);
    assert!(map["ACH_NAME"].earned);
    assert_eq!(map["ACH_NAME"].earned_time, 12345);
}
```

## Commits

### Commit message format

Use conventional commits:

```
<type>(<scope>): <description>

<optional body>
```

**Types:**
- `feat` — new feature (new functionality, new UI element)
- `fix` — bug fix
- `refactor` — code restructuring without behavior change
- `test` — adding or modifying tests
- `docs` — documentation only
- `chore` — build, config, dependencies, tooling
- `style` — formatting, CSS, whitespace (no logic change)

**Scopes:** crate name — `ui`, `db`, `api`, `platforms`, `parser`, `models`,
`config`, `watcher`, `images`, `launcher`, or `*` for cross-cutting.

**Examples:**
```
feat(ui): add shadPS4 version selector to game settings
fix(api): restore secret-tool key loading for SGDB API
refactor(db): extract shared game_entry_from_row helper
test(parser): add Goldberg status map parsing tests
chore(*): rename kind values steam→gbe_steam, gog→ne_gog
```

### Commit principles

1. **One logical change per commit.** A refactor and a bug fix are two commits,
   not one. A rename across files is one commit.

2. **Commit message describes WHY, not WHAT.** The diff shows what changed. The
   message explains the reasoning:
   ```
   # Bad:  "changed kind strings"
   # Good: "rename kind values to distinguish emulator games from native Steam/GOG"
   ```

3. **Small commits.** If the diff is over 300 lines, consider splitting. Each
   commit should build and pass tests on its own.

4. **Never commit secrets.** API keys go in the system keyring via `secret-tool`,
   never in config files or source code. The `config::save()` method strips keys
   before writing to disk.

5. **Stage only intended files.** Before committing, review `git status` and
   `git diff`. Never `git add .` blindly — exclude build artifacts, temporary
   files, and unrelated changes.

6. **Present tense, imperative mood.** "Add feature" not "Added feature" or
   "Adds feature".

7. **No emoji in commit messages.** Plain text only.

### Before committing

```bash
cargo build                              # must succeed with zero warnings
cargo clippy --all-targets -- -D warnings # zero clippy warnings
cargo test                               # all tests must pass
rg '#\[allow' crates/                    # must return nothing
git status                               # review staged files
git diff --cached                        # review the actual diff
```

### Commit workflow

```bash
git add <specific files>
git commit -m "type(scope): description"
```

For multi-line commit messages:
```bash
git commit -m "refactor(db): extract shared game_entry_from_row helper

Eliminates 4×15 lines of duplicated row-to-GameEntry mapping.
Adds GAME_COLUMNS constant so column list stays in sync."
```

## Code review checklist

- [ ] Zero compiler warnings
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] No `#[allow(...)]` suppressions (`rg '#\[allow' crates/` returns nothing)
- [ ] No duplication (3+ occurrences = extract a helper)
- [ ] No dead code or unused captures
- [ ] Functions under 100 lines
- [ ] New logic has at least one test
- [ ] `Option` fields match schema nullability
- [ ] No circular imports between sibling files
- [ ] No raw string literals for closed sets (use constants from `ira-models`)
- [ ] `lib.rs` files contain only `mod`/`pub use` declarations
- [ ] Every `AppMessage` variant is both sent and handled
- [ ] Commit message follows conventional format
- [ ] No secrets in the diff
