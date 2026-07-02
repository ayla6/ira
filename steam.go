package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

// SteamClient handles Steam API requests and local disk caching.
type SteamClient struct {
	APIKey   string
	cacheDir string
	http     *http.Client
}

// SteamGameDetails holds the subset of store API data we care about.
type SteamGameDetails struct {
	Name         string `json:"name"`
	HeaderImage  string `json:"header_image"`
	CapsuleImage string `json:"capsule_imagev5"`
}

func NewSteamClient(apiKey string) *SteamClient {
	cacheBase, _ := os.UserCacheDir()
	return &SteamClient{
		APIKey:   apiKey,
		cacheDir: filepath.Join(cacheBase, "achievement-viewer"),
		http:     &http.Client{Timeout: 20 * time.Second},
	}
}

func (s *SteamClient) gameDir(appID string) string {
	return filepath.Join(s.cacheDir, appID)
}

// FetchGameDetails returns name/icon/hero URLs from the Steam store API.
// Cached to disk; subsequent calls are instant.
func (s *SteamClient) FetchGameDetails(appID string) (*SteamGameDetails, error) {
	cachePath := filepath.Join(s.gameDir(appID), "appdetails.json")
	if data, err := os.ReadFile(cachePath); err == nil {
		var d SteamGameDetails
		if json.Unmarshal(data, &d) == nil {
			return &d, nil
		}
	}

	resp, err := s.http.Get("https://store.steampowered.com/api/appdetails?appids=" + appID)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	var raw map[string]struct {
		Success bool             `json:"success"`
		Data    SteamGameDetails `json:"data"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return nil, err
	}
	entry, ok := raw[appID]
	if !ok || !entry.Success {
		return nil, fmt.Errorf("no store data for app %s", appID)
	}

	d := &entry.Data
	_ = os.MkdirAll(s.gameDir(appID), 0755)
	if b, err := json.Marshal(d); err == nil {
		_ = os.WriteFile(cachePath, b, 0644)
	}
	return d, nil
}

// FetchGlobalAchievements returns achievement name → global unlock %.
// Cache is permanent — once written it is never invalidated.
func (s *SteamClient) FetchGlobalAchievements(appID string) (map[string]float64, error) {
	cachePath := filepath.Join(s.gameDir(appID), "global_achievements.json")
	if data, err := os.ReadFile(cachePath); err == nil {
		var m map[string]float64
		if json.Unmarshal(data, &m) == nil {
			return m, nil
		}
	}

	url := fmt.Sprintf(
		"https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v0002/?gameid=%s&format=json",
		appID,
	)
	resp, err := s.http.Get(url)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	var raw struct {
		AchievementPercentages struct {
			Achievements []struct {
				Name    string          `json:"name"`
				Percent json.Number     `json:"percent"`
			} `json:"achievements"`
		} `json:"achievementpercentages"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return nil, err
	}

	m := make(map[string]float64)
	for _, a := range raw.AchievementPercentages.Achievements {
		pct, err := a.Percent.Float64()
		if err != nil {
			fmt.Printf("  skipping %s: bad percent value %q: %v\n", a.Name, a.Percent.String(), err)
			continue
		}
		m[a.Name] = pct
	}
	_ = os.MkdirAll(s.gameDir(appID), 0755)
	if b, err := json.Marshal(m); err == nil {
		_ = os.WriteFile(cachePath, b, 0644)
	}
	return m, nil
}

func (s *SteamClient) downloadFile(url, destPath string) error {
	if err := os.MkdirAll(filepath.Dir(destPath), 0755); err != nil {
		return err
	}
	resp, err := s.http.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	f, err := os.Create(destPath)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = io.Copy(f, resp.Body)
	return err
}

// EnsureAssets downloads icon and library hero for a game if not yet cached.
// Hero image always comes from the Steam CDN using the known URL pattern.
// Returns the local paths (empty string when unavailable).
func (s *SteamClient) EnsureAssets(appID string, d *SteamGameDetails) (iconPath, heroPath string) {
	dir := s.gameDir(appID)

	// Icon: from store API capsule image
	iconPath = filepath.Join(dir, "icon.jpg")
	if _, err := os.Stat(iconPath); os.IsNotExist(err) {
		if d.CapsuleImage != "" {
			if err := s.downloadFile(d.CapsuleImage, iconPath); err != nil {
				fmt.Printf("icon download failed for %s: %v\n", appID, err)
				iconPath = ""
			}
		} else {
			iconPath = ""
		}
	} else if err != nil {
		iconPath = ""
	}

	// Hero: always from Steam CDN library_hero_2x — use a distinct filename
	heroURL := fmt.Sprintf(
		"https://shared.steamstatic.com/store_item_assets/steam/apps/%s/library_hero_2x.jpg",
		appID,
	)
	heroPath = filepath.Join(dir, "library_hero.jpg")
	if _, err := os.Stat(heroPath); os.IsNotExist(err) {
		if err := s.downloadFile(heroURL, heroPath); err != nil {
			fmt.Printf("hero download failed for %s: %v\n", appID, err)
			heroPath = ""
		}
	} else if err != nil {
		heroPath = ""
	}

	return iconPath, heroPath
}
