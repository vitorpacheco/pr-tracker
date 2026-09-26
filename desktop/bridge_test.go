package main

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

func TestBridgeSerializesCacheAndResolvesBrowserLink(t *testing.T) {
	cfg := config.Default()
	cfg.WorktreeDir = t.TempDir()
	cfg.Instances = []config.Instance{{Name: "test", Provider: config.GitHub, Host: "github.com"}}
	store, err := cache.Open(filepath.Join(t.TempDir(), "cache.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	item := provider.Item{Instance: "test", Provider: config.GitHub, Host: "github.com", Repo: "team/repo", Number: 1, Title: "Cached PR", URL: "https://github.com/team/repo/pull/1", Checks: []provider.Check{{Name: "test", State: provider.CISuccess}}}
	if err := store.Replace(context.Background(), cfg.Instances[0], []provider.Item{item}, time.Now()); err != nil {
		t.Fatal(err)
	}
	session := app.NewSession(cfg, store)
	defer session.Close()
	opened := ""
	bridge := &Bridge{session: session, openURL: func(url string) { opened = url }}
	view, err := bridge.Load()
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(view)
	if err != nil {
		t.Fatal(err)
	}
	var decoded struct {
		Items []struct {
			Key  string
			Data struct {
				Title  string
				Checks []provider.Check
			}
		}
		Errors map[string]string
	}
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}
	if len(decoded.Items) != 1 || decoded.Items[0].Key != item.Key() || decoded.Items[0].Data.Title != "Cached PR" || len(decoded.Items[0].Data.Checks) != 1 {
		t.Fatalf("contract = %s", data)
	}
	if err := bridge.OpenItem(item.Key()); err != nil || opened != item.URL {
		t.Fatalf("open = %q, %v", opened, err)
	}
	if err := bridge.OpenItem("https://attacker.example"); err == nil {
		t.Fatal("arbitrary link accepted")
	}
	result := bridge.Execute(Request{Key: "missing", Action: app.Approve})
	if result.Error == "" || result.Message != "" {
		t.Fatalf("invalid action = %+v", result)
	}
}

func TestBridgeErrorsRemainJSONStrings(t *testing.T) {
	session := app.NewSession(config.Default(), nil)
	defer session.Close()
	bridge := &Bridge{session: session, startupError: "startup cache error"}
	view := bridge.view(app.State{Snapshot: app.Snapshot{Errors: map[string]error{"offline": errors.New("offline")}, CacheError: errors.New("disk full")}})
	data, err := json.Marshal(view)
	if err != nil || !strings.Contains(string(data), `"offline":"offline"`) || view.CacheError != "disk full" || view.Items == nil {
		t.Fatalf("contract = %s, %v", data, err)
	}
}

func TestBridgeWaitsForStartupBeforeAccessingSession(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ready := make(chan struct{})
		bridge := &Bridge{ready: ready}
		done := make(chan error, 1)
		go func() { _, err := bridge.Load(); done <- err }()
		synctest.Wait()
		bridge.session = app.NewSession(config.Default(), nil)
		defer bridge.session.Close()
		close(ready)
		synctest.Wait()
		if err := <-done; err != nil {
			t.Fatal(err)
		}
	})
}

func TestBridgeLanguageUsesSharedConfiguration(t *testing.T) {
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	t.Setenv("LC_ALL", "pt_BR.UTF-8")
	t.Setenv("LANGUAGE", "")
	cfg := config.Default()
	session := app.NewSession(cfg, nil)
	defer session.Close()
	bridge := &Bridge{session: session}
	initial, err := bridge.Load()
	if err != nil || initial.Language != "pt" {
		t.Fatalf("initial = %+v, %v", initial, err)
	}
	for _, language := range []string{"en", "pt", "system"} {
		settings := bridge.Settings()
		settings.Language = language
		view, err := bridge.SaveSettings(settings)
		want := language
		if want == "system" {
			want = "pt"
		}
		if err != nil || view.Language != want {
			t.Fatalf("view = %+v, %v", view, err)
		}
		saved, _, err := config.Load()
		if err != nil || saved.Language != language {
			t.Fatalf("saved = %+v, %v", saved, err)
		}
	}
	invalid := bridge.Settings()
	invalid.Language = "fr"
	if _, err := bridge.SaveSettings(invalid); err == nil {
		t.Fatal("unsupported language accepted")
	}
	if bridge.Settings().Language != "system" {
		t.Fatal("failed save changed language")
	}
}
