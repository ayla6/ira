package main

import (
	"fmt"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/glib/v2"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
	"github.com/diamondburned/gotk4/pkg/pango"
)

// sidebarRowWidgets holds references to the widgets inside one sidebar entry
// so we can update them in place once background enrichment finishes for
// that game, without rebuilding the whole list.
type sidebarRowWidgets struct {
	row      *gtk.ListBoxRow
	icon     *gtk.Image
	title    *gtk.Label
	subtitle *gtk.Label
}

// uiState bundles everything buildUI's closures need to share, so the
// top-level onGameUpdated callback (invoked from main.go's background
// goroutines) can refresh the right widgets.
type uiState struct {
	window        *adw.ApplicationWindow
	games         []*Game
	rows          []*sidebarRowWidgets
	gameList      *gtk.ListBox
	contentScroll *gtk.ScrolledWindow
	contentBox    *gtk.Box
	emptyState    *adw.StatusPage
	selectedID    string
	cfg           *Config
	steam         *SteamClient
	watcher       *AchievementWatcher

	// scrollPositions remembers each game's last scroll offset, so flipping
	// back to a game you were previously scrolled through returns you right
	// where you left off. Switching to a game with no remembered position
	// (i.e. you haven't visited it this session) starts at the top instead
	// of inheriting whatever the previous game happened to be scrolled to.
	scrollPositions map[string]float64
}

// onGameUpdated is invoked (always on the GTK main thread, via glib.IdleAdd)
// whenever background enrichment finishes loading fresh data for a game.
var onGameUpdated func(updated Game)

// onNewGameDiscovered is invoked (always on the GTK main thread, via
// glib.IdleAdd) when a game that wasn't previously in the sidebar shows up -
// either through the "Add Game" dialog or because the watcher noticed a new
// folder appear in the saves directory on its own.
var onNewGameDiscovered func(game Game)

// buildUI constructs the main window layout.
func buildUI(window *adw.ApplicationWindow, games []Game, cfg *Config, steam *SteamClient, watcher *AchievementWatcher) {
	state := &uiState{window: window, cfg: cfg, steam: steam, watcher: watcher}
	for i := range games {
		g := games[i]
		state.games = append(state.games, &g)
	}

	splitView := gtk.NewPaned(gtk.OrientationHorizontal)
	splitView.SetPosition(260)
	splitView.SetShrinkStartChild(false)
	splitView.SetResizeStartChild(false)

	// ── Sidebar ──────────────────────────────────────────────────────────────
	// Never scroll horizontally: long titles must ellipsize and show their
	// full text in a tooltip instead of forcing the sidebar to grow or scroll.
	sidebarScroll := gtk.NewScrolledWindow()
	sidebarScroll.SetPolicy(gtk.PolicyNever, gtk.PolicyAutomatic)
	sidebarScroll.SetSizeRequest(220, -1)
	sidebarScroll.SetVExpand(true)

	gameList := gtk.NewListBox()
	gameList.AddCSSClass("navigation-sidebar")
	sidebarScroll.SetChild(gameList)
	state.gameList = gameList

	splitView.SetStartChild(sidebarScroll)

	// ── Content area ─────────────────────────────────────────────────────────
	contentScroll := gtk.NewScrolledWindow()
	contentScroll.SetPolicy(gtk.PolicyNever, gtk.PolicyAutomatic)
	contentBox := gtk.NewBox(gtk.OrientationVertical, 0)
	contentScroll.SetChild(contentBox)
	splitView.SetEndChild(contentScroll)
	state.contentScroll = contentScroll
	state.contentBox = contentBox
	state.scrollPositions = make(map[string]float64)

	// ── Header bar ───────────────────────────────────────────────────────────
	headerBar := adw.NewHeaderBar()

	settingsBtn := gtk.NewButton()
	settingsBtn.SetIconName("preferences-system-symbolic")
	settingsBtn.SetTooltipText("Settings")
	settingsBtn.AddCSSClass("flat")
	headerBar.PackEnd(settingsBtn)

	settingsBtn.ConnectClicked(func() {
		showSettingsDialog(window, cfg, steam)
	})

	addBtn := gtk.NewButton()
	addBtn.SetIconName("list-add-symbolic")
	addBtn.SetTooltipText("Add a game by pointing at its install folder")
	addBtn.AddCSSClass("flat")
	headerBar.PackStart(addBtn)

	toolbarView := adw.NewToolbarView()
	toolbarView.AddTopBar(headerBar)
	toolbarView.SetContent(splitView)
	window.SetContent(toolbarView)

	// ── Empty state ───────────────────────────────────────────────────────────
	emptyState := adw.NewStatusPage()
	emptyState.SetTitle("No Game Selected")
	emptyState.SetDescription("Select a game from the sidebar to view achievements.")
	emptyState.SetIconName("applications-games-symbolic")
	contentBox.Append(emptyState)
	state.emptyState = emptyState

	rebuildSidebar(state)

	gameList.ConnectRowSelected(func(row *gtk.ListBoxRow) {
		if row == nil {
			return
		}
		idx := row.Index()
		if idx >= 0 && idx < len(state.games) {
			switchToGame(state, state.games[idx].AppID)
		}
	})

	addBtn.ConnectClicked(func() {
		showAddGameDialog(state)
	})

	// Wire up the global callbacks used by background enrichment goroutines.
	onGameUpdated = func(updated Game) {
		applyGameUpdate(state, updated)
	}
	onNewGameDiscovered = func(game Game) {
		insertOrUpdateGame(state, game)
	}
}

