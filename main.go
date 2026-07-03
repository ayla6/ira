package main

import (
	"fmt"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"strings"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"
	"github.com/diamondburned/gotk4/pkg/gio/v2"
	"github.com/diamondburned/gotk4/pkg/glib/v2"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
	_ "github.com/sergeymakinen/go-ico"
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

		.sidebar-row-title {
			min-width: 0;
		}
	`)
	gtk.StyleContextAddProviderForDisplay(
		gdk.DisplayGetDefault(),
		cssProvider,
		gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
	)

	cfg := LoadConfig()
	steam := NewSteamClient(cfg.SteamAPIKey, cfg.SteamGridDBAPIKey, filepath.Join(saveDir, "data"))

	// Load whatever is already on disk. This does no network I/O, so the
	// window can appear almost instantly instead of blocking on Steam/asset
	// downloads for every game up front.
	games, err := loadGames(saveDir)
	if err != nil {
		fmt.Println("Error loading games:", err)
	}

	window.Show()
	buildUI(window, games, cfg, steam)

	// Enrich every game in the background: fetch missing achievement
	// definitions, titles, icons, hero art, and global unlock percentages.
	// Each game's UI row/content is refreshed in place as its data arrives,
	// so nothing blocks the window from being usable immediately.
	for i := range games {
		enrichGameAsync(games[i].AppID, filepath.Join(saveDir, games[i].AppID), steam)
	}
}

// enrichGameAsync performs all network-dependent work for a single game on a
// background goroutine, then hands the freshly loaded Game back to the UI
// thread via glib.IdleAdd so widgets are only ever touched from the main loop.
func enrichGameAsync(appID, gameDir string, steam *SteamClient) {
	go func() {
		// Generate achievement definitions if we don't have them yet.
		metaPath := filepath.Join(gameDir, "steam_settings", "achievements.json")
		if _, err := os.Stat(metaPath); os.IsNotExist(err) {
			if err := steam.GenerateSteamSettings(appID, gameDir); err != nil {
				fmt.Printf("Could not generate steam_settings for %s: %v\n", appID, err)
			}
		}

		// Reload from disk now that achievements.json may have appeared.
		game, err := loadGame(appID, gameDir)
		if err != nil {
			fmt.Printf("Failed reloading %s: %v\n", appID, err)
			return
		}

		// Resolve a proper title if we're still using the App ID fallback.
		if strings.HasPrefix(game.Name, "App ID:") {
			if name, err := steam.FetchNemirtingasGameName(appID); err == nil && name != "" {
				game.Name = name
			}
		}

		details, err := steam.FetchGameDetails(appID)
		if err != nil {
			fmt.Printf("Steam details unavailable for %s: %v\n", appID, err)
		} else {
			if strings.HasPrefix(game.Name, "App ID:") && details.Name != "" {
				game.Name = details.Name
			}
			iconPath, heroPath := steam.EnsureAssets(appID, details, game.IconPath != "")
			if game.IconPath == "" && iconPath != "" {
				game.IconPath = iconPath
			}
			if heroPath != "" {
				game.HeroImagePath = heroPath
			}
		}

		globalPcts, err := steam.FetchGlobalAchievements(appID)
		if err == nil {
			for j := range game.Achievements {
				game.Achievements[j].GlobalPercent = globalPcts[game.Achievements[j].Name]
			}
		}

		glib.IdleAdd(func() {
			onGameUpdated(game)
		})
	}()
}

// convertIcoToPng checks if a file is an .ico, and if so, decodes it (extracting
// the largest resolution image by default via go-ico) and saves it as .png.
func convertIcoToPng(icoPath string) (string, error) {
	if !strings.HasSuffix(strings.ToLower(icoPath), ".ico") {
		return icoPath, nil
	}

	pngPath := icoPath[:len(icoPath)-4] + ".png"

	if _, err := os.Stat(pngPath); err == nil {
		return pngPath, nil // Already converted
	}

	inFile, err := os.Open(icoPath)
	if err != nil {
		return icoPath, err
	}
	defer inFile.Close()

	img, _, err := image.Decode(inFile)
	if err != nil {
		return icoPath, err
	}

	outFile, err := os.Create(pngPath)
	if err != nil {
		return icoPath, err
	}
	defer outFile.Close()

	if err := png.Encode(outFile, img); err != nil {
		return icoPath, err
	}

	return pngPath, nil
}
