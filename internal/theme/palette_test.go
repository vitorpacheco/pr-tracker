package theme

import (
	"os"
	"path/filepath"
	"testing"
)

func TestExportAndReload(t *testing.T) {
	for _, original := range []Palette{DefaultTUI(), parse([]byte("background=\"#ffffff\"\nforeground=\"#111111\"\naccent=\"#445566\""))} {
		path := filepath.Join(t.TempDir(), "colors.toml")
		original.GUI = map[string]string{"shadow": "#0007"}
		if err := Export(path, original); err != nil {
			t.Fatal(err)
		}
		loaded, err := Resolve(path)
		if err != nil {
			t.Fatal(err)
		}
		original.Name = "Custom"
		if !loaded.Equal(original) {
			t.Fatalf("round trip: %+v != %+v", loaded, original)
		}
		if err := Export(path, DefaultTUI()); err == nil {
			t.Fatal("overwrote existing palette")
		}
		after, _ := Resolve(path)
		if !after.Equal(loaded) {
			t.Fatal("existing file changed")
		}
	}
}

func TestCustomPaletteValidationAndReload(t *testing.T) {
	path := filepath.Join(t.TempDir(), "colors.toml")
	if _, err := Resolve(path); err == nil {
		t.Fatal("missing file accepted")
	}
	const base = "background=\"#ffffff\"\nforeground=\"#111111\"\naccent=\"#445566\"\n"
	for _, invalid := range []string{"mode=\"invalid\"", "red=\"red\"", "panel=123", "[gui]\nshadow=\"url(file)\""} {
		if err := os.WriteFile(path, []byte(base+invalid), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := Resolve(path); err == nil {
			t.Fatalf("accepted %s", invalid)
		}
	}
	if err := os.WriteFile(path, []byte(base), 0600); err != nil {
		t.Fatal(err)
	}
	if p, err := Resolve(path); err != nil || p.Name != "Custom" || p.Mode != "light" {
		t.Fatalf("custom: %+v %v", p, err)
	}
	if err := os.WriteFile(path, []byte(base+"red=\"#ff0000\""), 0600); err != nil {
		t.Fatal(err)
	}
	if p, _ := Resolve(path); p.Red != "#ff0000" {
		t.Fatal("file changes not adopted")
	}
}
