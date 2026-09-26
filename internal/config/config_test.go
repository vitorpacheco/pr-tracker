package config

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"
)

func TestDirUsesXDGOnLinux(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("XDG only applies to Linux")
	}
	t.Setenv("XDG_CONFIG_HOME", "/xdg")
	d, err := Dir()
	if err != nil || d != "/xdg/pr-tracker" {
		t.Fatalf("Dir() = %q, %v", d, err)
	}
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("HOME", "/home/u")
	if d, _ := Dir(); d != filepath.Join("/home/u", ".config", "pr-tracker") {
		t.Fatalf("fallback Dir() = %q", d)
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	p := filepath.Join(t.TempDir(), "cfg", "config.toml")
	t.Setenv("PR_TRACKER_CONFIG", p)
	cfg, created, err := Load()
	if err != nil || !created {
		t.Fatalf("Load() created=%v err=%v", created, err)
	}
	if err := cfg.UpsertInstance("", Instance{Provider: GitLab, Host: "https://gitlab.corp.io/"}); err != nil {
		t.Fatal(err)
	}
	if err := cfg.UpsertInstance("", Instance{Provider: GitLab, Name: "gitlab.corp.io"}); err == nil {
		t.Fatal("expected duplicate name error")
	}
	cfg.SetRepoPath("gitlab.corp.io", "grp/sub/proj", "~/code/proj")
	if err := cfg.Save(); err != nil {
		t.Fatal(err)
	}
	got, created, err := Load()
	if err != nil || created {
		t.Fatalf("reload created=%v err=%v", created, err)
	}
	in, ok := got.Instance("gitlab.corp.io")
	if !ok || in.Host != "gitlab.corp.io" || in.Provider != GitLab {
		t.Fatalf("instance = %+v", in)
	}
	if r, ok := got.Repo("gitlab.corp.io", "GRP/sub/proj"); !ok || r.Path != "~/code/proj" {
		t.Fatalf("repo = %+v", r)
	}
	if got.Interval() != 5*time.Minute {
		t.Fatalf("default interval = %v", got.Interval())
	}
	// Renaming an instance carries its repo mappings along.
	if err := got.UpsertInstance("gitlab.corp.io", Instance{Name: "work", Provider: GitLab, Host: "gitlab.corp.io"}); err != nil {
		t.Fatal(err)
	}
	if _, ok := got.Repo("work", "grp/sub/proj"); !ok {
		t.Fatal("repo mapping not renamed")
	}
	got.RemoveInstance("work")
	if len(got.Repos) != 0 {
		t.Fatal("repo mappings not removed with instance")
	}
}

func TestValidate(t *testing.T) {
	c := Default()
	c.RefreshInterval = "soon"
	if c.Validate() == nil {
		t.Fatal("expected invalid interval")
	}
	c = Default()
	c.RefreshInterval = "1s"
	if c.Interval() != 15*time.Second {
		t.Fatalf("interval floor = %v", c.Interval())
	}
}

func TestTrackAllRepoPersistsWithoutLocalPath(t *testing.T) {
	p := filepath.Join(t.TempDir(), "config.toml")
	t.Setenv("PR_TRACKER_CONFIG", p)
	cfg := Default()
	cfg.SetRepoTrackAll("work", "group/project", true)
	if err := cfg.Save(); err != nil {
		t.Fatal(err)
	}

	got, _, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	repo, ok := got.Repo("work", "GROUP/project")
	if !ok || !repo.TrackAll || repo.Path != "" {
		t.Fatalf("repo = %+v, found = %v", repo, ok)
	}

	got.SetRepoPath("work", "group/project", "")
	if repo, ok = got.Repo("work", "group/project"); !ok || !repo.TrackAll {
		t.Fatalf("clearing a path removed tracked repo: %+v, found = %v", repo, ok)
	}
	got.SetRepoTrackAll("work", "group/project", false)
	if _, ok := got.Repo("work", "group/project"); ok {
		t.Fatal("otherwise empty repo was not removed after disabling track_all")
	}
}

func TestLanguagePreferenceRoundTrip(t *testing.T) {
	t.Setenv("PR_TRACKER_CONFIG", filepath.Join(t.TempDir(), "config.toml"))
	cfg, _, err := Load()
	if err != nil || cfg.Language != "system" {
		t.Fatalf("defaults = %+v, %v", cfg, err)
	}
	for _, language := range []string{"pt", "en", "system"} {
		cfg.Language = language
		if err := cfg.Save(); err != nil {
			t.Fatal(err)
		}
		loaded, _, err := Load()
		if err != nil || loaded.Language != language {
			t.Fatalf("language = %+v, %v", loaded, err)
		}
	}
	cfg.Language = "fr"
	if err := cfg.Validate(); err == nil {
		t.Fatal("unsupported language accepted")
	}
	if err := os.WriteFile(cfg.FilePath(), []byte("refresh_interval = \"5m\"\nterminal = \"auto\"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	cfg, _, err = Load()
	if err != nil || cfg.Language != "system" {
		t.Fatalf("legacy config = %+v, %v", cfg, err)
	}
}

func TestThemePathAndPersistence(t *testing.T) {
	path := filepath.Join(t.TempDir(), "config.toml")
	t.Setenv("PR_TRACKER_CONFIG", path)
	cfg, _, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	cfg.ThemeFile = "colors.toml"
	if cfg.ThemePath() != filepath.Join(filepath.Dir(path), "colors.toml") {
		t.Fatal(cfg.ThemePath())
	}
	if err := cfg.Save(); err != nil {
		t.Fatal(err)
	}
	loaded, _, err := Load()
	if err != nil || loaded.ThemeFile != cfg.ThemeFile {
		t.Fatalf("persistence: %+v %v", loaded, err)
	}
}
