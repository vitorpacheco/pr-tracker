//! Turns the configured palette, or the built-in light and dark ones, into the
//! colours the window draws with.

use std::collections::BTreeMap;

use pr_tracker_core::theme::Palette;
use rune::Color;
use rune::color::hex;
use rune_kit::{Kit, PaletteSeed};

/// The appearance chosen in settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePreference {
    pub const ALL: [ThemePreference; 3] = [
        ThemePreference::System,
        ThemePreference::Dark,
        ThemePreference::Light,
    ];

    pub fn parse(value: &str) -> ThemePreference {
        match value {
            "dark" => ThemePreference::Dark,
            "light" => ThemePreference::Light,
            _ => ThemePreference::System,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePreference::System => "system",
            ThemePreference::Dark => "dark",
            ThemePreference::Light => "light",
        }
    }

    /// The label shown in settings; a catalog key.
    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "Sistema",
            ThemePreference::Dark => "Escuro",
            ThemePreference::Light => "Claro",
        }
    }
}

/// The interface's own colour roles, as hex strings so they can be exported.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub dark: bool,
    pub background: &'static str,
    pub panel: &'static str,
    pub raised: &'static str,
    pub border: &'static str,
    pub muted: &'static str,
    pub foreground: &'static str,
    pub accent: &'static str,
    pub selection: &'static str,
    pub green: &'static str,
    pub red: &'static str,
    pub yellow: &'static str,
    pub blue: &'static str,
}

pub const DARK: Scheme = Scheme {
    dark: true,
    background: "#0d131b",
    panel: "#111a24",
    raised: "#192431",
    border: "#263342",
    muted: "#90a3b8",
    foreground: "#e4ecf5",
    accent: "#60adff",
    selection: "#142e4d",
    green: "#5bddab",
    red: "#ff858c",
    yellow: "#eab308",
    blue: "#38bdf8",
};

pub const LIGHT: Scheme = Scheme {
    dark: false,
    background: "#f6f8fc",
    panel: "#ffffff",
    raised: "#edf1f8",
    border: "#d9e1eb",
    muted: "#51677d",
    foreground: "#172b43",
    accent: "#175fa8",
    selection: "#e0efff",
    green: "#197c55",
    red: "#b8243f",
    yellow: "#9a6700",
    blue: "#0b6ea8",
};

/// What a view draws with: the component kit and the interface's own roles.
#[derive(Clone)]
pub struct Look {
    pub kit: Kit,
    pub background: Color,
    pub panel: Color,
    pub raised: Color,
    pub border: Color,
    pub muted: Color,
    pub text: Color,
    pub accent: Color,
    pub selection: Color,
    pub selection_text: Color,
    pub green: Color,
    pub red: Color,
    pub yellow: Color,
    pub blue: Color,
    /// What lifted surfaces cast: black, stronger on a dark background.
    pub shadow: Color,
}

/// A custom palette always applies; the system one only while the preference
/// follows the system.
fn active(preference: ThemePreference, palette: Option<&Palette>) -> Option<&Palette> {
    palette.filter(|palette| palette.name == "Custom" || preference == ThemePreference::System)
}

fn scheme(preference: ThemePreference, system_dark: bool) -> &'static Scheme {
    match preference {
        ThemePreference::Dark => &DARK,
        ThemePreference::Light => &LIGHT,
        ThemePreference::System if system_dark => &DARK,
        ThemePreference::System => &LIGHT,
    }
}

fn color(value: &str, fallback: Color) -> Color {
    hex(value).unwrap_or(fallback)
}

