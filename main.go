package main

import (
	"fmt"
	"os"
	"strings"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"
	"github.com/diamondburned/gotk4/pkg/gio/v2"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
)

const saveDir = "/data/Games/Saves/GSE Saves"

func main() {
	app := adw.NewApplication("com.github.achievement.viewer", gio.ApplicationFlagsNone)
	app.ConnectActivate(func() {
		activate(app)
	})
	if code := app.Run(os.Args); code > 0 {
		os.Exit(code)
	}
}

func activate(app *adw.Application) {
	window := adw.NewApplicationWindow(&app.Application)
	window.SetTitle("Achievement Viewer")
	window.SetDefaultSize(1100, 720)

	// Load custom CSS
	cssProvider := gtk.NewCSSProvider()
	cssProvider.LoadFromString(`
		.hero-gradient {
			background-image: linear-gradient(
				to bottom,
				rgba(0, 0, 0, 0) 0%,
				rgba(0, 0, 0, 0.85) 100%
			);
		}
		
		/* Translucent hero progress bar track */
		.hero-progress trough {
			background-color: transparent;
			border: none;
			border-radius: 0;
		}
		.hero-progress progress {
			background-color: @accent_color;
			border: none;
			border-radius: 0;
		}
		.hero-progress {
			min-height: 8px;
			border: none;
		}
	`)
	gtk.StyleContextAddProviderForDisplay(
		gdk.DisplayGetDefault(),
		cssProvider,
		gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
	)

	cfg := LoadConfig()
	steam := NewSteamClient(cfg.SteamAPIKey, cfg.SteamGridDBAPIKey)

	games, err := loadGames(saveDir, steam)
	if err != nil {
		fmt.Println("Error loading games:", err)
	}

	// Enrich each game with Steam data (uses cache; first run may be slow)
	for i := range games {
		g := &games[i]
		details, err := steam.FetchGameDetails(g.AppID)
		if err != nil {
			fmt.Printf("Steam details unavailable for %s: %v\n", g.AppID, err)
		} else {
			// Fill in name if we only have the App ID fallback
			if strings.HasPrefix(g.Name, "App ID:") && details.Name != "" {
				g.Name = details.Name
			}
			// Download icon/hero if not already locally present
			iconPath, heroPath := steam.EnsureAssets(g.AppID, details, g.IconPath != "")
			if g.IconPath == "" && iconPath != "" {
				g.IconPath = iconPath
			}
			if heroPath != "" {
				g.HeroImagePath = heroPath
			}
		}

		// Fetch global achievement percentages (permanently cached)
		globalPcts, err := steam.FetchGlobalAchievements(g.AppID)
		if err != nil {
			fmt.Printf("Global achievements unavailable for %s: %v\n", g.AppID, err)
		} else {
			for j := range g.Achievements {
				g.Achievements[j].GlobalPercent = globalPcts[g.Achievements[j].Name]
			}
		}
	}

	buildUI(window, games, cfg, steam)
	window.Show()
}
