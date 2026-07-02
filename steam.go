package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// SteamClient handles Steam API requests and local disk caching.
type SteamClient struct {
	APIKey            string
	SteamGridDBAPIKey string
	cacheDir          string
	http              *http.Client
}

// SteamGameDetails holds the subset of store API data we care about.
type SteamGameDetails struct {
	Name         string `json:"name"`
	HeaderImage  string `json:"header_image"`
	CapsuleImage string `json:"capsule_imagev5"`
}

func NewSteamClient(apiKey, sgdbKey string) *SteamClient {
	cacheBase, _ := os.UserCacheDir()
	return &SteamClient{
		APIKey:            apiKey,
		SteamGridDBAPIKey: sgdbKey,
		cacheDir:          filepath.Join(cacheBase, "achievement-viewer"),
		http:              &http.Client{Timeout: 20 * time.Second},
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
				Name    string      `json:"name"`
				Percent json.Number `json:"percent"`
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

func (s *SteamClient) findCachedIcon(appID string) string {
	dir := s.gameDir(appID)
	for _, ext := range []string{".png", ".ico", ".jpg", ".webp"} {
		path := filepath.Join(dir, "icon"+ext)
		if _, err := os.Stat(path); err == nil {
			return path
		}
	}
	return ""
}

func (s *SteamClient) findCachedHero(appID string) string {
	path := filepath.Join(s.gameDir(appID), "library_hero.jpg")
	if _, err := os.Stat(path); err == nil {
		return path
	}
	return ""
}

func (s *SteamClient) fetchSteamGridDBIconURL(appID string) (string, error) {
	req, err := http.NewRequest("GET", "https://www.steamgriddb.com/api/v2/icons/steam/"+appID, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Authorization", "Bearer "+s.SteamGridDBAPIKey)

	resp, err := s.http.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("steamgriddb returned status %d", resp.StatusCode)
	}

	var raw struct {
		Success bool `json:"success"`
		Data    []struct {
			URL string `json:"url"`
		} `json:"data"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return "", err
	}

	if !raw.Success || len(raw.Data) == 0 {
		return "", fmt.Errorf("no icons found on steamgriddb")
	}

	return raw.Data[0].URL, nil
}

// EnsureAssets downloads icon and library hero for a game if not yet cached.
// Hero image always comes from the Steam CDN using the known URL pattern.
// Returns the local paths (empty string when unavailable).
func (s *SteamClient) EnsureAssets(appID string, d *SteamGameDetails, hasLocalIcon bool) (iconPath, heroPath string) {
	dir := s.gameDir(appID)

	// 1. Icon Resolution
	if hasLocalIcon {
		iconPath = ""
	} else {
		iconPath = s.findCachedIcon(appID)
		if iconPath == "" {
			// Try SteamGridDB if API key is set
			if s.SteamGridDBAPIKey != "" {
				if sgdbUrl, err := s.fetchSteamGridDBIconURL(appID); err == nil && sgdbUrl != "" {
					ext := filepath.Ext(sgdbUrl)
					if ext == "" {
						ext = ".png"
					}
					dest := filepath.Join(dir, "icon"+ext)
					if err := s.downloadFile(sgdbUrl, dest); err == nil {
						if converted, err := convertIcoToPng(dest); err == nil {
							iconPath = converted
						} else {
							iconPath = dest
						}
					}
				}
			}

			// Fallback to Steam capsule if SteamGridDB fails or no API key
			if iconPath == "" && d != nil && d.CapsuleImage != "" {
				dest := filepath.Join(dir, "icon.jpg")
				if err := s.downloadFile(d.CapsuleImage, dest); err == nil {
					iconPath = dest
				}
			}
		}
	}

	// 2. Hero Resolution
	heroPath = s.findCachedHero(appID)
	if heroPath == "" {
		heroURL := fmt.Sprintf(
			"https://shared.steamstatic.com/store_item_assets/steam/apps/%s/library_hero_2x.jpg",
			appID,
		)
		dest := filepath.Join(dir, "library_hero.jpg")
		if err := s.downloadFile(heroURL, dest); err == nil {
			heroPath = dest
		}
	}

	return iconPath, heroPath
}

// SteamSchemaAchievement is the shape Steam returns for each achievement in GetSchemaForGame.
type SteamSchemaAchievement struct {
	Name        string `json:"name"`
	DefaultVal  int    `json:"defaultvalue"`
	DisplayName string `json:"displayName"`
	Hidden      int    `json:"hidden"`
	Description string `json:"description"`
	Icon        string `json:"icon"`
	IconGray    string `json:"icongray"`
}

// goldbergAchievement is the shape Goldberg (and our parser) expect in achievements.json.
type goldbergAchievement struct {
	Name        string `json:"name"`
	DisplayName string `json:"displayName"`
	Description string `json:"description"`
	Hidden      string `json:"hidden"`
	Icon        string `json:"icon"`
	IconGray    string `json:"icon_gray"`
}

// GenerateSteamSettings fetches the game schema from Steam and writes
// steam_settings/achievements.json + downloads achievement images into
// steam_settings/achievement_images/ under gameDir.
// Returns a descriptive error (or nil on success).
func (s *SteamClient) GenerateSteamSettings(appID, gameDir string) error {
	if s.APIKey == "" {
		return fmt.Errorf("no Steam API key configured — add it in Settings first")
	}

	// 1. Fetch the game schema
	url := fmt.Sprintf(
		"https://api.steampowered.com/ISteamUserStats/GetSchemaForGame/v2/?key=%s&appid=%s&format=json",
		s.APIKey, appID,
	)
	resp, err := s.http.Get(url)
	if err != nil {
		return fmt.Errorf("schema request failed: %w", err)
	}
	defer resp.Body.Close()

	var raw struct {
		Game struct {
			AvailableGameStats struct {
				Achievements []SteamSchemaAchievement `json:"achievements"`
			} `json:"availableGameStats"`
		} `json:"game"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return fmt.Errorf("failed to decode schema: %w", err)
	}

	achs := raw.Game.AvailableGameStats.Achievements
	if len(achs) == 0 {
		return fmt.Errorf("Steam returned 0 achievements for app %s (check appid and API key)", appID)
	}

	// 2. Convert to Goldberg format and collect icon URLs
	settingsDir := filepath.Join(gameDir, "steam_settings")
	imgDir := filepath.Join(settingsDir, "achievement_images")
	if err := os.MkdirAll(imgDir, 0755); err != nil {
		return fmt.Errorf("could not create steam_settings dir: %w", err)
	}

	type iconJob struct{ url, dest string }
	var jobs []iconJob
	var out []goldbergAchievement

	for _, a := range achs {
		hidden := "0"
		if a.Hidden != 0 {
			hidden = "1"
		}
		// Icon filename is just the basename of the URL
		iconBase := filepath.Base(a.Icon)
		iconGrayBase := filepath.Base(a.IconGray)

		out = append(out, goldbergAchievement{
			Name:        a.Name,
			DisplayName: a.DisplayName,
			Description: a.Description,
			Hidden:      hidden,
			Icon:        "achievement_images/" + iconBase,
			IconGray:    "achievement_images/" + iconGrayBase,
		})

		if a.Icon != "" {
			jobs = append(jobs, iconJob{a.Icon, filepath.Join(imgDir, iconBase)})
		}
		if a.IconGray != "" {
			jobs = append(jobs, iconJob{a.IconGray, filepath.Join(imgDir, iconGrayBase)})
		}
	}

	// 3. Write achievements.json
	b, err := json.MarshalIndent(out, "", "    ")
	if err != nil {
		return fmt.Errorf("failed to marshal achievements: %w", err)
	}
	if err := os.WriteFile(filepath.Join(settingsDir, "achievements.json"), b, 0644); err != nil {
		return fmt.Errorf("failed to write achievements.json: %w", err)
	}

	// 4. Download icons concurrently (8 workers)
	jobCh := make(chan iconJob, len(jobs))
	for _, j := range jobs {
		jobCh <- j
	}
	close(jobCh)

	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := range jobCh {
				// Skip if already cached
				if _, err := os.Stat(j.dest); err == nil {
					continue
				}
				if err := s.downloadFile(j.url, j.dest); err != nil {
					fmt.Printf("  icon download failed %s: %v\n", j.url, err)
				}
			}
		}()
	}
	wg.Wait()

	fmt.Printf("Generated steam_settings for app %s: %d achievements, %d icons\n", appID, len(out), len(jobs))
	return nil
}
