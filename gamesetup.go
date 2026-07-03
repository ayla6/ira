package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// detectAppID looks for a steam_appid.txt in the game folder (Goldberg/GSE
// convention places it either at the root or inside steam_settings/) and
// returns the app ID string if found.
func detectAppID(folder string) (string, bool) {
	candidates := []string{
		filepath.Join(folder, "steam_appid.txt"),
		filepath.Join(folder, "steam_settings", "steam_appid.txt"),
	}
	for _, p := range candidates {
		data, err := os.ReadFile(p)
		if err != nil {
			continue
		}
		id := strings.TrimSpace(string(data))
		if id == "" {
			continue
		}
		if _, err := strconv.Atoi(id); err != nil {
			continue
		}
		return id, true
	}
	return "", false
}

// AddGameFromFolder wires up a game installation folder so the viewer can
// track it:
//
//  1. Ensures <folder>/steam_settings exists.
//  2. Ensures <folder>/steam_settings/steam_appid.txt exists (writing appID).
//  3. Symlinks <folder>/steam_settings into the saves directory, so the
//     viewer (which only scans the saves directory) can see it.
//  4. Downloads achievement definitions into steam_settings.
//
// It returns the appID used, and the game's directory inside the saves tree
// (which the caller should pass to loadGame to display it).
func AddGameFromFolder(folder, appID string, steam *SteamClient) (savesGameDir string, err error) {
	folder = strings.TrimSpace(folder)
	appID = strings.TrimSpace(appID)
	if folder == "" {
		return "", fmt.Errorf("no folder selected")
	}
	if _, err := strconv.Atoi(appID); appID == "" || err != nil {
		return "", fmt.Errorf("invalid Steam App ID %q", appID)
	}

	settingsDir := filepath.Join(folder, "steam_settings")
	if err := os.MkdirAll(settingsDir, 0755); err != nil {
		return "", fmt.Errorf("could not create steam_settings: %w", err)
	}

	appIDPath := filepath.Join(settingsDir, "steam_appid.txt")
	if _, err := os.Stat(appIDPath); os.IsNotExist(err) {
		if err := os.WriteFile(appIDPath, []byte(appID), 0644); err != nil {
			return "", fmt.Errorf("could not write steam_appid.txt: %w", err)
		}
	}

	savesGameDir = filepath.Join(saveDir, appID)
	if err := os.MkdirAll(saveDir, 0755); err != nil {
		return "", fmt.Errorf("could not create saves directory: %w", err)
	}

	linkPath := filepath.Join(savesGameDir, "steam_settings")
	if _, statErr := os.Lstat(linkPath); statErr != nil {
		if err := os.MkdirAll(savesGameDir, 0755); err != nil {
			return "", fmt.Errorf("could not create game save directory: %w", err)
		}
		if err := os.Symlink(settingsDir, linkPath); err != nil {
			return "", fmt.Errorf("could not symlink steam_settings into saves: %w", err)
		}
	}

	if err := steam.GenerateSteamSettings(appID, savesGameDir); err != nil {
		return savesGameDir, fmt.Errorf("achievements could not be downloaded: %w", err)
	}

	return savesGameDir, nil
}