// rebuildSidebar clears and repopulates the game list from state.games. Used
// both for the initial population and after a new game is added.
func rebuildSidebar(state *uiState) {
	for child := state.gameList.FirstChild(); child != nil; child = state.gameList.FirstChild() {
		state.gameList.Remove(child)
	}
	state.rows = nil

	for _, g := range state.games {
		state.rows = append(state.rows, buildSidebarRow(state.gameList, g))
	}
}

// buildSidebarRow creates one sidebar entry and appends it to list.
func buildSidebarRow(list *gtk.ListBox, game *Game) *sidebarRowWidgets {
	row := gtk.NewListBoxRow()

	hbox := gtk.NewBox(gtk.OrientationHorizontal, 10)
	hbox.SetMarginTop(8)
	hbox.SetMarginBottom(8)
	hbox.SetMarginStart(10)
	hbox.SetMarginEnd(10)

	var icon *gtk.Image
	if game.IconPath != "" {
		icon = gtk.NewImageFromFile(game.IconPath)
	} else {
		icon = gtk.NewImageFromIconName("application-x-executable")
	}
	icon.SetPixelSize(32)
	hbox.Append(icon)

	vbox := gtk.NewBox(gtk.OrientationVertical, 2)
	vbox.SetVAlign(gtk.AlignCenter)
	vbox.SetHExpand(true)

	titleLabel := gtk.NewLabel(game.Name)
	titleLabel.SetXAlign(0)
	titleLabel.AddCSSClass("heading")
	titleLabel.AddCSSClass("sidebar-row-title")
	titleLabel.SetEllipsize(pango.EllipsizeEnd)
	titleLabel.SetHExpand(true)
	titleLabel.SetTooltipText(fmt.Sprintf("%s (App ID: %s)", game.Name, game.AppID))
	vbox.Append(titleLabel)

	pct := 0
	if game.TotalCount > 0 {
		pct = game.EarnedCount * 100 / game.TotalCount
	}
	countLabel := gtk.NewLabel(fmt.Sprintf("%d/%d · %d%%", game.EarnedCount, game.TotalCount, pct))
	countLabel.SetXAlign(0)
	countLabel.AddCSSClass("dim-label")
	countLabel.AddCSSClass("caption")
	countLabel.SetEllipsize(pango.EllipsizeEnd)
	vbox.Append(countLabel)

	hbox.Append(vbox)
	row.SetChild(hbox)
	list.Append(row)

	return &sidebarRowWidgets{row: row, icon: icon, title: titleLabel, subtitle: countLabel}
}

// mergeGameEnrichment fills in any enrichment data updated is missing (a
// resolved title, downloaded icon/hero art, per-achievement global unlock
// percentages) from the existing, previously-enriched copy.
//
// This matters because not every source of a Game update has that
// enrichment available: a plain loadGame (e.g. from the live filesystem
// watcher reacting to an achievement being unlocked, or a manual "mark as
// unlocked" edit) only reads what's on local disk under steam_settings/ —
// it has no way to know about the title/icon/hero art resolved from Steam or
// the Nemirtingas repo, or global unlock percentages fetched from the Steam
// API, since none of that is persisted back to disk anywhere loadGame reads
// from. Without this merge, every live-reload would regress the game back
// to its pre-enrichment state (losing its title, hero banner, and icon)
// even though nothing about that data actually changed.
func mergeGameEnrichment(existing, updated Game) Game {
	if strings.HasPrefix(updated.Name, "App ID:") && !strings.HasPrefix(existing.Name, "App ID:") {
		updated.Name = existing.Name
	}
	if updated.IconPath == "" {
		updated.IconPath = existing.IconPath
	}
	if updated.HeroImagePath == "" {
		updated.HeroImagePath = existing.HeroImagePath
	}

	if len(existing.Achievements) > 0 {
		existingPct := make(map[string]float64, len(existing.Achievements))
		for _, a := range existing.Achievements {
			existingPct[a.Name] = a.GlobalPercent
		}
		for i := range updated.Achievements {
			if updated.Achievements[i].GlobalPercent == 0 {
				updated.Achievements[i].GlobalPercent = existingPct[updated.Achievements[i].Name]
			}
		}
	}

	return updated
}

// applyGameUpdate replaces a game's data (after background enrichment) and
// refreshes only the affected sidebar row / content view.
func applyGameUpdate(state *uiState, updated Game) {
	for i, g := range state.games {
		if g.AppID != updated.AppID {
			continue
		}
		merged := mergeGameEnrichment(*state.games[i], updated)
		*state.games[i] = merged

		if i < len(state.rows) {
			rw := state.rows[i]
			rw.title.SetText(merged.Name)
			rw.title.SetTooltipText(fmt.Sprintf("%s (App ID: %s)", merged.Name, merged.AppID))
			pct := 0
			if merged.TotalCount > 0 {
				pct = merged.EarnedCount * 100 / merged.TotalCount
			}
			rw.subtitle.SetText(fmt.Sprintf("%d/%d · %d%%", merged.EarnedCount, merged.TotalCount, pct))
			if merged.IconPath != "" {
				setImageAsync(rw.icon, merged.IconPath)
			}
		}

		if state.selectedID == merged.AppID {
			// This is a live refresh of the game already on screen (e.g. a new
			// achievement just got unlocked), not the user switching games, so
			// keep them exactly where they were scrolled to instead of jumping
			// anywhere.
			current := state.contentScroll.VAdjustment().Value()
			displayGame(state.games[i], state)
			glib.IdleAdd(func() {
				state.contentScroll.VAdjustment().SetValue(current)
			})
		}
		return
	}
}

