package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
)

type AchievementStatus struct {
	Earned     bool  `json:"earned"`
	EarnedTime int64 `json:"earned_time"`
}

type StringOrMap struct {
	Val string
}

func (s *StringOrMap) UnmarshalJSON(data []byte) error {
	if len(data) == 0 {
		return nil
	}
	if data[0] == '"' {
		var str string
		if err := json.Unmarshal(data, &str); err != nil {
			return err
		}
		s.Val = str
		return nil
	} else if data[0] == '{' {
		var m map[string]string
		if err := json.Unmarshal(data, &m); err != nil {
			return err
		}
		if eng, ok := m["english"]; ok {
			s.Val = eng
		} else {
			for _, v := range m {
				s.Val = v
				break
			}
		}
		return nil
	}
	return fmt.Errorf("invalid type for string or map")
}

type AchievementMeta struct {
	Description StringOrMap `json:"description"`
	DisplayName StringOrMap `json:"displayName"`
	Hidden      any         `json:"hidden"`
	Icon        string      `json:"icon"`
	IconGray    string      `json:"icongray"`
	IconGrayAlt string      `json:"icon_gray"`
	Name        string      `json:"name"`
}

type MergedAchievement struct {
	Name          string
	DisplayName   string
	Description   string
	Hidden        bool
	Earned        bool
	EarnedTime    int64
	IconPath      string
	IconGrayPath  string
	GlobalPercent float64
}

type Game struct {
	AppID         string
	Name          string
	IconPath      string
	HeroImagePath string
	Achievements  []MergedAchievement
	EarnedCount   int
	TotalCount    int
}

func findIconPath(gameDir, iconField string) string {
	if iconField == "" {
		return ""
	}
	// If the field has no extension (e.g. "achievement_images/481510"),
	// the API returned a bare ID — no real image file to look up.
	if filepath.Ext(iconField) == "" {
		return ""
	}
	path := filepath.Join(gameDir, "steam_settings", iconField)
	if info, err := os.Stat(path); err == nil && !info.IsDir() {
		if converted, err := convertIcoToPng(path); err == nil {
			return converted
		}
		if !strings.HasSuffix(strings.ToLower(path), ".ico") {
			return path
		}
	}

	base := filepath.Base(iconField)
	candidates := []string{
		filepath.Join(gameDir, "steam_settings", base),
		filepath.Join(gameDir, "steam_settings", "achievement_images", base),
		filepath.Join(gameDir, "steam_settings", "img", base),
	}
	for _, cand := range candidates {
		if info, err := os.Stat(cand); err == nil && !info.IsDir() {
			if converted, err := convertIcoToPng(cand); err == nil {
				return converted
			}
			if !strings.HasSuffix(strings.ToLower(cand), ".ico") {
				return cand
			}
		}
	}
	return ""
}

// loadGames scans basePath for game directories and loads whatever is already
// present on disk. It performs no network access, so it returns almost
// instantly — this is what lets the window appear immediately on startup.
// Games that don't have steam_settings/achievements.json yet are still
// included (with zero achievements); the caller is expected to enrich them
// asynchronously via steam.GenerateSteamSettings and reload them in place.
func loadGames(basePath string) ([]Game, error) {
	var games []Game

	entries, err := os.ReadDir(basePath)
	if err != nil {
		return nil, err
	}

	var wg sync.WaitGroup
	var mu sync.Mutex

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}

		appID := entry.Name()
		// Steam App IDs are always numeric. Skip any non-numeric directory
		// (e.g. "settings", our own "data" asset cache, etc.) so it's never
		// mistaken for a game.
		if _, err := strconv.Atoi(appID); err != nil {
			continue
		}

		gameDir := filepath.Join(basePath, appID)

		wg.Add(1)
		go func(appID, gameDir string) {
			defer wg.Done()

			game, err := loadGame(appID, gameDir)
			if err != nil {
				fmt.Printf("Skipping game %s: %v\n", appID, err)
				return
			}
			mu.Lock()
			games = append(games, game)
			mu.Unlock()
		}(appID, gameDir)
	}

	wg.Wait()

	sort.Slice(games, func(i, j int) bool {
		return games[i].Name < games[j].Name
	})

	return games, nil
}

