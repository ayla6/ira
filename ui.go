package main

import (
	"fmt"
	"sort"
	"time"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
)

// buildUI constructs the main window layout.
func buildUI(window *adw.ApplicationWindow, games []Game, cfg *Config, steam *SteamClient) {
	splitView := gtk.NewPaned(gtk.OrientationHorizontal)
	splitView.SetPosition(260)
	splitView.SetShrinkStartChild(false)
	splitView.SetResizeStartChild(false)

	// ── Sidebar ──────────────────────────────────────────────────────────────
	sidebarScroll := gtk.NewScrolledWindow()
	sidebarScroll.SetPolicy(gtk.PolicyAutomatic, gtk.PolicyAutomatic)
	sidebarScroll.SetSizeRequest(200, -1)

	gameList := gtk.NewListBox()
	gameList.AddCSSClass("navigation-sidebar")
	sidebarScroll.SetChild(gameList)
	splitView.SetStartChild(sidebarScroll)

	// ── Content area ─────────────────────────────────────────────────────────
	contentScroll := gtk.NewScrolledWindow()
	contentScroll.SetPolicy(gtk.PolicyAutomatic, gtk.PolicyAutomatic)
	contentBox := gtk.NewBox(gtk.OrientationVertical, 0)
	contentScroll.SetChild(contentBox)
	splitView.SetEndChild(contentScroll)

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

	toolbarView := adw.NewToolbarView()
	toolbarView.AddTopBar(headerBar)
	toolbarView.SetContent(splitView)
	window.SetContent(toolbarView)

	// ── Empty state ───────────────────────────────────────────────────────────
	emptyState := adw.NewStatusPage()
	emptyState.SetTitle("No Game Selected")
	emptyState.SetDescription("Select a game from the sidebar to view achievements.")
	emptyState.SetIconName("trophy-symbolic")
	contentBox.Append(emptyState)

	// ── Sidebar game rows ─────────────────────────────────────────────────────
	for _, g := range games {
		game := g
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
		titleLabel.SetMaxWidthChars(20)
		vbox.Append(titleLabel)

		pct := 0
		if game.TotalCount > 0 {
			pct = game.EarnedCount * 100 / game.TotalCount
		}
		countLabel := gtk.NewLabel(fmt.Sprintf("%d/%d · %d%%", game.EarnedCount, game.TotalCount, pct))
		countLabel.SetXAlign(0)
		countLabel.AddCSSClass("dim-label")
		countLabel.AddCSSClass("caption")
		vbox.Append(countLabel)

		hbox.Append(vbox)
		row.SetChild(hbox)
		gameList.Append(row)
	}

	gameList.ConnectRowSelected(func(row *gtk.ListBoxRow) {
		if row == nil {
			return
		}
		idx := row.Index()
		if idx >= 0 && idx < len(games) {
			displayGame(games[idx], contentBox, emptyState)
		}
	})
}

// showSettingsDialog opens a modal window for editing the Steam and SteamGridDB API keys.
func showSettingsDialog(parent *adw.ApplicationWindow, cfg *Config, steam *SteamClient) {
	dialog := adw.NewWindow()
	dialog.SetTitle("Settings")
	dialog.SetDefaultSize(450, 260)
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
		
		steam.APIKey = cfg.SteamAPIKey
		steam.SteamGridDBAPIKey = cfg.SteamGridDBAPIKey
		
		if err := cfg.Save(); err != nil {
			fmt.Println("Failed to save config:", err)
		}
		dialog.Destroy()
	})

	box.Append(group)
	toolbarView.SetContent(box)
	dialog.SetContent(toolbarView)
	dialog.Show()
}

