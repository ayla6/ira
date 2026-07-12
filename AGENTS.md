# AGENTS.md

Guidelines for working on Ira.

## Project overview

Rust GTK4 + libadwaita desktop app for viewing game achievements across multiple
emulator platforms (Goldberg Steam Emulator, Nemirtingas GOG Emulator, shadPS4).
Lutris is the source of truth for most of the game list, but not all — shadPS4
games are discovered independently, and future platforms may also bypass Lutris.
SteamGridDB provides image assets.

## Build & test commands

```
cargo build                    # Build the main binary
cargo build --bin ira-test     # Build the test binary
cargo test                     # Run all tests
cargo build 2>&1 | grep warning # Check for warnings (should be zero)
```

**Always run `cargo build` and `cargo test` before committing.** Zero warnings is the baseline.

## Architecture: unidirectional dependency flow

Dependencies flow **upward only**. A module may import from modules below it, never
above or sideways in a way that creates a cycle.

```
Level 0 (leaf):    models/          — only serde/std imports, nothing from this crate
Level 1:           config/           — imports from models
                   db/               — imports from models
                   parser/           — imports from models
Level 2:           api/              — imports from models, parser
                   platforms/        — imports from models, db, parser, api
                   images/           — imports from models (GTK only)
Level 3:           watcher/          — imports from models, parser, db
Level 4:           ui/               — imports from everything below
Level 5:           activate.rs       — top-level orchestration
                   game_list.rs
                   migration.rs
Level 6:           main.rs           — entry point only
```

### Rules for file independence

1. **No reaching across module boundaries for internals.** If `ui/dialogs.rs` needs
   a function from `parser/`, it uses `crate::parser::function_name()` (the public
   re-export), never `crate::parser::paths::function_name()` (the internal module).

2. **`mod.rs` files contain only re-exports and module declarations.** No business
   logic. If `mod.rs` grows past ~30 lines, move logic into a named file.

3. **`pub(super)` for methods shared within a module directory.** If `api/assets.rs`
   needs a method defined in `api/download.rs`, that method is `pub(super)` — visible
   to siblings in `api/` but not to `ui/` or `platforms/`.

4. **`pub(crate)` for items needed by other modules but not externally.** Use this
   sparingly — if a type is needed by 3+ modules, it probably belongs in `models/`.

5. **One responsibility per file.** If the filename needs "and", split it.
   `db/crud.rs` (write operations) and `db/lookup.rs` (read operations) — not
   `db/crud_and_lookup.rs`.

6. **No circular `use` between sibling files.** If `ui/sidebar.rs` needs something
   from `ui/helpers.rs` and `ui/helpers.rs` needs something from `ui/sidebar.rs`,
   extract the shared dependency into a third file or move it up to `ui/mod.rs`.

7. **Prefer `super::` over `crate::` within a module.** Inside `ui/`, use
   `super::helpers::clear_children()` not `crate::ui::helpers::clear_children()`.
   This keeps the dependency local and makes refactoring easier.

## Code organization

### File size
- **Soft cap: 100 lines per function.** If longer, extract sub-functions.
- **Hard cap: 200 lines per function.** No exceptions — split it.
- **File cap: 500 lines.** If longer, split into sibling files.

### Duplication
- **Extract on the 3rd occurrence** of any pattern, not the 5th.
- If you copy-paste more than 5 lines, ask: "should this be a helper?"
- Shared helpers go in the module's `helpers.rs` (for `ui/`) or `mod.rs`
  (for smaller modules).

### Types
- **Use enums for closed sets.** Game kinds (`gbe_steam`, `ne_gog`, `sgdb`, `ps4`)
  should eventually be an enum, not string literals. Asset types (`icon`, `hero`,
  `grid`, `header`, `logo`) same.
- **`Option` only for genuinely nullable fields.** If the DB column is
  `NOT NULL DEFAULT 0`, the Rust field is `i64`, not `Option<i64>`.
- **Implement `Default` for any struct with 10+ fields.** Construction sites
  use `..Default::default()` instead of spelling out 20 empty strings.

### Error handling
- **`Result<T, String>` for all fallible operations.** Consistent across the crate.
- **Never swallow errors silently.** At minimum `eprintln!` before returning a default.
- **DB multi-step mutations use transactions.** Wrap in `c.unchecked_transaction()`
  / `tx.commit()`.
- **DB getters return `Result`, not silent defaults.** Callers decide whether to
  fall back. (Exception: `get_ignored_lutris_ids` / `get_hidden_lutris_ids` —
  these are best-effort and returning empty on error is acceptable, but
  `eprintln!` the error first.)

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
- **`models/`** — pure types, `Default` impls, `sort_key()`, `unmatched_game()`.
- **`parser/`** — `load_status_map` with Goldberg and GOG format fixtures,
  `convert_ico_to_png`, path helpers.
- **`db/`** — CRUD operations with a `tempfile` database, migration correctness.
- **`platforms/ps4/`** — PSF parsing, npbind parsing, trophy XML, playtime parsing
  (already have integration tests in `tests/`).
- **`api/`** — SGDB endpoint construction, `pick_lang`, `urlencode`. Mock HTTP
  with fixture JSON files (no real network calls in tests).

### How to test
- **Integration tests** in `tests/` — one file per concern, each is a separate
  binary. Use `tempfile` for filesystem fixtures.
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

**Scopes:** module name — `ui`, `db`, `api`, `platforms`, `parser`, `models`,
`config`, `watcher`, `images`, or `*` for cross-cutting.

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
cargo build         # must succeed with zero warnings
cargo test          # all tests must pass
git status          # review staged files
git diff --cached   # review the actual diff
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
- [ ] `cargo test` passes
- [ ] No duplication (3+ occurrences = extract a helper)
- [ ] No dead code or unused captures
- [ ] Functions under 100 lines
- [ ] New logic has at least one test
- [ ] `Option` fields match schema nullability
- [ ] No circular imports between sibling files
- [ ] Commit message follows conventional format
- [ ] No secrets in the diff
