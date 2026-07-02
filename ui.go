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
	splitView.SetPosition(220)
	splitView.SetShrinkStartChild(false)
	splitView.SetResizeStartChild(false)

	// ── Sidebar ──────────────────────────────────────────────────────────────
	sidebarScroll := gtk.NewScrolledWindow()
	sidebarScroll.SetPolicy(gtk.PolicyAutomatic, gtk.PolicyAutomatic)
	sidebarScroll.SetSizeRequest(160, -1)

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
		titleLabel.SetMaxWidthChars(18)
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

// showSettingsDialog opens a modal window for editing the Steam API key.
func showSettingsDialog(parent *adw.ApplicationWindow, cfg *Config, steam *SteamClient) {
	dialog := gtk.NewWindow()
	dialog.SetTitle("Settings")
	dialog.SetDefaultSize(420, 180)
	dialog.SetModal(true)
	dialog.SetTransientFor(&parent.Window)
	dialog.SetDestroyWithParent(true)

	box := gtk.NewBox(gtk.OrientationVertical, 0)
	hbar := adw.NewHeaderBar()
	box.Append(hbar)

	group := adw.NewPreferencesGroup()
	group.SetTitle("Steam")
	group.SetMarginTop(16)
	group.SetMarginBottom(16)
	group.SetMarginStart(16)
	group.SetMarginEnd(16)

	entry := adw.NewEntryRow()
	entry.SetTitle("Steam Web API Key")
	entry.SetText(cfg.SteamAPIKey)
	entry.SetInputPurpose(gtk.InputPurposePassword)
	group.Add(entry)

	saveBtn := gtk.NewButton()
	saveBtn.SetLabel("Save")
	saveBtn.AddCSSClass("suggested-action")
	saveBtn.SetMarginTop(8)
	saveBtn.SetMarginBottom(16)
	saveBtn.SetMarginStart(16)
	saveBtn.SetMarginEnd(16)

	saveBtn.ConnectClicked(func() {
		cfg.SteamAPIKey = entry.Text()
		steam.APIKey = cfg.SteamAPIKey
		if err := cfg.Save(); err != nil {
			fmt.Println("Failed to save config:", err)
		}
		dialog.Destroy()
	})

	box.Append(group)
	box.Append(saveBtn)
	dialog.SetChild(box)
	dialog.Show()
}