// setImageAsync sets an already-downloaded local image file on img.
//
// By the time any code in this file references an achievement/game icon
// path, that file is guaranteed to already exist on disk: achievement art is
// fully downloaded during background enrichment (see enrichGameAsync in
// main.go) *before* a Game is ever handed to the UI, and findIconPath
// (parser.go) only ever returns paths that pass an os.Stat check. Decoding a
// single small local PNG/JPEG is sub-millisecond, so there's no need to hop
// through a goroutine for it — that would just add a round trip and a frame
// of placeholder-then-pop-in flicker for something already cheap.
//
// The cost that *does* matter is decoding hundreds of them back-to-back on
// the main thread in one go (e.g. opening a 280-achievement game), which is
// what imageLoader/queueImageLoads below are for: only the handful of rows
// visible immediately are loaded through this function eagerly, the rest are
// deferred and trickled in via GLib idle callbacks.
func setImageAsync(img *gtk.Image, path string) {
	if path == "" {
		return
	}
	img.SetFromFile(path)
}

// setPictureAsync is the gtk.Picture equivalent of setImageAsync.
func setPictureAsync(pic *gtk.Picture, path string) {
	if path == "" {
		return
	}
	pic.SetFilename(path)
}

// imageLoadRequest is one deferred (not-yet-visible) icon load.
type imageLoadRequest struct {
	img  *gtk.Image
	path string
}

// imageLoader lets row-builder functions stay agnostic of whether an icon
// should be loaded immediately (because the row will be visible the instant
// the tab is shown) or deferred (because it's below the fold).
type imageLoader func(img *gtk.Image, path string)

// eagerImageBudget is how many icons get loaded immediately per tab —
// comfortably more than fit in a typical window's initial viewport, so nothing
// on-screen ever shows a placeholder. Everything past this is deferred.
const eagerImageBudget = 18

// queueImageLoads progressively sets images on rows that are very likely
// scrolled out of view when a tab is first opened. Work happens in small
// batches on GLib's low-priority idle queue so it never competes with
// painting or input handling, and it typically finishes well before a user
// could scroll down far enough to notice a still-unset icon.
func queueImageLoads(reqs []imageLoadRequest) {
	if len(reqs) == 0 {
		return
	}
	const batchSize = 12
	i := 0
	glib.IdleAddPriority(glib.PriorityLow, func() bool {
		end := i + batchSize
		if end > len(reqs) {
			end = len(reqs)
		}
		for _, r := range reqs[i:end] {
			r.img.SetFromFile(r.path)
		}
		i = end
		return i < len(reqs)
	})
}

// newImageLoaderBudget returns an imageLoader factory: the first
// eagerImageBudget calls load immediately (so whatever's visible the moment
// a tab appears is already correct, no flicker), and every call after that
// defers loading via queueImageLoads instead of blocking row construction.
func newImageLoaderBudget(budget int) (next func() imageLoader, flush func()) {
	remaining := budget
	var deferred []imageLoadRequest
	next = func() imageLoader {
		if remaining > 0 {
			remaining--
			return setImageAsync
		}
		return func(img *gtk.Image, path string) {
			if path == "" {
				return
			}
			deferred = append(deferred, imageLoadRequest{img: img, path: path})
		}
	}
	flush = func() {
		queueImageLoads(deferred)
		deferred = nil
	}
	return next, flush
}

// showAddGameDialog walks the user through adding a new game: pick its
// install folder, detect (or ask for) its Steam App ID, then wire everything
// up and reload.
func showAddGameDialog(state *uiState) {
	chooser := gtk.NewFileChooserNative(
		"Select Game Folder",
		&state.window.Window,
		gtk.FileChooserActionSelectFolder,
		"Select",
		"Cancel",
	)
	chooser.ConnectResponse(func(response int) {
		if response != int(gtk.ResponseAccept) {
			return
		}
		file := chooser.File()
		if file == nil {
			return
		}
		folder := file.Path()
		if folder == "" {
			return
		}

		if appID, ok := detectAppID(folder); ok {
			finishAddGame(state, folder, appID)
			return
		}

		promptForAppID(state, folder)
	})
	chooser.Show()
}

// promptForAppID is shown when we couldn't auto-detect a steam_appid.txt in
// the chosen folder.
func promptForAppID(state *uiState, folder string) {
	dialog := adw.NewMessageDialog(&state.window.Window, "Enter Steam App ID",
		"No steam_appid.txt was found in this folder. Enter the game's Steam App ID to continue.")

	entry := gtk.NewEntry()
	entry.SetPlaceholderText("e.g. 1687950")
	entry.SetInputPurpose(gtk.InputPurposeDigits)
	entry.SetMarginTop(8)
	entry.SetMarginBottom(8)
	entry.SetMarginStart(8)
	entry.SetMarginEnd(8)
	dialog.SetExtraChild(entry)

	dialog.AddResponse("cancel", "Cancel")
	dialog.AddResponse("add", "Add Game")
	dialog.SetResponseAppearance("add", adw.ResponseSuggested)
	dialog.SetDefaultResponse("add")
	dialog.SetCloseResponse("cancel")

	dialog.ConnectResponse(func(response string) {
		if response != "add" {
			return
		}
		finishAddGame(state, folder, entry.Text())
	})

	dialog.Show()
}