pub fn look(preference: ThemePreference, palette: Option<&Palette>, system_dark: bool) -> Look {
    let palette = displayed(preference, palette, system_dark);
    let background = color(&palette.background, Color::BLACK);
    let text = color(&palette.foreground, Color::WHITE);
    let accent = color(&palette.accent, text);
    let mut kit = Kit::from_palette(PaletteSeed {
        bg: background,
        fg: text,
        accent,
    });
    let (panel, raised) = (
        color(&palette.panel, background),
        color(&palette.raised, background),
    );
    let (border, muted) = (color(&palette.border, text), color(&palette.muted, text));
    let (green, red) = (color(&palette.green, accent), color(&palette.red, accent));
    let (yellow, blue) = (color(&palette.yellow, accent), color(&palette.blue, accent));
    kit.c.page = background;
    kit.c.bg_base = background;
    kit.c.layer_01 = panel;
    kit.c.layer_02 = raised;
    kit.c.text_muted = muted;
    kit.c.icon_muted = muted;
    kit.c.border_base = border;
    kit.c.border_muted = border.alpha(0.6);
    for (state, ink) in [
        (&mut kit.c.success, green),
        (&mut kit.c.danger, red),
        (&mut kit.c.warning, yellow),
        (&mut kit.c.info, blue),
    ] {
        state.fg = ink;
        state.icon = ink;
        state.border = ink.alpha(0.45);
        state.bg = ink.alpha(0.12);
    }
    let shadow = Color::BLACK.alpha(if kit.dark { 0.42 } else { 0.1 });
    Look {
        shadow,
        kit,
        background,
        panel,
        raised,
        border,
        muted,
        text,
        accent,
        selection: color(&palette.selection, raised),
        selection_text: color(&palette.selection_text, text),
        green,
        red,
        yellow,
        blue,
    }
}

/// The palette on screen, in the shape that can be exported and loaded back.
pub fn displayed(
    preference: ThemePreference,
    palette: Option<&Palette>,
    system_dark: bool,
) -> Palette {
    if let Some(palette) = active(preference, palette) {
        return Palette {
            name: "Custom".into(),
            ..palette.clone()
        };
    }
    let scheme = scheme(preference, system_dark);
    Palette {
        name: "Custom".into(),
        mode: if scheme.dark { "dark" } else { "light" }.into(),
        background: scheme.background.into(),
        panel: scheme.panel.into(),
        raised: scheme.raised.into(),
        border: scheme.border.into(),
        muted: scheme.muted.into(),
        foreground: scheme.foreground.into(),
        accent: scheme.accent.into(),
        selection: scheme.selection.into(),
        selection_text: scheme.foreground.into(),
        red: scheme.red.into(),
        green: scheme.green.into(),
        yellow: scheme.yellow.into(),
        blue: scheme.blue.into(),
        accent_alt: scheme.accent.into(),
        dim: scheme.muted.into(),
        heading: scheme.foreground.into(),
        label_background: scheme.selection.into(),
        title_text: scheme.background.into(),
        gui: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pr_tracker_core::theme;

    #[test]
    fn manual_preference_overrides_the_system_palette_but_not_a_custom_one() {
        let omarchy = Palette {
            name: "Omarchy".into(),
            ..theme::default_dark()
        };
        assert_eq!(
            displayed(ThemePreference::System, Some(&omarchy), false).background,
            omarchy.background
        );
        assert_eq!(
            displayed(ThemePreference::Light, Some(&omarchy), true).background,
            LIGHT.background
        );
        let custom = Palette {
            name: "Custom".into(),
            ..theme::default_dark()
        };
        assert_eq!(
            displayed(ThemePreference::Light, Some(&custom), false).background,
            custom.background
        );
    }

    #[test]
    fn built_in_schemes_follow_the_preference_then_the_system() {
        assert_eq!(displayed(ThemePreference::System, None, true).mode, "dark");
        assert_eq!(
            displayed(ThemePreference::System, None, false).mode,
            "light"
        );
        assert_eq!(
            displayed(ThemePreference::Dark, None, false).background,
            DARK.background
        );
    }

    #[test]
    fn the_displayed_palette_exports_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        let shown = displayed(ThemePreference::Light, None, true);
        theme::export(&path, &shown).unwrap();
        assert_eq!(theme::resolve(Some(&path)).unwrap(), Some(shown));
    }

    #[test]
    fn look_maps_palette_roles_onto_the_kit() {
        let look = look(ThemePreference::Dark, None, false);
        assert!(look.kit.dark);
        assert_eq!(look.kit.c.layer_01, hex(DARK.panel).unwrap());
        assert_eq!(look.kit.c.success.fg, hex(DARK.green).unwrap());
    }
}
