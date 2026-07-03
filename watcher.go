package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"sync"
	"time"

	"github.com/diamondburned/gotk4/pkg/glib/v2"
	"github.com/fsnotify/fsnotify"
)

// AchievementWatcher watches every tracked game's achievements.json (the
// small "is this unlocked" status file Goldberg/GSE writes to) and reloads +
// re-renders that game the moment it changes — no polling involved, so it's
// effectively free until something actually happens. It also fires a desktop
// notification (if enabled) the first time it observes a new achievement
// flip to earned.
type AchievementWatcher struct {
	cfg *Config

	fs      *fsnotify.Watcher
	rootDir string

	// OnNewGameDir, if set, is called (on an arbitrary goroutine, never the
	// GTK main thread) whenever a new numeric-named directory shows up
	// directly inside the watched saves root — e.g. a game folder dropped in
	// by some other tool while the app is already running. It's the caller's
	// job to enrich/watch/display it; the watcher itself only detects it.
	OnNewGameDir func(appID, gameDir string)

	mu         sync.Mutex
	dirToApp   map[string]string          // watched directory -> appID
	lastEarned map[string]map[string]bool // appID -> achievement name -> earned
	debounce   map[string]*time.Timer     // appID -> pending-reload timer
}

// NewAchievementWatcher creates a watcher. The caller is expected to call
// Watch for every game it wants tracked, then Start to begin processing
// events.
func NewAchievementWatcher(cfg *Config) (*AchievementWatcher, error) {
	fw, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	return &AchievementWatcher{
		cfg:        cfg,
		fs:         fw,
		dirToApp:   make(map[string]string),
		lastEarned: make(map[string]map[string]bool),
		debounce:   make(map[string]*time.Timer),
	}, nil
}

// Watch starts tracking one game's directory for achievements.json changes.
// It's cheap to call redundantly (e.g. re-adding an already-watched game).
func (w *AchievementWatcher) Watch(appID, gameDir string, achievements []MergedAchievement) {
	w.mu.Lock()
	earned := make(map[string]bool, len(achievements))
	for _, a := range achievements {
		earned[a.Name] = a.Earned
	}
	w.lastEarned[appID] = earned
	alreadyWatching := false
	for _, existing := range w.dirToApp {
		if existing == appID {
			alreadyWatching = true
			break
		}
	}
	w.dirToApp[gameDir] = appID
	w.mu.Unlock()

	if alreadyWatching {
		return
	}
	// A single non-recursive watch on the game's directory is all that's
	// needed — achievements.json lives directly inside it, and watching a
	// whole directory (rather than the file itself) also survives the file
	// not existing yet or being replaced outright, which editors/emulators
	// sometimes do instead of writing in place.
	if err := w.fs.Add(gameDir); err != nil {
		fmt.Printf("Could not watch %s for live updates: %v\n", gameDir, err)
	}
}

// WatchRoot additionally watches the saves directory itself (still just one
// more entry on the same single fsnotify.Watcher/goroutine, not a second
// watcher) so that a brand new game folder appearing there — created by
// something other than this app's own "Add Game" flow — is picked up
// automatically instead of requiring a restart.
func (w *AchievementWatcher) WatchRoot(dir string) error {
	w.rootDir = dir
	return w.fs.Add(dir)
}

// Start begins processing filesystem events in the background. Safe to call
// once; further calls are no-ops.
func (w *AchievementWatcher) Start() {
	go w.loop()
}

func (w *AchievementWatcher) loop() {
	for {
		select {
		case event, ok := <-w.fs.Events:
			if !ok {
				return
			}
			w.handleEvent(event)

		case err, ok := <-w.fs.Errors:
			if !ok {
				return
			}
			fmt.Println("Achievement watcher error:", err)
		}
	}
}

func (w *AchievementWatcher) handleEvent(event fsnotify.Event) {
	// A new directory appearing directly inside the saves root: only
	// relevant if it looks like a Steam App ID (purely numeric), matching
	// the same rule loadGames uses to decide what's a game.
	if w.rootDir != "" && filepath.Dir(event.Name) == w.rootDir && event.Op&fsnotify.Create != 0 {
		appID := filepath.Base(event.Name)
		if _, err := strconv.Atoi(appID); err == nil {
			if info, statErr := os.Stat(event.Name); statErr == nil && info.IsDir() {
				if w.OnNewGameDir != nil {
					w.OnNewGameDir(appID, event.Name)
				}
			}
		}
		return
	}

	if filepath.Base(event.Name) != "achievements.json" {
		return
	}
	if event.Op&(fsnotify.Write|fsnotify.Create) == 0 {
		return
	}
	w.scheduleReload(filepath.Dir(event.Name))
}

// scheduleReload debounces bursts of writes to the same file (some tools
// write in several small syscalls) so a single unlock doesn't trigger
// multiple reloads.
func (w *AchievementWatcher) scheduleReload(gameDir string) {
	w.mu.Lock()
	appID, ok := w.dirToApp[gameDir]
	if !ok {
		w.mu.Unlock()
		return
	}
	if t, exists := w.debounce[appID]; exists {
		t.Stop()
	}
	w.debounce[appID] = time.AfterFunc(300*time.Millisecond, func() {
		w.reload(appID, gameDir)
	})
	w.mu.Unlock()
}

// reload re-reads the game from disk, diffs against the last known earned
// set to notify about anything newly unlocked, and pushes the fresh data to
// the UI thread.
func (w *AchievementWatcher) reload(appID, gameDir string) {
	game, err := loadGame(appID, gameDir)
	if err != nil {
		fmt.Printf("Live-reload of %s failed: %v\n", appID, err)
		return
	}

	w.mu.Lock()
	previous := w.lastEarned[appID]
	newlyEarned := make([]MergedAchievement, 0)
	current := make(map[string]bool, len(game.Achievements))
	for _, a := range game.Achievements {
		current[a.Name] = a.Earned
		if a.Earned && !previous[a.Name] {
			newlyEarned = append(newlyEarned, a)
		}
	}
	w.lastEarned[appID] = current
	w.mu.Unlock()

	glib.IdleAdd(func() {
		if onGameUpdated != nil {
			onGameUpdated(game)
		}
		if w.cfg.NotificationsEnabled {
			for _, a := range newlyEarned {
				w.notify(game.Name, a)
			}
		}
	})
}

// notify shows a desktop notification for a newly-unlocked achievement.
//
// This shells out to notify-send (the standard freedesktop.org notification
// CLI, part of libnotify and present on virtually every Linux desktop)
// rather than using GApplication's SendNotification. GApplication
// notifications are delivered over D-Bus keyed by the app's application-id,
// and several notification daemons (notably GNOME Shell) silently drop them
// unless a matching .desktop file is installed for that ID — which won't be
// the case for a binary just run directly off disk. notify-send has no such
// requirement, making it the more reliable (and, being a single one-shot
// process spawned only when something actually unlocks, still very light)
// choice here.
func (w *AchievementWatcher) notify(gameName string, ach MergedAchievement) {
	title := fmt.Sprintf("%s — Achievement Unlocked", gameName)
	body := ach.DisplayName
	if ach.Description != "" {
		body = fmt.Sprintf("%s\n%s", ach.DisplayName, ach.Description)
	}

	cmd := exec.Command("notify-send", "--app-name=Achievement Viewer", "--icon=starred-symbolic", title, body)
	if err := cmd.Run(); err != nil {
		fmt.Printf("Could not show notification for %s: %v\n", ach.DisplayName, err)
	}
}
