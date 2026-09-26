package ui

import (
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/glamour/v2"
	"charm.land/lipgloss/v2"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/theme"
)

func TestThemeChangePreservesStateAndInvalidatesMarkdown(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	m := New(config.Default(), nil)
	defer m.Close()
	other := New(config.Default(), nil)
	defer other.Close()
	m.selected = "selected-item"
	m.filter.SetValue("search")
	m.thread = &threadView{lines: []string{"old colors"}, thread: &provider.Thread{Body: "# Heading\n\nText and `code`."}}
	p := theme.Palette{Name: "Omarchy", Mode: "light", Background: "#faf4ed", Foreground: "#575279", Accent: "#56949f", Muted: "#6e6a86", Border: "#cecacd", Raised: "#f2e9e1", Selection: "#dfdad9", SelectionText: "#575279", Red: "#b4637a", Green: "#286983", Yellow: "#ea9d34", Blue: "#56949f"}
	m.Update(paletteMsg(p))
	m.Update(tea.WindowSizeMsg{Width: 100, Height: 30})
	v := m.View()
	if v.BackgroundColor != lipgloss.Color(p.Background) || v.ForegroundColor != lipgloss.Color(p.Foreground) {
		t.Fatal("view did not adopt system colors")
	}
	if m.selected != "selected-item" || m.filter.Value() != "search" || m.thread.lines != nil {
		t.Fatal("theme update lost presentation state or retained old Markdown")
	}
	if other.styles.palette.Name != "" {
		t.Fatal("theme leaked into another model")
	}
	r, err := glamour.NewTermRenderer(m.styles.markdownStyle(), glamour.WithWordWrap(80))
	if err != nil {
		t.Fatal(err)
	}
	lines := renderMarkdown(r, m.thread.thread.Body, 80)
	if len(lines) == 0 || !strings.Contains(strings.Join(lines, "\n"), "Heading") {
		t.Fatal("themed Markdown did not render")
	}
	m.Update(paletteMsg(theme.Palette{}))
	if m.View().BackgroundColor != nil {
		t.Fatal("missing palette did not restore terminal defaults")
	}
}

func TestThemeExportKeepsUnsavedSettings(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	m := New(config.Default(), nil)
	defer m.Close()
	m.openSettingsForm()
	settings := m.form
	settings.get("Arquivo de cores").input.SetValue("unsaved.toml")
	m.formResult(formExport)
	destination := filepath.Join(t.TempDir(), "colors.toml")
	m.form.get("Destino").input.SetValue(destination)
	m.formResult(formSubmit)
	p, err := theme.Resolve(destination)
	if err != nil || p.Accent != theme.DefaultTUI().Accent {
		t.Fatalf("export: %+v %v", p, err)
	}
	if m.form != settings || settings.get("Arquivo de cores").value() != "unsaved.toml" {
		t.Fatal("lost unsaved settings")
	}
	m.formResult(formSubmit)
	if m.cfg.ThemeFile != "" || m.form.err == "" {
		t.Fatal("invalid theme persisted")
	}
}
