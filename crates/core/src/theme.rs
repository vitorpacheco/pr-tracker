//! Reads the desktop palette shared by the native interfaces.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::msg;

static HEX_COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#[0-9a-fA-F]{6}$").expect("constant pattern"));
static CSS_COLOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$")
        .expect("constant pattern")
});
static CSS_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_-]{0,63}$").expect("constant pattern"));

/// Validated colors. The field order is the key order of an exported file.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Palette {
    #[serde(skip)]
    pub name: String,
    pub mode: String,
    pub background: String,
    pub panel: String,
    pub raised: String,
    pub border: String,
    #[serde(rename = "muted_text")]
    pub muted: String,
    pub foreground: String,
    pub accent: String,
    pub selection: String,
    #[serde(rename = "selection_foreground")]
    pub selection_text: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub accent_alt: String,
    pub dim: String,
    pub heading: String,
    pub label_background: String,
    pub title_text: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub gui: BTreeMap<String, String>,
}

/// Reads the generated current theme, including older installations. A current
/// Omarchy palette is the detection signal; merely running Hyprland does not
/// enable this integration. Never execute theme files or shell hooks.
pub fn omarchy() -> Option<Palette> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let home = crate::config::home_dir().ok()?;
    let root = |variable: &str, fallback: &[&str]| {
        std::env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                fallback
                    .iter()
                    .fold(home.clone(), |path, part| path.join(part))
            })
    };
    let state = root("XDG_STATE_HOME", &[".local", "state"]);
    let config = root("XDG_CONFIG_HOME", &[".config"]);
    for root in [state, config] {
        let colors = root
            .join("omarchy")
            .join("current")
            .join("theme")
            .join("colors.toml");
        match std::fs::read(colors) {
            Ok(data) => return parse(&data),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Gives an explicit file priority over the auto-detected system palette. A
/// custom file failure is reported, never silently replaced with another theme.
pub fn resolve(path: Option<&Path>) -> Result<Option<Palette>> {
    let Some(path) = path else {
        return Ok(omarchy());
    };
    let shown = path.display().to_string();
    let data = std::fs::read(path)
        .map_err(|err| msg!("lendo esquema de cores %q: %w", &shown, Error::from(err)))?;
    let mut palette =
        decode(&data).map_err(|err| msg!("esquema de cores inválido %q: %w", &shown, err))?;
    palette.name = "Custom".into();
    Ok(Some(palette))
}

fn parse(data: &[u8]) -> Option<Palette> {
    let mut palette = decode(data).ok()?;
    palette.name = "Omarchy".into();
    Some(palette)
}

fn decode(data: &[u8]) -> Result<Palette> {
    let text = std::str::from_utf8(data).map_err(|err| Error::external(err.to_string()))?;
    let table: toml::Table = text
        .parse()
        .map_err(|err: toml::de::Error| Error::external(err.to_string()))?;
    let value = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    let (background, foreground, accent) =
        (value("background"), value("foreground"), value("accent"));
    for (key, core) in [
        ("background", &background),
        ("foreground", &foreground),
        ("accent", &accent),
    ] {
        if !HEX_COLOR.is_match(core) {
            return Err(msg!("cor inválida em %s: use #RRGGBB", key));
        }
    }

    let mut mode = value("mode");
    if mode.is_empty() {
        let rgb = u32::from_str_radix(&background[1..], 16).unwrap_or_default();
        let luma =
            (299 * ((rgb >> 16) & 255) + 587 * ((rgb >> 8) & 255) + 114 * (rgb & 255)) / 1000;
        mode = if luma > 127 { "light" } else { "dark" }.into();
    }
    if mode != "dark" && mode != "light" {
        return Err(msg!("modo de cores inválido: use light ou dark"));
    }

    // The first key present decides a colour, so a newer name shadows the
    // legacy ones even when its value is wrong; that is reported, not skipped.
    let mut invalid = None;
    let mut color = |fallback: &str, keys: &[&'static str]| -> String {
        for &key in keys {
            if let Some(raw) = table.get(key) {
                return match raw.as_str() {
                    Some(hex) if HEX_COLOR.is_match(hex) => hex.to_string(),
                    _ => {
                        invalid = Some(key);
                        fallback.to_string()
                    }
                };
            }
        }
        fallback.to_string()
    };
    let mut palette = Palette {
        panel: color(&background, &["panel", "dark_background"]),
        raised: color(&background, &["raised", "lighter_background", "color0"]),
        border: color(&foreground, &["border", "muted", "color8"]),
        muted: color(&foreground, &["muted_text", "light_foreground", "color7"]),
        selection: color(&accent, &["selection", "selection_background"]),
        selection_text: color(&foreground, &["selection_foreground"]),
        red: color(&accent, &["red", "color1"]),
        green: color(&accent, &["green", "color2"]),
        yellow: color(&accent, &["yellow", "color3"]),
        blue: color(&accent, &["blue", "color4"]),
        accent_alt: color(&accent, &["accent_alt"]),
        heading: color(&accent, &["heading"]),
        title_text: color(&background, &["title_text"]),
        ..Palette::default()
    };
    palette.dim = color(&palette.muted, &["dim"]);
    palette.label_background = color(&palette.selection, &["label_background"]);
    if let Some(key) = invalid {
        return Err(msg!("cor inválida em %s: use #RRGGBB", key));
    }
    palette.mode = mode;
    palette.background = background;
    palette.foreground = foreground;
    palette.accent = accent;

    if let Some(raw) = table.get("gui") {
        let gui = raw.as_table().ok_or_else(|| msg!("tabela gui inválida"))?;
        for (key, raw) in gui {
            match raw.as_str() {
                Some(color) if CSS_KEY.is_match(key) && CSS_COLOR.is_match(color) => {
                    palette.gui.insert(key.clone(), color.to_string());
                }
                _ => return Err(msg!("cor inválida em gui.%s", key)),
            }
        }
    }
    Ok(palette)
}

/// Writes a reusable snapshot. Exclusive creation protects existing themes and
/// configuration files even if the destination is a symbolic link.
pub fn export(path: &Path, palette: &Palette) -> Result<()> {
    let text = toml::to_string(palette).map_err(|err| Error::external(err.to_string()))?;
    // A file that could not be loaded back must never be written.
    decode(text.as_bytes())?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(msg!(
                "o arquivo já existe; escolha outro destino: %s",
                path.display().to_string()
            ));
        }
        Err(err) => return Err(msg!("exportando esquema de cores: %w", Error::from(err))),
    };
    file.write_all(text.as_bytes())
        .map_err(|err| msg!("exportando esquema de cores: %w", Error::from(err)))
}