// displayGame clears the content area and renders a game's achievements.
func displayGame(game Game, contentBox *gtk.Box, emptyState *adw.StatusPage) {
	for child := contentBox.FirstChild(); child != nil; child = contentBox.FirstChild() {
		contentBox.Remove(child)
	}

	// ── Hero banner ──────────────────────────────────────────────────────────
	if game.HeroImagePath != "" {
		overlay := gtk.NewOverlay()
		overlay.SetSizeRequest(-1, 200)

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
		infoBox.SetMarginBottom(16)

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

		fraction := 0.0
		if game.TotalCount > 0 {
			fraction = float64(game.EarnedCount) / float64(game.TotalCount)
		}
		subLabel := gtk.NewLabel(fmt.Sprintf(
			"%d of %d achievements  ·  %.0f%% complete",
			game.EarnedCount, game.TotalCount, fraction*100,
		))
		subLabel.SetXAlign(0)
		subLabel.AddCSSClass("dim-label")
		titleVbox.Append(subLabel)

		infoBox.Append(titleVbox)
		overlay.AddOverlay(infoBox)

		contentBox.Append(overlay)

		// Progress bar below banner
		progress := gtk.NewProgressBar()
		progress.SetFraction(fraction)
		progress.SetMarginStart(24)
		progress.SetMarginEnd(24)
		progress.SetMarginTop(12)
		progress.SetMarginBottom(4)
		contentBox.Append(progress)

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

		fraction := 0.0
		if game.TotalCount > 0 {
			fraction = float64(game.EarnedCount) / float64(game.TotalCount)
		}
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

	sep := gtk.NewSeparator(gtk.OrientationHorizontal)
	sep.SetMarginTop(12)
	contentBox.Append(sep)

	// ── Categorize ────────────────────────────────────────────────────────────
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

	sort.Slice(earned, func(i, j int) bool {
		return earned[i].EarnedTime > earned[j].EarnedTime
	})
	sort.Slice(locked, func(i, j int) bool {
		return locked[i].DisplayName < locked[j].DisplayName
	})

	// ── Sections ─────────────────────────────────────────────────────────────
	if len(earned) > 0 {
		appendEarnedSection(contentBox, earned)
	}
	appendLockedSection(contentBox, locked, hidden)
}

func appendEarnedSection(contentBox *gtk.Box, achievements []MergedAchievement) {
	group := adw.NewPreferencesGroup()
	group.SetTitle(fmt.Sprintf("Unlocked  ·  %d", len(achievements)))
	group.SetMarginStart(24)
	group.SetMarginEnd(24)
	group.SetMarginTop(16)
	group.SetMarginBottom(8)
	for _, ach := range achievements {
		group.Add(createAchievementRow(ach))
	}
	contentBox.Append(group)
}

func appendLockedSection(contentBox *gtk.Box, locked, hidden []MergedAchievement) {
	total := len(locked) + len(hidden)
	if total == 0 {
		return
	}

	group := adw.NewPreferencesGroup()
	group.SetTitle(fmt.Sprintf("Locked  ·  %d", total))
	group.SetMarginStart(24)
	group.SetMarginEnd(24)
	group.SetMarginTop(16)
	group.SetMarginBottom(24)

	for _, ach := range locked {
		group.Add(createAchievementRow(ach))
	}

	// Hidden achievements — shown inline, spoiler-protected per row
	for _, ach := range hidden {
		group.Add(createHiddenAchievementRow(ach))
	}

	contentBox.Append(group)
}

// createAchievementRow builds a standard (non-hidden) achievement row.
func createAchievementRow(ach MergedAchievement) *adw.ActionRow {
	row := adw.NewActionRow()
	row.SetTitle(ach.DisplayName)
	row.SetSubtitle(ach.Description)
	row.SetSubtitleLines(2)
	row.SetTitleLines(1)

	var img *gtk.Image
	if ach.Earned && ach.IconPath != "" {
		img = gtk.NewImageFromFile(ach.IconPath)
	} else if !ach.Earned && ach.IconGrayPath != "" {
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

	// Global unlock % suffix
	if ach.GlobalPercent > 0 {
		pctLabel := gtk.NewLabel(fmt.Sprintf("%.1f%%\nglobal", ach.GlobalPercent))
		pctLabel.SetJustify(gtk.JustifyRight)
		pctLabel.AddCSSClass("dim-label")
		pctLabel.AddCSSClass("caption")
		pctLabel.SetMarginEnd(8)
		row.AddSuffix(pctLabel)
	}

	// Unlock timestamp for earned achievements
	if ach.Earned && ach.EarnedTime > 0 {
		t := time.Unix(ach.EarnedTime, 0)
		timeLabel := gtk.NewLabel(t.Format("Jan 2, 2006") + "\n" + t.Format("3:04 PM"))
		timeLabel.SetJustify(gtk.JustifyRight)
		timeLabel.AddCSSClass("dim-label")
		timeLabel.AddCSSClass("caption")
		timeLabel.SetMarginEnd(8)
		row.AddSuffix(timeLabel)
	}

	return row
}

// createHiddenAchievementRow builds a spoiler-protected row for hidden achievements.
// The row shows "Hidden Achievement" with the global %; clicking it reveals the real content.
func createHiddenAchievementRow(ach MergedAchievement) *adw.ActionRow {
	row := adw.NewActionRow()
	revealed := false

	subtitle := "Hidden achievement"
	if ach.GlobalPercent > 0 {
		subtitle = fmt.Sprintf("%.1f%% of players have unlocked this  ·  Click to reveal spoiler", ach.GlobalPercent)
	} else {
		subtitle = "Unknown unlock rate  ·  Click to reveal spoiler"
	}

	row.SetTitle("Hidden Achievement")
	row.SetSubtitle(subtitle)
	row.SetSubtitleLines(1)
	row.SetTitleLines(1)
	row.SetActivatable(true)

	img := gtk.NewImageFromIconName("view-conceal-symbolic")
	img.SetPixelSize(48)
	img.SetMarginTop(8)
	img.SetMarginBottom(8)
	img.SetMarginStart(4)
	img.SetMarginEnd(4)
	row.AddPrefix(img)

	row.ConnectActivated(func() {
		if revealed {
			return
		}
		revealed = true
		row.SetTitle(ach.DisplayName)
		revealedSub := ach.Description
		if ach.GlobalPercent > 0 {
			revealedSub += fmt.Sprintf("  ·  %.1f%% global", ach.GlobalPercent)
		}
		row.SetSubtitle(revealedSub)
		row.SetActivatable(false)
		if ach.IconGrayPath != "" {
			img.SetFromFile(ach.IconGrayPath)
		} else {
			img.SetFromIconName("dialog-question")
		}
	})

	return row
}