// SetAchievementEarned manually marks an achievement as earned (or not) directly
// in the game's achievements.json status file, without going through the game
// itself. When marking as earned manually, EarnedTime is deliberately left at
// 0 so it's clear in the UI that the unlock time isn't real/known.
func SetAchievementEarned(gameDir, achName string, earned bool) error {
	statusPath := filepath.Join(gameDir, "achievements.json")

	statusMap := make(map[string]AchievementStatus)
	if data, err := os.ReadFile(statusPath); err == nil {
		_ = json.Unmarshal(data, &statusMap)
	}

	statusMap[achName] = AchievementStatus{
		Earned:     earned,
		EarnedTime: 0,
	}

	b, err := json.MarshalIndent(statusMap, "", "    ")
	if err != nil {
		return err
	}
	return os.WriteFile(statusPath, b, 0644)
}

func loadGame(appID, gameDir string) (Game, error) {
	game := Game{
		AppID: appID,
		Name:  "App ID: " + appID,
	}

	titlePath := filepath.Join(gameDir, "steam_settings", "title.txt")
	if data, err := os.ReadFile(titlePath); err == nil {
		game.Name = strings.TrimSpace(string(data))
	}

	iconPath := filepath.Join(gameDir, "steam_settings", "icon.png")
	if _, err := os.Stat(iconPath); err == nil {
		game.IconPath = iconPath
	} else {
		icoPath := filepath.Join(gameDir, "steam_settings", "icon.ico")
		if _, err := os.Stat(icoPath); err == nil {
			if converted, err := convertIcoToPng(icoPath); err == nil {
				game.IconPath = converted
			}
		}
	}

	statusPath := filepath.Join(gameDir, "achievements.json")
	statusData, err := os.ReadFile(statusPath)
	statusMap := make(map[string]AchievementStatus)
	if err == nil {
		if err := json.Unmarshal(statusData, &statusMap); err != nil {
			fmt.Printf("Warning: failed to unmarshal status for %s: %v\n", appID, err)
		}
	}

	metaPath := filepath.Join(gameDir, "steam_settings", "achievements.json")
	metaData, err := os.ReadFile(metaPath)

	metaLoaded := false
	if err == nil {
		var metaList []AchievementMeta
		if err := json.Unmarshal(metaData, &metaList); err == nil {
			metaLoaded = true
			for _, meta := range metaList {
				status := statusMap[meta.Name]

				var hidden bool
				switch v := meta.Hidden.(type) {
				case bool:
					hidden = v
				case float64:
					hidden = v != 0
				case int:
					hidden = v != 0
				case string:
					hidden = v == "1" || v == "true"
				}

				iconGray := meta.IconGray
				if iconGray == "" {
					iconGray = meta.IconGrayAlt
				}

				ach := MergedAchievement{
					Name:         meta.Name,
					DisplayName:  meta.DisplayName.Val,
					Description:  meta.Description.Val,
					Hidden:       hidden,
					Earned:       status.Earned,
					EarnedTime:   status.EarnedTime,
					IconPath:     findIconPath(gameDir, meta.Icon),
					IconGrayPath: findIconPath(gameDir, iconGray),
				}

				game.Achievements = append(game.Achievements, ach)
				game.TotalCount++
				if ach.Earned {
					game.EarnedCount++
				}
			}
		} else {
			fmt.Printf("Meta load error for %s: %v\n", appID, err)
		}
	} else {
		fmt.Printf("Meta read error for %s: %v\n", appID, err)
	}

	if !metaLoaded {
		var keys []string
		for k := range statusMap {
			keys = append(keys, k)
		}
		sort.Strings(keys)

		for _, name := range keys {
			status := statusMap[name]
			ach := MergedAchievement{
				Name:        name,
				DisplayName: name,
				Description: "No description available.",
				Hidden:      false,
				Earned:      status.Earned,
				EarnedTime:  status.EarnedTime,
			}
			game.Achievements = append(game.Achievements, ach)
			game.TotalCount++
			if ach.Earned {
				game.EarnedCount++
			}
		}
	}

	return game, nil
}
