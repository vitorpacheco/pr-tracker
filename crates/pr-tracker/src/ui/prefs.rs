//! Window preferences kept between runs, beside the configuration file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pr_tracker_core::config;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// system, dark or light.
    pub theme: String,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub inspector_width: f32,
    pub inspector_collapsed: bool,
    /// Unsent comments, by item key.
    pub drafts: BTreeMap<String, String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: "system".into(),
            sidebar_width: 220.0,
            sidebar_collapsed: false,
            inspector_width: 400.0,
            inspector_collapsed: false,
            drafts: BTreeMap::new(),
        }
    }
}

/// `window.json` in the directory of the configuration file.
pub fn path() -> Option<PathBuf> {
    Some(config::path().ok()?.parent()?.join("window.json"))
}

impl Prefs {
    /// Missing or unreadable preferences fall back to the defaults.
    pub fn load() -> Prefs {
        path().and_then(|path| std::fs::read(path).ok()).and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
    }

    /// Preferences are a convenience: a failed write is not worth interrupting for.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        let Ok(data) = serde_json::to_vec_pretty(self) else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        if std::fs::write(&tmp, data).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_missing_fields_keep_defaults() {
        let prefs: Prefs = serde_json::from_str(r#"{"theme":"dark","future":1,"drafts":{"a|b|0#1":"oi"}}"#).unwrap();
        assert_eq!((prefs.theme.as_str(), prefs.sidebar_width, prefs.drafts.len()), ("dark", 220.0, 1));
        let again: Prefs = serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(again, prefs);
    }
}
