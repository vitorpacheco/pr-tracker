// Package theme reads the desktop palette shared by the native interfaces.
package theme

import (
	"os"
	"path/filepath"
	"runtime"
)

// Palette contains validated colors; the zero value means no system palette.
type Palette struct {
	Name            string            `toml:"-"`
	Mode            string            `toml:"mode"`
	Background      string            `toml:"background"`
	Panel           string            `toml:"panel"`
	Raised          string            `toml:"raised"`
	Border          string            `toml:"border"`
	Muted           string            `toml:"muted_text"`
	Foreground      string            `toml:"foreground"`
	Accent          string            `toml:"accent"`
	Selection       string            `toml:"selection"`
	SelectionText   string            `toml:"selection_foreground"`
	Red             string            `toml:"red"`
	Green           string            `toml:"green"`
	Yellow          string            `toml:"yellow"`
	Blue            string            `toml:"blue"`
	AccentAlt       string            `toml:"accent_alt"`
	Dim             string            `toml:"dim"`
	Heading         string            `toml:"heading"`
	LabelBackground string            `toml:"label_background"`
	TitleText       string            `toml:"title_text"`
	GUI             map[string]string `toml:"gui,omitempty"`
}

// Omarchy reads the generated current theme, including older installations.
// A current Omarchy palette is the detection signal; merely running Hyprland
// does not enable this integration. Never execute theme files or shell hooks.
func Omarchy() Palette {
	if runtime.GOOS != "linux" {
		return Palette{}
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return Palette{}
	}
	state := os.Getenv("XDG_STATE_HOME")
	if state == "" {
		state = filepath.Join(home, ".local", "state")
	}
	config := os.Getenv("XDG_CONFIG_HOME")
	if config == "" {
		config = filepath.Join(home, ".config")
	}
	for _, root := range []string{state, config} {
		current := filepath.Join(root, "omarchy", "current")
		data, err := os.ReadFile(filepath.Join(current, "theme", "colors.toml"))
		if os.IsNotExist(err) {
			continue
		}
		if err != nil {
			return Palette{}
		}
		return parse(data)
	}
	return Palette{}
}
