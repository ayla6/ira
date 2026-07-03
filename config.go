package main

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Config struct {
	SteamAPIKey       string `json:"steam_api_key,omitempty"`
	SteamGridDBAPIKey string `json:"steam_griddb_api_key,omitempty"`
}

func configPath() string {
	dir, _ := os.UserConfigDir()
	return filepath.Join(dir, "achievement-viewer", "config.json")
}

func getSecret(key string) string {
	cmd := exec.Command("secret-tool", "lookup", "app", "achievement-viewer", "key", key)
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

func setSecret(key, value string) error {
	if value == "" {
		// Clear secret if empty
		_ = exec.Command("secret-tool", "clear", "app", "achievement-viewer", "key", key).Run()
		return nil
	}
	cmd := exec.Command("secret-tool", "store", "--label=Achievement Viewer Key", "app", "achievement-viewer", "key", key)
	cmd.Stdin = bytes.NewBufferString(value)
	return cmd.Run()
}

func LoadConfig() *Config {
	c := &Config{}

	// 1. Load fallback plaintext config
	data, err := os.ReadFile(configPath())
	if err == nil {
		_ = json.Unmarshal(data, c)
	}

	// 2. Override with secure keyring values if available
	if steamKey := getSecret("steam"); steamKey != "" {
		c.SteamAPIKey = steamKey
	}
	if sgdbKey := getSecret("steamgriddb"); sgdbKey != "" {
		c.SteamGridDBAPIKey = sgdbKey
	}

	return c
}

func (c *Config) Save() error {
	// Try keyring first
	steamErr := setSecret("steam", c.SteamAPIKey)
	sgdbErr := setSecret("steamgriddb", c.SteamGridDBAPIKey)

	// Save plaintext config. If keyring succeeded, we omit the key to be secure.
	// If keyring failed, we fall back to plaintext storage in config.json.
	plaintextConfig := Config{}
	if steamErr != nil {
		plaintextConfig.SteamAPIKey = c.SteamAPIKey
	}
	if sgdbErr != nil {
		plaintextConfig.SteamGridDBAPIKey = c.SteamGridDBAPIKey
	}

	path := configPath()
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(plaintextConfig, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0600)
}
