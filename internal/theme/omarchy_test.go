package theme

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestPaletteValidationAndFallbacks(t *testing.T) {
	for _, tc := range []struct {
		name, data, mode string
		valid            bool
	}{
		{"dark", `background="#121212"
foreground="#bebebe"
accent="#e68e0d"
red="#D35F5F"`, "dark", true},
		{"light legacy", `background="#faf4ed"
foreground="#575279"
accent="#56949f"
color1="#b4637a"`, "light", true},
		{"invalid core", `background="red"
foreground="#ffffff"
accent="#abcdef"`, "", false},
		{"malformed", `background=[`, "", false},
		{"missing", `accent="#abcdef"`, "", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := parse([]byte(tc.data))
			if (p.Name != "") != tc.valid || p.Mode != tc.mode {
				t.Fatalf("unexpected palette: %+v", p)
			}
			if tc.valid && (p.Red == p.Accent || p.Muted != p.Foreground) {
				t.Fatalf("mapping/fallback: %+v", p)
			}
		})
	}
}

func TestOmarchyPathsAndThemeChanges(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("Linux integration")
	}
	root := t.TempDir()
	t.Setenv("HOME", root)
	t.Setenv("XDG_STATE_HOME", filepath.Join(root, "state"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "config"))
	if p := Omarchy(); p.Name != "" {
		t.Fatal("detected Omarchy without a palette")
	}
	write := func(root, accent string) {
		t.Helper()
		path := filepath.Join(root, "omarchy/current/theme")
		if err := os.MkdirAll(path, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(path, "colors.toml"), []byte("background=\"#121212\"\nforeground=\"#bebebe\"\naccent=\""+accent+"\"\n"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	write(os.Getenv("XDG_CONFIG_HOME"), "#112233")
	if p := Omarchy(); p.Accent != "#112233" {
		t.Fatalf("legacy path: %+v", p)
	}
	write(os.Getenv("XDG_STATE_HOME"), "#445566")
	if p := Omarchy(); p.Accent != "#445566" {
		t.Fatalf("state precedence: %+v", p)
	}
	write(os.Getenv("XDG_STATE_HOME"), "#778899")
	if p := Omarchy(); p.Accent != "#778899" {
		t.Fatalf("theme change: %+v", p)
	}
	write(os.Getenv("XDG_STATE_HOME"), "invalid")
	if p := Omarchy(); p.Name != "" {
		t.Fatalf("invalid theme must fall back: %+v", p)
	}
}