// finishAddGame performs the actual filesystem wiring + achievement download,
// then inserts (or refreshes) the game in the sidebar.
func finishAddGame(state *uiState, folder, appID string) {
	go func() {
		gameDir, err := AddGameFromFolder(folder, appID, state.steam)
		if err != nil {
			fmt.Println("Add game failed:", err)
			glib.IdleAdd(func() {
				errDialog := adw.NewMessageDialog(&state.window.Window, "Couldn't Add Game", err.Error())
				errDialog.AddResponse("ok", "OK")
				errDialog.SetDefaultResponse("ok")
				errDialog.SetCloseResponse("ok")
				errDialog.Show()
			})
			if gameDir == "" {
				return
			}
		}

		game, lerr := loadGame(filepath.Base(gameDir), gameDir)
		if lerr != nil {
			fmt.Println("Failed to load newly added game:", lerr)
			return
		}
		if strings.HasPrefix(game.Name, "App ID:") {
			if name, nerr := state.steam.FetchNemirtingasGameName(game.AppID); nerr == nil && name != "" {
				game.Name = name
			}
		}

		glib.IdleAdd(func() {
			insertOrUpdateGame(state, game)
		})

		if state.watcher != nil {
			state.watcher.Watch(game.AppID, gameDir, game.Achievements)
		}
		enrichGameAsync(game.AppID, gameDir, state.steam, state.watcher, onNewGameDiscovered)
	}()
}

// insertOrUpdateGame adds a freshly-added game to the sidebar (or refreshes
// it if it already existed), keeping the list sorted by name.
func insertOrUpdateGame(state *uiState, game Game) {
	for i, g := range state.games {
		if g.AppID == game.AppID {
			*state.games[i] = game
			rebuildSidebar(state)
			return
		}
	}
	state.games = append(state.games, &game)
	sort.Slice(state.games, func(i, j int) bool {
		return state.games[i].Name < state.games[j].Name
	})
	rebuildSidebar(state)
}

// showSettingsDialog opens a modal window for editing the Steam and SteamGridDB API keys.
func showSettingsDialog(parent *adw.ApplicationWindow, cfg *Config, steam *SteamClient) {
	dialog := adw.NewWindow()
	dialog.SetTitle("Settings")
	dialog.SetDefaultSize(450, 360)
	dialog.SetModal(true)
	dialog.SetTransientFor(&parent.Window)

	toolbarView := adw.NewToolbarView()
	toolbarView.AddTopBar(adw.NewHeaderBar())

	box := gtk.NewBox(gtk.OrientationVertical, 0)

	group := adw.NewPreferencesGroup()
	group.SetTitle("API Keys")
	group.SetMarginTop(16)
	group.SetMarginBottom(16)
	group.SetMarginStart(16)
	group.SetMarginEnd(16)

	steamEntry := adw.NewEntryRow()
	steamEntry.SetTitle("Steam Web API Key")
	steamEntry.SetText(cfg.SteamAPIKey)
	steamEntry.SetInputPurpose(gtk.InputPurposePassword)
	group.Add(steamEntry)

	sgdbEntry := adw.NewEntryRow()
	sgdbEntry.SetTitle("SteamGridDB API Key")
	sgdbEntry.SetText(cfg.SteamGridDBAPIKey)
	sgdbEntry.SetInputPurpose(gtk.InputPurposePassword)
	group.Add(sgdbEntry)

	notifGroup := adw.NewPreferencesGroup()
	notifGroup.SetTitle("Live Updates")
	notifGroup.SetMarginTop(16)
	notifGroup.SetMarginStart(16)
	notifGroup.SetMarginEnd(16)

	notifRow := adw.NewSwitchRow()
	notifRow.SetTitle("Notify on New Unlocks")
	notifRow.SetSubtitle("Show a desktop notification the moment an achievement unlocks")
	notifRow.SetActive(cfg.NotificationsEnabled)
	notifGroup.Add(notifRow)

	saveBtn := gtk.NewButton()
	saveBtn.SetLabel("Save")
	saveBtn.AddCSSClass("suggested-action")
	saveBtn.SetMarginTop(8)
	saveBtn.SetMarginBottom(16)
	saveBtn.SetMarginStart(16)
	saveBtn.SetMarginEnd(16)

	saveBtn.ConnectClicked(func() {
		cfg.SteamAPIKey = steamEntry.Text()
		cfg.SteamGridDBAPIKey = sgdbEntry.Text()
		cfg.NotificationsEnabled = notifRow.Active()

		steam.APIKey = cfg.SteamAPIKey
		steam.SteamGridDBAPIKey = cfg.SteamGridDBAPIKey

		if err := cfg.Save(); err != nil {
			fmt.Println("Failed to save config:", err)
		}
		dialog.Destroy()
	})

	box.Append(group)
	box.Append(notifGroup)
	box.Append(saveBtn)
	toolbarView.SetContent(box)
	dialog.SetContent(toolbarView)
	dialog.Show()
}

