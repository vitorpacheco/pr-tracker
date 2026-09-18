package config

import (
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
