package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
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
	path := filepath.Join(gameDir, "steam_settings", iconField)
	if _, err := os.Stat(path); err == nil {
		return path
	}

	base := filepath.Base(iconField)
	candidates := []string{
		filepath.Join(gameDir, "steam_settings", base),
		filepath.Join(gameDir, "steam_settings", "achievement_images", base),
		filepath.Join(gameDir, "steam_settings", "img", base),
	}
	for _, cand := range candidates {
		if _, err := os.Stat(cand); err == nil {
			return cand
		}
	}
	return path
}

func loadGames(basePath string) ([]Game, error) {
	var games []Game

	entries, err := os.ReadDir(basePath)
	if err != nil {
		return nil, err
	}

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}

		appID := entry.Name()
		if appID == "settings" {
			continue
		}

		gameDir := filepath.Join(basePath, appID)

		game, err := loadGame(appID, gameDir)
		if err != nil {
			fmt.Printf("Skipping game %s: %v\n", appID, err)
		} else if len(game.Achievements) == 0 {
			fmt.Printf("Skipping game %s: no achievements found\n", appID)
		} else {
			fmt.Printf("Loaded game %s (%d achievements)\n", game.Name, len(game.Achievements))
			games = append(games, game)
		}
	}

	sort.Slice(games, func(i, j int) bool {
		return games[i].Name < games[j].Name
	})

	return games, nil
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
	}

	statusPath := filepath.Join(gameDir, "achievements.json")
	statusData, err := os.ReadFile(statusPath)
	if err != nil {
		return game, err
	}

	var statusMap map[string]AchievementStatus
	if err := json.Unmarshal(statusData, &statusMap); err != nil {
		return game, err
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