/// The terminal interface's built-in palette.
pub fn default_dark() -> Palette {
    Palette {
        name: "Default".into(),
        mode: "dark".into(),
        background: "#111111".into(),
        panel: "#111111".into(),
        raised: "#2E2A47".into(),
        border: "#4B5263".into(),
        muted: "#7C8594".into(),
        foreground: "#E5E7EB".into(),
        accent: "#8B5CF6".into(),
        accent_alt: "#A78BFA".into(),
        selection: "#2E2A47".into(),
        selection_text: "#E5E7EB".into(),
        red: "#EF4444".into(),
        green: "#22C55E".into(),
        yellow: "#EAB308".into(),
        blue: "#38BDF8".into(),
        dim: "#4B5263".into(),
        heading: "#8B5CF6".into(),
        label_background: "#2A2540".into(),
        title_text: "#111111".into(),
        gui: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "background=\"#ffffff\"\nforeground=\"#111111\"\naccent=\"#445566\"\n";

    #[test]
    fn export_and_reload() {
        for mut original in [default_dark(), parse(BASE.as_bytes()).unwrap()] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("colors.toml");
            original.gui = BTreeMap::from([("shadow".to_string(), "#0007".to_string())]);
            export(&path, &original).unwrap();
            let loaded = resolve(Some(&path)).unwrap().unwrap();
            original.name = "Custom".into();
            assert_eq!(loaded, original, "round trip");

            let err = export(&path, &default_dark()).expect_err("overwrote existing palette");
            assert!(
                err.is("o arquivo já existe; escolha outro destino: %s"),
                "{err}"
            );
            assert_eq!(
                resolve(Some(&path)).unwrap().unwrap(),
                loaded,
                "existing file changed"
            );
        }
    }

    #[test]
    fn export_writes_the_documented_keys_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        let mut palette = default_dark();
        palette.gui.insert("shadow".into(), "#0007".into());
        export(&path, &palette).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let keys: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once(" = ").map(|(key, _)| key.trim()))
            .collect();
        let want = [
            "mode",
            "background",
            "panel",
            "raised",
            "border",
            "muted_text",
            "foreground",
            "accent",
            "selection",
            "selection_foreground",
            "red",
            "green",
            "yellow",
            "blue",
            "accent_alt",
            "dim",
            "heading",
            "label_background",
            "title_text",
            "shadow",
        ];
        assert_eq!(keys, want, "{text}");
        assert!(text.contains("\n[gui]\nshadow = \"#0007\"\n"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let unloadable = Palette {
            background: "red".into(),
            ..default_dark()
        };
        let other = dir.path().join("invalid.toml");
        assert!(
            export(&other, &unloadable).is_err(),
            "exported an invalid palette"
        );
        assert!(!other.exists(), "invalid palette left a file behind");
    }

    #[test]
    fn custom_palette_validation_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        let err = resolve(Some(&path)).expect_err("missing file accepted");
        assert!(err.is("lendo esquema de cores %q: %w"), "{err}");

        for (invalid, cause) in [
            (
                "mode=\"invalid\"",
                "modo de cores inválido: use light ou dark",
            ),
            ("red=\"red\"", "cor inválida em %s: use #RRGGBB"),
            ("panel=123", "cor inválida em %s: use #RRGGBB"),
            ("[gui]\nshadow=\"url(file)\"", "cor inválida em gui.%s"),
            ("gui=\"#000000\"", "tabela gui inválida"),
        ] {
            std::fs::write(&path, format!("{BASE}{invalid}")).unwrap();
            let err = resolve(Some(&path)).expect_err(invalid);
            assert!(
                err.is("esquema de cores inválido %q: %w") && err.is(cause),
                "{invalid}: {err}"
            );
        }

        std::fs::write(&path, BASE).unwrap();
        let palette = resolve(Some(&path)).unwrap().unwrap();
        assert_eq!(
            (palette.name.as_str(), palette.mode.as_str()),
            ("Custom", "light")
        );

        std::fs::write(&path, format!("{BASE}red=\"#ff0000\"")).unwrap();
        assert_eq!(
            resolve(Some(&path)).unwrap().unwrap().red,
            "#ff0000",
            "file changes not adopted"
        );
    }

    #[test]
    fn palette_validation_and_fallbacks() {
        let cases = [
            (
                "dark",
                "background=\"#121212\"\nforeground=\"#bebebe\"\naccent=\"#e68e0d\"\nred=\"#D35F5F\"",
                Some("dark"),
            ),
            (
                "light legacy",
                "background=\"#faf4ed\"\nforeground=\"#575279\"\naccent=\"#56949f\"\ncolor1=\"#b4637a\"",
                Some("light"),
            ),
            (
                "invalid core",
                "background=\"red\"\nforeground=\"#ffffff\"\naccent=\"#abcdef\"",
                None,
            ),
            ("malformed", "background=[", None),
            ("missing", "accent=\"#abcdef\"", None),
        ];
        for (name, data, mode) in cases {
            let palette = parse(data.as_bytes());
            assert_eq!(
                palette.as_ref().map(|palette| palette.mode.as_str()),
                mode,
                "{name}: {palette:?}"
            );
            if let Some(palette) = palette {
                assert_eq!(palette.name, "Omarchy", "{name}");
                assert!(
                    palette.red != palette.accent && palette.muted == palette.foreground,
                    "{name} mapping/fallback: {palette:?}"
                );
            }
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn omarchy_paths_and_theme_changes() {
        use std::ffi::OsString;
        use std::path::PathBuf;

        /// Restores the variables this test replaces.
        struct Restore(Vec<(&'static str, Option<OsString>)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (key, previous) in self.0.drain(..) {
                    // SAFETY: the test still holds env_lock.
                    unsafe {
                        match previous {
                            Some(value) => std::env::set_var(key, value),
                            None => std::env::remove_var(key),
                        }
                    }
                }
            }
        }

        let _guard = crate::i18n::tests::env_lock();
        let root = tempfile::tempdir().unwrap();
        let (state, config) = (root.path().join("state"), root.path().join("config"));
        let variables: [(&'static str, PathBuf); 3] = [
            ("HOME", root.path().to_path_buf()),
            ("XDG_STATE_HOME", state.clone()),
            ("XDG_CONFIG_HOME", config.clone()),
        ];
        let _restore = Restore(
            variables
                .iter()
                .map(|(key, _)| (*key, std::env::var_os(key)))
                .collect(),
        );
        for (key, value) in &variables {
            // SAFETY: every test that reads or writes the environment holds env_lock.
            unsafe { std::env::set_var(key, value) }
        }
        assert_eq!(omarchy(), None, "detected Omarchy without a palette");

        let write = |root: &Path, accent: &str| {
            let dir = root.join("omarchy/current/theme");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("colors.toml"),
                format!("background=\"#121212\"\nforeground=\"#bebebe\"\naccent=\"{accent}\"\n"),
            )
            .unwrap();
        };
        write(&config, "#112233");
        assert_eq!(omarchy().unwrap().accent, "#112233", "legacy path");
        write(&state, "#445566");
        assert_eq!(omarchy().unwrap().accent, "#445566", "state precedence");
        write(&state, "#778899");
        assert_eq!(omarchy().unwrap().accent, "#778899", "theme change");
        write(&state, "invalid");
        assert_eq!(omarchy(), None, "invalid theme must fall back");
    }
}
