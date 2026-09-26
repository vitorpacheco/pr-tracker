package theme

import (
	"bytes"
	"os"
	"reflect"
	"regexp"
	"strconv"

	"github.com/BurntSushi/toml"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
)

var hexColor = regexp.MustCompile(`^#[0-9a-fA-F]{6}$`)
var cssColor = regexp.MustCompile(`^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$`)
var cssKey = regexp.MustCompile(`^[a-z][a-z0-9_-]{0,63}$`)

func (p Palette) Equal(other Palette) bool { return reflect.DeepEqual(p, other) }

// Resolve gives an explicit file priority over the auto-detected system palette.
// A custom file failure is reported, never silently replaced with another theme.
func Resolve(path string) (Palette, error) {
	if path == "" {
		return Omarchy(), nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return Palette{}, i18n.Errorf("lendo esquema de cores %q: %w", path, err)
	}
	p, err := decode(data)
	if err != nil {
		return Palette{}, i18n.Errorf("esquema de cores inválido %q: %w", path, err)
	}
	p.Name = "Custom"
	return p, nil
}

func parse(data []byte) Palette {
	p, err := decode(data)
	if err != nil {
		return Palette{}
	}
	p.Name = "Omarchy"
	return p
}

func decode(data []byte) (Palette, error) {
	var c map[string]any
	if _, err := toml.Decode(string(data), &c); err != nil {
		return Palette{}, err
	}
	value := func(key string) string { s, _ := c[key].(string); return s }
	for _, key := range []string{"background", "foreground", "accent"} {
		if !hexColor.MatchString(value(key)) {
			return Palette{}, i18n.Errorf("cor inválida em %s: use #RRGGBB", key)
		}
	}
	var invalid string
	color := func(fallback string, keys ...string) string {
		for _, key := range keys {
			if _, ok := c[key]; ok {
				if !hexColor.MatchString(value(key)) {
					invalid = key
					return fallback
				}
				return value(key)
			}
		}
		return fallback
	}
	mode := value("mode")
	if mode == "" {
		n, _ := strconv.ParseUint(value("background")[1:], 16, 32)
		mode = "dark"
		if (299*((n>>16)&255)+587*((n>>8)&255)+114*(n&255))/1000 > 127 {
			mode = "light"
		}
	}
	if mode != "dark" && mode != "light" {
		return Palette{}, i18n.Errorf("modo de cores inválido: use light ou dark")
	}
	p := Palette{
		Mode: mode, Background: value("background"), Foreground: value("foreground"), Accent: value("accent"),
		Panel: color(value("background"), "panel", "dark_background"), Raised: color(value("background"), "raised", "lighter_background", "color0"),
		Border: color(value("foreground"), "border", "muted", "color8"), Muted: color(value("foreground"), "muted_text", "light_foreground", "color7"),
		Selection: color(value("accent"), "selection", "selection_background"), SelectionText: color(value("foreground"), "selection_foreground"),
		Red: color(value("accent"), "red", "color1"), Green: color(value("accent"), "green", "color2"), Yellow: color(value("accent"), "yellow", "color3"), Blue: color(value("accent"), "blue", "color4"),
		AccentAlt: color(value("accent"), "accent_alt"), Heading: color(value("accent"), "heading"), TitleText: color(value("background"), "title_text"),
	}
	p.Dim = color(p.Muted, "dim")
	p.LabelBackground = color(p.Selection, "label_background")
	if invalid != "" {
		return Palette{}, i18n.Errorf("cor inválida em %s: use #RRGGBB", invalid)
	}
	if raw, ok := c["gui"]; ok {
		table, ok := raw.(map[string]any)
		if !ok {
			return Palette{}, i18n.Errorf("tabela gui inválida")
		}
		p.GUI = map[string]string{}
		for key, raw := range table {
			val, ok := raw.(string)
			if !ok || !cssKey.MatchString(key) || !cssColor.MatchString(val) {
				return Palette{}, i18n.Errorf("cor inválida em gui.%s", key)
			}
			p.GUI[key] = val
		}
	}
	return p, nil
}

// Export writes a reusable snapshot. Exclusive creation protects existing themes
// and configuration files even if the destination is a symbolic link.
func Export(path string, p Palette) error {
	var buf bytes.Buffer
	if err := toml.NewEncoder(&buf).Encode(p); err != nil {
		return err
	}
	if _, err := decode(buf.Bytes()); err != nil {
		return err
	}
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if os.IsExist(err) {
		return i18n.Errorf("o arquivo já existe; escolha outro destino: %s", path)
	}
	if err != nil {
		return i18n.Errorf("exportando esquema de cores: %w", err)
	}
	_, writeErr := f.Write(buf.Bytes())
	closeErr := f.Close()
	if writeErr != nil {
		return i18n.Errorf("exportando esquema de cores: %w", writeErr)
	}
	return closeErr
}

// DefaultTUI describes the terminal interface's built-in palette.
func DefaultTUI() Palette {
	return Palette{Name: "Default", Mode: "dark", Background: "#111111", Panel: "#111111", Raised: "#2E2A47", Border: "#4B5263", Muted: "#7C8594", Foreground: "#E5E7EB", Accent: "#8B5CF6", AccentAlt: "#A78BFA", Selection: "#2E2A47", SelectionText: "#E5E7EB", Red: "#EF4444", Green: "#22C55E", Yellow: "#EAB308", Blue: "#38BDF8", Dim: "#4B5263", Heading: "#8B5CF6", LabelBackground: "#2A2540", TitleText: "#111111"}
}
