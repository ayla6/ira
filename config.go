package main

import (
	"encoding/json"
	"os"
	"path/filepath"
)

type Config struct {
	SteamAPIKey string `json:"steam_api_key"`
}

func configPath() string {
	dir, _ := os.UserConfigDir()
	return filepath.Join(dir, "achievement-viewer", "config.json")
}

func LoadConfig() *Config {
	data, err := os.ReadFile(configPath())
	if err != nil {
		return &Config{}
	}
	var c Config
	if err := json.Unmarshal(data, &c); err != nil {
		return &Config{}
	}
	return &c
}

func (c *Config) Save() error {
	path := configPath()
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0600)
}
