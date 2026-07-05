# TODO

## Immediate fixes

- [ ] **Clean up dead code**: Remove leftover params, unify the two texture caches in `images.rs`, remove unnecessary wrapper Box in `show_grid_view`.
- [ ] **Fix hero aspect ratio**: The hero banner's aspect ratio changes with window size. Should maintain a consistent ratio.

## Lutris integration

- [ ] Read `~/.local/share/lutris/pga.db` for games (name, slug, runner, steamid, gogid, playtime)
- [ ] Auto-match: match by `steamid` (Steam games) or `gogid` (GOG games) or fuzzy name match
- [ ] Store `lutris_id` (slug) in our DB (field already exists)
- [ ] Play button: launch via `lutris:rungame/<slug>` command
- [ ] Display playtime from Lutris DB, with a refresh button
- [ ] Last played data pulled from Lutris
- [ ] Note: Lutris only stores total playtime numbers, not per-week breakdowns. Don't try to compute that ourselves.

## UI improvements

- [ ] **Hero banner redesign**: Professional Steam-like hero with game logo overlaid, play button, playtime/last played stats
- [ ] **Game settings dialog**: Edit title, Steam ID, Lutris slug, remove game (right-click sidebar row — partially done)
- [ ] **Hamburger menu polish**: All text same size, left-aligned, zoom slider on same line as label (partially done)
- [ ] **Scroll reset**: Should happen instantly when switching games, not after a visible jump (fixed — now happens before content removal)

## RetroArch / retroachievements support (future)

- [ ] Not every game will have a Steam entry or ID — the DB schema already supports arbitrary `kind` values
- [ ] Will need a new `kind` (e.g. "retro") with its own achievement format parser
- [ ] May need a different asset source (not Steam grids)
- [ ] Consider how to handle games without Steam App IDs in the `data/` folder structure

## Architecture / code quality

- [ ] **Unify texture caches**: `TEXTURE_CACHE` and `SCALED_CACHE` in `images.rs` could be a single cache keyed by `(path, Option<(width, height)>)`
- [ ] **Remove dead code**: Clean up leftover params, unused CSS classes, old layout attempts
- [ ] **Consider replacing FlowBox**: FlowBox's `set_homogeneous(true)` affects both axes. A custom layout or `GridView` (GTK 4.6+) might give better control over horizontal vs vertical sizing.
- [ ] **Config migration**: When adding new config fields, handle old config files gracefully (already using `#[serde(default)]`)