// displayGame clears the content area and renders a game's achievements.
func displayGame(game Game, contentBox *gtk.Box, emptyState *adw.StatusPage) {
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
		hero.SetFilename(game.HeroImagePath)
		hero.SetContentFit(gtk.ContentFitCover)
		hero.SetCanShrink(true)
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

		overlay.SetSizeRequest(-1, 280)
		contentBox.Append(overlay)

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

	// ── Populate Tab 1: My Progress ──────────────────────────────────────────
	if len(earned) > 0 {
		earnedGroup := adw.NewPreferencesGroup()
		earnedGroup.SetTitle(fmt.Sprintf("Earned  ·  %d", len(earned)))
		for _, ach := range earned {
			earnedGroup.Add(createAchievementRow(ach))
		}
		progressVBox.Append(earnedGroup)
	}

	if len(locked) > 0 || len(hidden) > 0 {
		lockedGroup := adw.NewPreferencesGroup()
		lockedGroup.SetTitle(fmt.Sprintf("Locked  ·  %d", len(locked)+len(hidden)))
		for _, ach := range locked {
			lockedGroup.Add(createAchievementRow(ach))
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

	// ── Populate Tab 2: Global Stats ──────────────────────────────────────────
	allAch := append([]MergedAchievement{}, game.Achievements...)
	sort.Slice(allAch, func(i, j int) bool {
		return allAch[i].GlobalPercent > allAch[j].GlobalPercent
	})

	globalGroup := adw.NewPreferencesGroup()
	globalGroup.SetTitle("Global Unlock Rates")
	globalGroup.SetMarginBottom(24)

	for _, ach := range allAch {
		row, reveal := createGlobalStatsRow(ach)
		globalGroup.Add(row)
		if reveal != nil {
			row.Connect("activate", func() {
				reveal()
			})
		}
	}

	globalVBox.Append(globalGroup)

	// Add pages to ViewStack
	progressPage := viewStack.AddTitled(progressVBox, "progress", "My Progress")
	progressPage.SetIconName("user-home-symbolic")

	globalPage := viewStack.AddTitled(globalVBox, "global", "Global Stats")
	globalPage.SetIconName("dialog-information-symbolic")

	gameVBox.Append(viewStack)

	// Restrict content width to 860px max and center it
	clamp := adw.NewClamp()
	clamp.SetMaximumSize(860)
	clamp.SetMarginBottom(32)
	clamp.SetChild(gameVBox)

	contentBox.Append(clamp)
}

// createGlobalStatsRow builds a list row with a native Adwaita progress bar background using Grid overlapping.
func createGlobalStatsRow(ach MergedAchievement) (*gtk.ListBoxRow, func()) {
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
	progress.SetOpacity(0.18) // Native Adwaita accent color, translucent!

	provider := gtk.NewCSSProvider()
	provider.LoadFromString(`
		trough {
			background-color: transparent;
			border: none;
		}
		progress {
			border: none;
			border-radius: 0;
		}
	`)
	progress.StyleContext().AddProvider(provider, gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

	// Content in the foreground (draws second, on top)
	content := gtk.NewBox(gtk.OrientationHorizontal, 12)
	// Match ActionRow padding EXACTLY so heights are identical
	content.SetMarginTop(8)
	content.SetMarginBottom(8)
	content.SetMarginStart(12)
	content.SetMarginEnd(12)
	// Icon
	var img *gtk.Image
	if ach.Hidden && !ach.Earned {
		if ach.IconGrayPath != "" {
			img = gtk.NewImageFromFile(ach.IconGrayPath)
		} else {
			img = gtk.NewImageFromIconName("dialog-question")
		}
	} else {
		if ach.IconPath != "" {
			img = gtk.NewImageFromFile(ach.IconPath)
		} else {
			img = gtk.NewImageFromIconName("dialog-question")
		}
	}
	img.SetPixelSize(48)
	content.Append(img)

	// Text VBox
	vbox := gtk.NewBox(gtk.OrientationVertical, 2)
	vbox.SetVAlign(gtk.AlignCenter)
	vbox.SetHExpand(true)

	title := gtk.NewLabel("")
	title.SetXAlign(0)
	vbox.Append(title)

	desc := gtk.NewLabel("")
	desc.SetXAlign(0)
	desc.AddCSSClass("dim-label")
	desc.AddCSSClass("caption")
	vbox.Append(desc)

	content.Append(vbox)

	// Percentage
	pct := gtk.NewLabel(fmt.Sprintf("%.1f%%", ach.GlobalPercent))
	pct.SetVAlign(gtk.AlignCenter)
	pct.AddCSSClass("heading")
	content.Append(pct)

	// Attach to grid: both in cell (0,0). Progress added first (behind).
	grid.Attach(progress, 0, 0, 1, 1)
	grid.Attach(content, 0, 0, 1, 1)

	row.SetChild(grid)
	var reveal func()

	// Handle hidden vs normal display
	if ach.Hidden && !ach.Earned {
		title.SetLabel("Hidden Achievement")
		desc.SetLabel("Click to reveal spoiler")
		
		row.SetSelectable(true)
		row.SetActivatable(true)
		
		revealed := false
		reveal = func() {
			if revealed {
				return
			}
			revealed = true
			title.SetLabel(ach.DisplayName)
			desc.SetLabel(ach.Description)
			if ach.IconPath != "" {
				img.SetFromFile(ach.IconPath)
			}
			row.SetActivatable(false)
			row.SetSelectable(false)
		}
	} else {
		title.SetLabel(ach.DisplayName)
		desc.SetLabel(ach.Description)
	}

	return row, reveal
}

// createAchievementRow builds a standard achievement row.
func createAchievementRow(ach MergedAchievement) *gtk.ListBoxRow {
	row := gtk.NewListBoxRow()
	row.SetSelectable(false)

	content := gtk.NewBox(gtk.OrientationHorizontal, 12)
	content.SetMarginTop(8)
	content.SetMarginBottom(8)
	content.SetMarginStart(12)
	content.SetMarginEnd(12)

	var img *gtk.Image
	if ach.Earned && ach.IconPath != "" {
		img = gtk.NewImageFromFile(ach.IconPath)
	} else if !ach.Earned && ach.IconGrayPath != "" {
		img = gtk.NewImageFromFile(ach.IconGrayPath)
	} else {
		img = gtk.NewImageFromIconName("dialog-question")
	}
	img.SetPixelSize(48)
	content.Append(img)

	vbox := gtk.NewBox(gtk.OrientationVertical, 2)
	vbox.SetVAlign(gtk.AlignCenter)
	vbox.SetHExpand(true)

	title := gtk.NewLabel(ach.DisplayName)
	title.SetXAlign(0)
	vbox.Append(title)

	desc := gtk.NewLabel(ach.Description)
	desc.SetXAlign(0)
	desc.AddCSSClass("dim-label")
	desc.AddCSSClass("caption")
	vbox.Append(desc)

	content.Append(vbox)

	// In regular mode, show unlock timestamp if earned
	if ach.Earned && ach.EarnedTime > 0 {
		t := time.Unix(ach.EarnedTime, 0)
		timeLabel := gtk.NewLabel(t.Format("Jan 2, 2006 @ 3:04 PM"))
		timeLabel.SetJustify(gtk.JustifyRight)
		timeLabel.SetVAlign(gtk.AlignCenter)
		timeLabel.AddCSSClass("dim-label")
		timeLabel.AddCSSClass("caption")
		content.Append(timeLabel)
	}

	row.SetChild(content)
	return row
}

// createGlobalHiddenRow creates a clickable, spoiler-protected row for global stats.
func createGlobalHiddenRow(ach MergedAchievement) *adw.ActionRow {
	row := adw.NewActionRow()
	row.AddCSSClass("global-row-actionrow")
	revealed := false

	row.SetTitle("Hidden Achievement")
	row.SetSubtitle("Click to reveal spoiler")
	row.SetSubtitleLines(2)
	row.SetTitleLines(1)
	row.SetActivatable(true)

	var img *gtk.Image
	if ach.IconGrayPath != "" {
		img = gtk.NewImageFromFile(ach.IconGrayPath)
	} else {
		img = gtk.NewImageFromIconName("dialog-question")
	}
	img.SetPixelSize(48)
	img.SetMarginTop(8)
	img.SetMarginBottom(8)
	img.SetMarginStart(4)
	img.SetMarginEnd(4)
	row.AddPrefix(img)

	pctLabel := gtk.NewLabel(fmt.Sprintf("%.1f%%", ach.GlobalPercent))
	pctLabel.AddCSSClass("dim-label")
	pctLabel.AddCSSClass("heading")
	pctLabel.SetMarginEnd(8)
	row.AddSuffix(pctLabel)

	row.ConnectActivated(func() {
		if revealed {
			return
		}
		revealed = true
		row.SetTitle(ach.DisplayName)
		row.SetSubtitle(ach.Description)
		row.SetActivatable(false)
	})

	return row
}