// switchToGame remembers the outgoing game's scroll offset (so coming back
// to it later resumes where you left off), then displays the requested
// game and restores its own remembered offset - or the top, if it hasn't
// been visited yet this session, so switching games never inherits
// whatever the previous game happened to be scrolled to.
func switchToGame(state *uiState, appID string) {
	if state.selectedID != "" && state.selectedID != appID {
		state.scrollPositions[state.selectedID] = state.contentScroll.VAdjustment().Value()
	}
	state.selectedID = appID

	for _, g := range state.games {
		if g.AppID != appID {
			continue
		}
		displayGame(g, state)

		target := state.scrollPositions[appID] // zero value if never visited
		glib.IdleAdd(func() {
			state.contentScroll.VAdjustment().SetValue(target)
		})
		return
	}
}

// displayGame clears the content area and renders a game's achievements.
func displayGame(game *Game, state *uiState) {
	contentBox := state.contentBox
	for child := contentBox.FirstChild(); child != nil; child = contentBox.FirstChild() {
		contentBox.Remove(child)
	}

	fraction := 0.0
	if game.TotalCount > 0 {
		fraction = float64(game.EarnedCount) / float64(game.TotalCount)
	}

	// ── Hero banner ──────────────────────────────────────────────────────────
	if game.HeroImagePath != "" {
		overlay := gtk.NewOverlay()

		hero := gtk.NewPicture()
		hero.SetContentFit(gtk.ContentFitCover)
		hero.SetCanShrink(true)
		setPictureAsync(hero, game.HeroImagePath)
		overlay.SetChild(hero)

		// Dark gradient overlay so text is readable
		gradient := gtk.NewBox(gtk.OrientationVertical, 0)
		gradient.SetVExpand(true)
		gradient.SetHExpand(true)
		gradient.AddCSSClass("hero-gradient")
		overlay.AddOverlay(gradient)

		// Game info on top of banner
		infoBox := gtk.NewBox(gtk.OrientationHorizontal, 14)
		infoBox.SetVAlign(gtk.AlignEnd)
		infoBox.SetMarginStart(24)
		infoBox.SetMarginEnd(24)
		infoBox.SetMarginBottom(24)

		if game.IconPath != "" {
			img := gtk.NewImageFromFile(game.IconPath)
			img.SetPixelSize(56)
			infoBox.Append(img)
		}

		titleVbox := gtk.NewBox(gtk.OrientationVertical, 4)
		titleVbox.SetVAlign(gtk.AlignCenter)

		titleLabel := gtk.NewLabel(game.Name)
		titleLabel.SetXAlign(0)
		titleLabel.AddCSSClass("title-1")
		titleVbox.Append(titleLabel)

		subLabel := gtk.NewLabel(fmt.Sprintf(
			"%d of %d achievements  ·  %.0f%% complete",
			game.EarnedCount, game.TotalCount, fraction*100,
		))
		subLabel.SetXAlign(0)
		subLabel.AddCSSClass("dim-label")
		titleVbox.Append(subLabel)

		infoBox.Append(titleVbox)
		overlay.AddOverlay(infoBox)

		// Translucent progress bar overlayed directly on the hero bottom
		progress := gtk.NewProgressBar()
		progress.SetFraction(fraction)
		progress.SetVAlign(gtk.AlignEnd)
		progress.AddCSSClass("hero-progress")
		overlay.AddOverlay(progress)

		// Pin the hero to a fixed height. A plain container's measured size
		// always incorporates its children's natural size, so a Box (even
		// with an explicit size request, which only acts as a *minimum*)
		// still grows to fit the Picture's aspect-ratio-driven natural
		// height — which varies per game and can be far taller than 280px
		// for portrait-ish hero art, especially once there isn't much
		// achievement content below to fill out the rest of the window.
		//
		// GtkScrolledWindow is different: by default it does NOT propagate
		// its child's natural size upward (propagate-natural-height is
		// false), so giving it an explicit size request results in exactly
		// that size no matter how large the child wants to be — the extra
		// content is simply clipped. Scrollbars are disabled since we only
		// want the clipping behavior, not actual scrolling.
		overlay.SetVExpand(false)
		overlay.SetHExpand(true)
		gradient.SetVExpand(false)
		heroWrap := gtk.NewScrolledWindow()
		heroWrap.SetPolicy(gtk.PolicyNever, gtk.PolicyNever)
		heroWrap.SetPropagateNaturalHeight(false)
		heroWrap.SetPropagateNaturalWidth(false)
		heroWrap.SetSizeRequest(-1, 280)
		heroWrap.SetVExpand(false)
		heroWrap.SetChild(overlay)
		contentBox.Append(heroWrap)

	} else {
		// Fallback: plain header without hero
		headerBox := gtk.NewBox(gtk.OrientationVertical, 12)
		headerBox.SetMarginTop(28)
		headerBox.SetMarginBottom(8)
		headerBox.SetMarginStart(28)
		headerBox.SetMarginEnd(28)

		titleRow := gtk.NewBox(gtk.OrientationHorizontal, 14)
		if game.IconPath != "" {
			img := gtk.NewImageFromFile(game.IconPath)
			img.SetPixelSize(56)
			titleRow.Append(img)
		}

		titleVbox := gtk.NewBox(gtk.OrientationVertical, 4)
		titleVbox.SetVAlign(gtk.AlignCenter)

		titleLabel := gtk.NewLabel(game.Name)
		titleLabel.SetXAlign(0)
		titleLabel.AddCSSClass("title-1")
		titleVbox.Append(titleLabel)

		subLabel := gtk.NewLabel(fmt.Sprintf(
			"%d of %d achievements  ·  %.0f%% complete",
			game.EarnedCount, game.TotalCount, fraction*100,
		))
		subLabel.SetXAlign(0)
		subLabel.AddCSSClass("dim-label")
		titleVbox.Append(subLabel)

		titleRow.Append(titleVbox)
		headerBox.Append(titleRow)

		progress := gtk.NewProgressBar()
		progress.SetFraction(fraction)
		progress.SetMarginTop(4)
		headerBox.Append(progress)

		contentBox.Append(headerBox)
	}

	// Spacing instead of horizontal line
	spacer := gtk.NewBox(gtk.OrientationVertical, 0)
	spacer.SetMarginTop(12)
	contentBox.Append(spacer)

	// ── Centered / Clamped Content VBox ──────────────────────────────────────
	gameVBox := gtk.NewBox(gtk.OrientationVertical, 0)
	gameVBox.SetMarginStart(16)
	gameVBox.SetMarginEnd(16)

	// Tab View Stack & Switcher
	viewStack := adw.NewViewStack()

	viewSwitcher := adw.NewViewSwitcher()
	viewSwitcher.SetStack(viewStack)
	viewSwitcher.SetHAlign(gtk.AlignCenter)
	viewSwitcher.SetMarginTop(12)
	viewSwitcher.SetMarginBottom(12)
	gameVBox.Append(viewSwitcher)

	// Spacing instead of horizontal line
	switcherSpacer := gtk.NewBox(gtk.OrientationVertical, 0)
	switcherSpacer.SetMarginBottom(12)
	gameVBox.Append(switcherSpacer)

	// Tab 1: My Progress
	progressVBox := gtk.NewBox(gtk.OrientationVertical, 16)

	// Tab 2: Global Stats
	globalVBox := gtk.NewBox(gtk.OrientationVertical, 16)

	// ── Categorize achievements ──────────────────────────────────────────────
	var earned, locked, hidden []MergedAchievement
	for _, ach := range game.Achievements {
		if ach.Earned {
			earned = append(earned, ach)
		} else if ach.Hidden {
			hidden = append(hidden, ach)
		} else {
			locked = append(locked, ach)
		}
	}

	// Sorts
	sort.Slice(earned, func(i, j int) bool {
		return earned[i].EarnedTime > earned[j].EarnedTime
	})
	sort.Slice(locked, func(i, j int) bool {
		return locked[i].DisplayName < locked[j].DisplayName
	})

	gameDir := filepath.Join(saveDir, game.AppID)
	reload := func() {
		if updated, err := loadGame(game.AppID, gameDir); err == nil {
			applyGameUpdate(state, updated)
		}
	}

	// ── Populate Tab 1: My Progress ──────────────────────────────────────────
	// Only the first eagerImageBudget icons across this tab are decoded right
	// away (comfortably covering anything visible without scrolling); the
	// rest are trickled in afterwards so opening a game with hundreds of
	// achievements doesn't stall the UI thread decoding images nobody can
	// see yet.
	nextProgressLoader, flushProgressLoader := newImageLoaderBudget(eagerImageBudget)

	if len(earned) > 0 {
		earnedGroup := adw.NewPreferencesGroup()
		earnedGroup.SetTitle(fmt.Sprintf("Earned  ·  %d", len(earned)))
		for _, ach := range earned {
			earnedGroup.Add(createAchievementRow(ach, nil, nextProgressLoader()))
		}
		progressVBox.Append(earnedGroup)
	}

	if len(locked) > 0 || len(hidden) > 0 {
		lockedGroup := adw.NewPreferencesGroup()
		lockedGroup.SetTitle(fmt.Sprintf("Locked  ·  %d", len(locked)+len(hidden)))
		for _, ach := range locked {
			a := ach
			lockedGroup.Add(createAchievementRow(a, func() {
				confirmMarkUnlocked(state, gameDir, a, reload)
			}, nextProgressLoader()))
		}
		if len(hidden) > 0 {
			hiddenRow := adw.NewActionRow()
			hiddenRow.SetTitle(fmt.Sprintf("... and %d hidden achievements", len(hidden)))
			hiddenRow.SetSubtitle("Earn them to reveal details")
			hiddenRow.SetSensitive(false)
			lockedGroup.Add(hiddenRow)
		}
		progressVBox.Append(lockedGroup)
	}
	flushProgressLoader()

	// ── Populate Tab 2: Global Stats ──────────────────────────────────────────
	// This tab is expensive to build for games with hundreds of achievements
	// and, unlike "My Progress", most users never open it. So its rows aren't
	// built at all until the user actually switches to it.
	globalBuilt := false
	buildGlobalTab := func() {
		if globalBuilt {
			return
		}
		globalBuilt = true

		allAch := append([]MergedAchievement{}, game.Achievements...)
		sort.Slice(allAch, func(i, j int) bool {
			return allAch[i].GlobalPercent > allAch[j].GlobalPercent
		})

		globalGroup := adw.NewPreferencesGroup()
		globalGroup.SetTitle("Global Unlock Rates")
		globalGroup.SetMarginBottom(24)

		nextGlobalLoader, flushGlobalLoader := newImageLoaderBudget(eagerImageBudget)
		for _, ach := range allAch {
			row, reveal := createGlobalStatsRow(ach, nextGlobalLoader())
			globalGroup.Add(row)
			if reveal != nil {
				click := gtk.NewGestureClick()
				click.ConnectPressed(func(nPress int, x, y float64) {
					reveal()
				})
				row.AddController(click)
			}
		}
		flushGlobalLoader()

		globalVBox.Append(globalGroup)
	}

	// Add pages to ViewStack
	progressPage := viewStack.AddTitled(progressVBox, "progress", "My Progress")
	progressPage.SetIconName("user-home-symbolic")

	globalPage := viewStack.AddTitled(globalVBox, "global", "Global Stats")
	globalPage.SetIconName("dialog-information-symbolic")

	viewStack.NotifyProperty("visible-child-name", func() {
		if viewStack.VisibleChildName() == "global" {
			buildGlobalTab()
		}
	})

	// Disable homogeneous height so tabs size independently
	viewStack.SetVhomogeneous(false)
	viewStack.SetMarginBottom(32)

	gameVBox.Append(viewStack)

	// Restrict content width to 860px max and center it
	clamp := adw.NewClamp()
	clamp.SetMaximumSize(860)
	clamp.SetTighteningThreshold(860)
	clamp.SetMarginStart(16)
	clamp.SetMarginEnd(16)
	clamp.SetChild(gameVBox)

	contentBox.Append(clamp)
}

