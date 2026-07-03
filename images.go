package main

import (
	"runtime"
	"sync"

	"github.com/diamondburned/gotk4/pkg/gdk/v4"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"
)

// textureCache decodes each local image file into a *gdk.Texture exactly once
// and reuses it for every widget. gtk.NewImageFromFile / Picture.SetFilename
// decode a fresh texture per call, so without this, switching between games
// re-decodes (and re-allocates) every icon each time. clear() drops the cache
// so the pixel data is freed once no widget references it — used when hiding
// to the background.
type textureCache struct {
	mu sync.Mutex
	m  map[string]*gdk.Texture
}

var textures = &textureCache{m: make(map[string]*gdk.Texture)}

// loadTexture returns the cached texture for path, decoding it on first use.
// Returns nil if path is empty or decoding fails.
func (c *textureCache) loadTexture(path string) *gdk.Texture {
	if path == "" {
		return nil
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if t, ok := c.m[path]; ok {
		return t
	}
	t, err := gdk.NewTextureFromFilename(path)
	if err != nil {
		return nil
	}
	c.m[path] = t
	return t
}

// clear drops all cached textures. The pixel data is only actually freed once
// no widget still references it, so call this after tearing down the UI.
func (c *textureCache) clear() {
	c.mu.Lock()
	c.m = make(map[string]*gdk.Texture)
	c.mu.Unlock()
}

// textureFor returns the cached (or just-decoded) texture for a path, or nil
// if the path is empty or undecodable. Callers must nil-check before passing
// it to SetFromPaintable/SetPaintable.
func textureFor(path string) *gdk.Texture {
	return textures.loadTexture(path)
}

// setImage sets img from path, pulling from the shared texture cache so
// repeat calls (e.g. switching back to a game) are free.
func setImage(img *gtk.Image, path string) {
	if t := textureFor(path); t != nil {
		img.SetFromPaintable(t)
	}
}

// setPicture is the gtk.Picture equivalent of setImage.
func setPicture(pic *gtk.Picture, path string) {
	if t := textureFor(path); t != nil {
		pic.SetPaintable(t)
	}
}

// newImageFromFile is a cache-backed gtk.NewImageFromFile. On a decode failure
// it falls back to a generic icon.
func newImageFromFile(path string) *gtk.Image {
	if t := textureFor(path); t != nil {
		return gtk.NewImageFromPaintable(t)
	}
	return gtk.NewImageFromIconName("application-x-executable")
}

// gcSoon triggers a garbage collection off the main thread. Once
// destroyWidget/clearChildren have unparented a subtree, every widget in it is
// weakly referenced; a GC pass collects those wrappers and runs gotk4's
// finalizers, which queue the GObject unref on the main loop — that's what
// actually returns the widget and texture memory. One collection is enough to
// kick the whole chain off.
func gcSoon() {
	go runtime.GC()
}

// Shared CSS providers, created lazily (GTK isn't initialized until the
// GApplication runs). Reused by every global-stats row instead of building a
// new CSSProvider per row.
var (
	globalBarCSSProvider     *gtk.CSSProvider
	globalBarCSSProviderOnce sync.Once

	grayScaleCSSProvider     *gtk.CSSProvider
	grayScaleCSSProviderOnce sync.Once
)

func sharedBarCSSProvider() *gtk.CSSProvider {
	globalBarCSSProviderOnce.Do(func() {
		globalBarCSSProvider = gtk.NewCSSProvider()
		globalBarCSSProvider.LoadFromString(`
			trough {
				background-color: transparent;
				border: none;
			}
			progress {
				border: none;
				border-radius: 0;
			}
		`)
	})
	return globalBarCSSProvider
}

func sharedGrayScaleCSSProvider() *gtk.CSSProvider {
	grayScaleCSSProviderOnce.Do(func() {
		grayScaleCSSProvider = gtk.NewCSSProvider()
		grayScaleCSSProvider.LoadFromString("image { filter: grayscale(100%); }")
	})
	return grayScaleCSSProvider
}