// confirmMarkUnlocked shows a deliberately-unmissable confirmation dialog
// before writing a manual unlock, since this is meant to be hard to trigger
// by accident. The unlock time is left at zero so it's obvious in the UI
// this wasn't a real, timestamped unlock.
func confirmMarkUnlocked(state *uiState, gameDir string, ach MergedAchievement, reload func()) {
	dialog := adw.NewMessageDialog(&state.window.Window,
		"Mark as Already Unlocked?",
		fmt.Sprintf(
			"This will mark “%s” as earned without a real unlock time. "+
				"Use this only if you already unlocked it previously (e.g. before using this tool).",
			ach.DisplayName,
		),
	)
	dialog.AddResponse("cancel", "Cancel")
	dialog.AddResponse("confirm", "Mark as Unlocked")
	dialog.SetResponseAppearance("confirm", adw.ResponseDestructive)
	dialog.SetDefaultResponse("cancel")
	dialog.SetCloseResponse("cancel")

	dialog.ConnectResponse(func(response string) {
		if response != "confirm" {
			return
		}
		if err := SetAchievementEarned(gameDir, ach.Name, true); err != nil {
			fmt.Println("Failed to mark achievement as unlocked:", err)
			return
		}
		reload()
	})

	dialog.Show()
}

// createGlobalStatsRow builds a list row with a native Adwaita progress bar background using Grid overlapping.
func createGlobalStatsRow(ach MergedAchievement, loadImg imageLoader) (*gtk.ListBoxRow, func()) {
	row := gtk.NewListBoxRow()
	row.SetSelectable(false)

	grid := gtk.NewGrid()

	// Progress bar in the background (draws first)
	progress := gtk.NewProgressBar()
	progress.SetFraction(ach.GlobalPercent / 100.0)
	progress.SetVAlign(gtk.AlignFill)
	progress.SetHAlign(gtk.AlignFill)
	progress.SetHExpand(true)
	progress.SetVExpand(true)
	progress.SetOpacity(0.18)

	barProvider := gtk.NewCSSProvider()
	barProvider.LoadFromString(`
		trough {
			background-color: transparent;
			border: none;
		}
		progress {
			border: none;
			border-radius: 0;
		}
	`)
	progress.StyleContext().AddProvider(barProvider, gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

	isHiddenSpoiler := ach.Hidden && !ach.Earned

	// Build icon — hidden spoiler starts with placeholder, revealed on click
	img := gtk.NewImageFromIconName("changes-prevent-symbolic")
	img.SetPixelSize(48)
	img.SetVAlign(gtk.AlignStart)
	if isHiddenSpoiler && ach.IconGrayPath == "" {
		// keep placeholder
	} else if ach.Earned {
		if ach.IconPath != "" {
			loadImg(img, ach.IconPath)
		} else {
			img.SetFromIconName("starred-symbolic")
		}
	} else {
		if ach.IconGrayPath != "" {
			loadImg(img, ach.IconGrayPath)
		}
	}

	// Single content box — icon swaps on reveal, only text animates
	content := gtk.NewBox(gtk.OrientationHorizontal, 12)
	content.SetMarginTop(8)
	content.SetMarginBottom(8)
	content.SetMarginStart(12)
	content.SetMarginEnd(12)
	content.Append(img)

	// Animated text Stack (slides left on reveal)
	textStack := gtk.NewStack()
	textStack.SetHExpand(true)
	textStack.SetVAlign(gtk.AlignStart)
	textStack.SetTransitionType(gtk.StackTransitionTypeSlideLeft)
	textStack.SetTransitionDuration(350)

	vboxSpoiler := gtk.NewBox(gtk.OrientationVertical, 2)
	vboxSpoiler.SetVAlign(gtk.AlignStart)
	titleSpoiler := gtk.NewLabel("Hidden Achievement")
	titleSpoiler.SetXAlign(0)
	vboxSpoiler.Append(titleSpoiler)
	descSpoiler := gtk.NewLabel("Click to reveal spoiler")
	descSpoiler.SetXAlign(0)
	descSpoiler.AddCSSClass("dim-label")
	descSpoiler.AddCSSClass("caption")
	vboxSpoiler.Append(descSpoiler)
	textStack.AddNamed(vboxSpoiler, "spoiler")

	vboxReal := gtk.NewBox(gtk.OrientationVertical, 2)
	vboxReal.SetVAlign(gtk.AlignStart)
	titleReal := gtk.NewLabel(ach.DisplayName)
	titleReal.SetXAlign(0)
	vboxReal.Append(titleReal)
	descReal := gtk.NewLabel(ach.Description)
	descReal.SetWrap(true)
	descReal.SetWrapMode(pango.WrapWordChar)
	descReal.SetXAlign(0)
	descReal.AddCSSClass("dim-label")
	descReal.AddCSSClass("caption")
	vboxReal.Append(descReal)
	textStack.AddNamed(vboxReal, "real")

	content.Append(textStack)

	pct := gtk.NewLabel(fmt.Sprintf("%.1f%%", ach.GlobalPercent))
	pct.SetVAlign(gtk.AlignStart)
	pct.AddCSSClass("heading")
	content.Append(pct)

	grid.Attach(progress, 0, 0, 1, 1)
	grid.Attach(content, 0, 0, 1, 1)
	row.SetChild(grid)

	var reveal func()

	if isHiddenSpoiler {
		textStack.SetVisibleChildName("spoiler")
		row.SetSelectable(true)
		row.SetActivatable(true)

		revealed := false
		reveal = func() {
			if revealed {
				return
			}
			revealed = true
			textStack.SetVisibleChildName("real")
			// Swap to actual icon with grayscale filter (locked but revealed).
			// This is a direct user interaction (a click), so it always loads
			// immediately rather than going through the deferred loader.
			if ach.IconPath != "" {
				setImageAsync(img, ach.IconPath)
				grayProvider := gtk.NewCSSProvider()
				grayProvider.LoadFromString("image { filter: grayscale(100%); }")
				img.StyleContext().AddProvider(grayProvider, gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
			} else if ach.IconGrayPath != "" {
				setImageAsync(img, ach.IconGrayPath)
			}
			row.SetActivatable(false)
			row.SetSelectable(false)
		}
	} else {
		textStack.SetVisibleChildName("real")
	}

	return row, reveal
}

// createAchievementRow builds a standard achievement row. If onMarkUnlocked is
// non-nil, right-clicking the row offers a (confirmation-gated) way to
// manually mark it as already earned. loadImg controls whether this row's
// icon is decoded immediately or deferred (see imageLoader).
func createAchievementRow(ach MergedAchievement, onMarkUnlocked func(), loadImg imageLoader) *gtk.ListBoxRow {
	row := gtk.NewListBoxRow()
	row.SetSelectable(false)

	content := gtk.NewBox(gtk.OrientationHorizontal, 12)
	content.SetMarginTop(8)
	content.SetMarginBottom(8)
	content.SetMarginStart(12)
	content.SetMarginEnd(12)

	img := gtk.NewImageFromIconName("changes-prevent-symbolic")
	img.SetPixelSize(48)
	img.SetVAlign(gtk.AlignStart)
	if ach.Earned {
		if ach.IconPath != "" {
			loadImg(img, ach.IconPath)
		} else {
			img.SetFromIconName("starred-symbolic")
		}
	} else if ach.IconGrayPath != "" {
		loadImg(img, ach.IconGrayPath)
	}
	content.Append(img)

	// title stays pinned to the top of its own column, independent from how
	// many lines the description below it wraps to.
	vbox := gtk.NewBox(gtk.OrientationVertical, 2)
	vbox.SetVAlign(gtk.AlignStart)
	vbox.SetHExpand(true)

	title := gtk.NewLabel(ach.DisplayName)
	title.SetXAlign(0)
	title.SetVAlign(gtk.AlignStart)
	vbox.Append(title)

	desc := gtk.NewLabel(ach.Description)
	desc.SetXAlign(0)
	desc.SetVAlign(gtk.AlignStart)
	desc.SetWrap(true)
	desc.SetWrapMode(pango.WrapWordChar)
	desc.AddCSSClass("dim-label")
	desc.AddCSSClass("caption")
	vbox.Append(desc)

	content.Append(vbox)

	// In regular mode, show unlock timestamp if earned
	if ach.Earned {
		timeLabel := gtk.NewLabel("")
		timeLabel.SetJustify(gtk.JustifyRight)
		timeLabel.SetVAlign(gtk.AlignStart)
		timeLabel.AddCSSClass("dim-label")
		timeLabel.AddCSSClass("caption")
		if ach.EarnedTime > 0 {
			t := time.Unix(ach.EarnedTime, 0)
			timeLabel.SetText(t.Format("Jan 2, 2006 @ 3:04 PM"))
		} else {
			timeLabel.SetText("Marked manually")
		}
		content.Append(timeLabel)
	}

	row.SetChild(content)

	if onMarkUnlocked != nil {
		click := gtk.NewGestureClick()
		click.SetButton(3) // right-click only, so this can't be triggered by accident
		click.ConnectPressed(func(nPress int, x, y float64) {
			onMarkUnlocked()
		})
		row.AddController(click)
		row.SetTooltipText("Right-click to mark as already unlocked")
	}

	return row
}
