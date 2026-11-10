//! Loads and persists the pr-tracker configuration file.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::{i18n, msg};

const APP_NAME: &str = "pr-tracker";

/// A code hosting platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    GitHub,
    GitLab,
    Gitea,
    Bitbucket,
    /// A value this version does not know; rejected by [`Config::validate`].
    #[serde(other)]
    Unknown,
}

impl Provider {
    /// Every provider pr-tracker knows about, in display order.
    pub const ALL: [Provider; 4] = [Provider::GitHub, Provider::GitLab, Provider::Gitea, Provider::Bitbucket];

    pub fn parse(name: &str) -> Option<Provider> {
        Self::ALL.into_iter().find(|provider| provider.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::GitLab => "gitlab",
            Provider::Gitea => "gitea",
            Provider::Bitbucket => "bitbucket",
            Provider::Unknown => "unknown",
        }
    }

    /// The SaaS host of a provider.
    pub fn default_host(self) -> &'static str {
        match self {
            Provider::GitHub => "github.com",
            Provider::GitLab => "gitlab.com",
            Provider::Gitea => "gitea.com",
            Provider::Bitbucket => "bitbucket.org",
            Provider::Unknown => "",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A reusable PR/MR comment with a button label.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommentPreset {
    pub label: String,
    pub body: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One account on one host (SaaS or self-hosted).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Instance {
    pub name: String,
    pub provider: Provider,
    pub host: String,
    /// Disabled instances are kept in the file but never queried.
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// merge, squash or rebase.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub merge_method: String,
    /// Asks the provider to merge once the pipeline succeeds.
    #[serde(skip_serializing_if = "is_false")]
    pub auto_merge: bool,
    /// Removes the source branch after merging.
    #[serde(skip_serializing_if = "is_false")]
    pub delete_branch: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comment_presets: Vec<CommentPreset>,
}

/// Per-repository settings, including its optional local clone.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Repo {
    pub instance: String,
    /// owner/repo (GitHub, Gitea) or group/subgroup/project (GitLab).
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// Includes every open pull/merge request from this repository, regardless
    /// of its relation to the current user.
    #[serde(skip_serializing_if = "is_false")]
    pub track_all: bool,
    /// Overrides the instance merge method for this repository.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub merge_method: String,
}

/// The on-disk configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// system (default), en or pt.
    pub language: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub theme_file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desktop_terminal: String,
    /// A duration such as "5m" or "90s".
    pub refresh_interval: String,
    /// Holds the PR worktrees. Defaults to ~/.pr-tracker/worktrees.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub worktree_dir: String,
    /// Where interactive tools open: auto, herdr, tmux or inline.
    pub terminal: String,
    /// "hunk" or "none".
    pub diff_tool: String,
    /// Scanned to discover local clones.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clone_roots: Vec<String>,
    /// Optionally pins external executables.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_paths: BTreeMap<String, String>,

    pub instances: Vec<Instance>,
    pub repos: Vec<Repo>,

    #[serde(skip)]
    path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: "system".into(),
            theme_file: String::new(),
            desktop_terminal: String::new(),
            refresh_interval: "5m".into(),
            worktree_dir: String::new(),
            terminal: "auto".into(),
            diff_tool: "hunk".into(),
            clone_roots: Vec::new(),
            tool_paths: BTreeMap::new(),
            instances: Vec::new(),
            repos: Vec::new(),
            path: PathBuf::new(),
        }
    }
}

pub fn home_dir() -> Result<PathBuf> {
    std::env::home_dir().filter(|home| !home.as_os_str().is_empty()).ok_or_else(|| msg!("pasta do usuário não encontrada"))
}

/// The configuration directory: $XDG_CONFIG_HOME/pr-tracker on Linux (falling
/// back to ~/.config), and ~/.config/pr-tracker on macOS and Windows.
pub fn dir() -> Result<PathBuf> {
    if !cfg!(any(target_os = "macos", windows))
        && let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        && xdg.is_absolute()
    {
        return Ok(xdg.join(APP_NAME));
    }
    Ok(home_dir()?.join(".config").join(APP_NAME))
}

/// ~/.pr-tracker, where worktrees are stored.
pub fn data_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(format!(".{APP_NAME}")))
}

/// The configuration file path. PR_TRACKER_CONFIG overrides it.
pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PR_TRACKER_CONFIG").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    Ok(dir()?.join("config.toml"))
}

/// Parses durations as written in existing configuration files: a sequence of
/// decimal numbers with a unit, such as "5m", "90s" or "1h30m".
pub fn parse_duration(text: &str) -> Option<Duration> {
    let mut rest = text.strip_prefix('+').unwrap_or(text);
    if rest == "0" {
        return Some(Duration::ZERO);
    }
    if rest.is_empty() {
        return None;
    }
    let mut total = 0f64;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
        let value: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let seconds = match &rest[..unit] {
            "ns" => 1e-9,
            "us" | "µs" | "μs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return None,
        };
        rest = &rest[unit..];
        total += value * seconds;
    }
    Duration::try_from_secs_f64(total).ok()
}

/// Replaces a leading ~ with the user's home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if (path == "~" || path.starts_with("~/") || path.starts_with("~\\"))
        && let Ok(home) = home_dir()
    {
        return home.join(path[1..].trim_start_matches(['/', '\\']));
    }
    PathBuf::from(path)
}

/// Strips the scheme and trailing slashes from a host.
pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    let host = host.strip_prefix("https://").unwrap_or(host);
    let host = host.strip_prefix("http://").unwrap_or(host);
    host.trim_end_matches('/').to_string()
}

impl Config {
    /// Reads the configuration file. A missing file yields defaults and
    /// `created = true` so the caller can seed and save it.
    pub fn load() -> Result<(Config, bool)> {
        let path = path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Config { path, ..Config::default() }, true));
            }
            Err(err) => return Err(msg!("lendo %s: %w", path.display().to_string(), Error::from(err))),
        };
        let mut cfg: Config = toml::from_str(&text)
            .map_err(|err| msg!("lendo %s: %w", path.display().to_string(), Error::external(err.to_string())))?;
        cfg.path = path;
        cfg.validate()?;
        Ok((cfg, false))
    }

    /// Writes the configuration file, creating its directory.
    pub fn save(&mut self) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            self.path = path()?;
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string(self).map_err(|err| Error::external(err.to_string()))?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Where the configuration was loaded from.
    pub fn file_path(&self) -> &Path {
        &self.path
    }

    pub fn validate(&self) -> Result<()> {
        if !i18n::supported(&self.language) {
            return Err(msg!("idioma inválido %q (system, en, pt)", &self.language));
        }
        if parse_duration(&self.refresh_interval).is_none() {
            return Err(msg!(
                "refresh_interval inválido %q: %w",
                &self.refresh_interval,
                msg!("duração inválida")
            ));
        }
        let mut seen: Vec<&str> = Vec::new();
        for instance in &self.instances {
            if instance.name.is_empty() {
                return Err(msg!("instância sem nome"));
            }
            if seen.contains(&instance.name.as_str()) {
                return Err(msg!("instância duplicada %q", &instance.name));
            }
            seen.push(&instance.name);
            for (index, preset) in instance.comment_presets.iter().enumerate() {
                if preset.label.trim().is_empty() || preset.body.trim().is_empty() {
                    return Err(msg!("instância %q: comentário predefinido %d exige nome e texto", &instance.name, index + 1));
                }
            }
            if instance.provider == Provider::Unknown {
                return Err(msg!("instância %q: provider desconhecido %q", &instance.name, instance.provider.as_str()));
            }
            if !valid_merge_method(&instance.merge_method) {
                return Err(msg!("instância %q: merge_method inválido %q", &instance.name, &instance.merge_method));
            }
        }
        if !matches!(self.terminal.as_str(), "auto" | "herdr" | "tmux" | "inline") {
            return Err(msg!("terminal inválido %q (auto, herdr, tmux, inline)", &self.terminal));
        }
        Ok(())
    }

    /// The refresh interval, with a floor of 15 seconds.
    pub fn interval(&self) -> Duration {
        match parse_duration(&self.refresh_interval) {
            Some(interval) if !interval.is_zero() => interval.max(Duration::from_secs(15)),
            _ => Duration::from_secs(300),
        }
    }

    /// The resolved worktree directory.
    pub fn worktrees(&self) -> Result<PathBuf> {
        if !self.worktree_dir.is_empty() {
            return Ok(expand_home(&self.worktree_dir));
        }
        Ok(data_dir()?.join("worktrees"))
    }

    pub fn instance(&self, name: &str) -> Option<&Instance> {
        self.instances.iter().find(|instance| instance.name == name)
    }

    /// Adds or replaces an instance keyed by `old_name` (empty to add).
    pub fn upsert_instance(&mut self, old_name: &str, mut instance: Instance) -> Result<()> {
        if instance.host.is_empty() {
            instance.host = instance.provider.default_host().to_string();
        }
        instance.host = normalize_host(&instance.host);
        if instance.name.is_empty() {
            instance.name = instance.host.clone();
        }
        if self.instances.iter().any(|existing| existing.name == instance.name && instance.name != old_name) {
            return Err(msg!("já existe uma instância chamada %q", &instance.name));
        }
        if !old_name.is_empty()
            && let Some(index) = self.instances.iter().position(|existing| existing.name == old_name)
        {
            if old_name != instance.name {
                for repo in self.repos.iter_mut().filter(|repo| repo.instance == old_name) {
                    repo.instance = instance.name.clone();
                }
            }
            self.instances[index] = instance;
            return self.validate();
        }
        self.instances.push(instance);
        self.validate()
    }

    /// Deletes an instance and its repository mappings.
    pub fn remove_instance(&mut self, name: &str) -> bool {
        let before = self.instances.len();
        self.instances.retain(|instance| instance.name != name);
        self.repos.retain(|repo| repo.instance != name);
        self.instances.len() != before
    }

    fn repo_index(&self, instance: &str, name: &str) -> Option<usize> {
        self.repos.iter().position(|repo| repo.instance == instance && same_repo(&repo.name, name))
    }

    /// The settings of a repository.
    pub fn repo(&self, instance: &str, name: &str) -> Option<&Repo> {
        self.repo_index(instance, name).map(|index| &self.repos[index])
    }

    pub fn repo_mut(&mut self, instance: &str, name: &str) -> Option<&mut Repo> {
        self.repo_index(instance, name).map(|index| &mut self.repos[index])
    }

    /// Enables or disables tracking every open pull/merge request from a
    /// repository. Enabling creates an entry even without a local clone;
    /// disabling removes an otherwise empty entry.
    pub fn set_repo_track_all(&mut self, instance: &str, name: &str, enabled: bool) {
        let Some(index) = self.repo_index(instance, name) else {
            if enabled {
                self.repos.push(Repo { instance: instance.into(), name: name.into(), track_all: true, ..Repo::default() });
            }
            return;
        };
        let repo = &mut self.repos[index];
        repo.track_all = enabled;
        if !enabled && repo.path.is_empty() && repo.remote.is_empty() && repo.merge_method.is_empty() {
            self.repos.remove(index);
        }
    }

    /// Stores (or clears, with an empty path) the local clone path.
    pub fn set_repo_path(&mut self, instance: &str, name: &str, path: &str) {
        match self.repo_index(instance, name) {
            Some(index) if path.is_empty() => {
                self.repos[index].path.clear();
                if !self.repos[index].track_all {
                    self.repos.remove(index);
                }
            }
            Some(index) => self.repos[index].path = path.to_string(),
            None if path.is_empty() => {}
            None => self.repos.push(Repo { instance: instance.into(), name: name.into(), path: path.into(), ..Repo::default() }),
        }
    }

    /// Resolves relative palette files beside the configuration file. `None`
    /// means no custom palette is configured.
    pub fn theme_path(&self) -> Option<PathBuf> {
        if self.theme_file.is_empty() {
            return None;
        }
        let file = expand_home(&self.theme_file);
        if file.is_absolute() {
            return Some(file);
        }
        let base = if self.path.as_os_str().is_empty() { path().unwrap_or_default() } else { self.path.clone() };
        Some(base.parent().unwrap_or(Path::new("")).join(file))
    }
}

pub fn valid_merge_method(method: &str) -> bool {
    matches!(method, "" | "merge" | "squash" | "rebase")
}

/// Hosts compare repository paths case-insensitively.
pub fn same_repo(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::tests::env_lock;

    fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: every test that reads or writes the environment holds env_lock.
        unsafe { std::env::set_var(key, value) }
    }

    #[test]
    fn comment_presets_round_trip_and_validation() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let mut cfg = Config::default();
        let presets = vec![
            CommentPreset { label: "Aprovado".into(), body: "aprovado".into() },
            CommentPreset { label: "Reprovado".into(), body: "reprovado\n\nRevise os testes.".into() },
        ];
        cfg.instances = vec![
            Instance { name: "work".into(), provider: Provider::GitLab, host: "git.company.com".into(), comment_presets: presets.clone(), ..Instance::default() },
            Instance { name: "other".into(), provider: Provider::GitHub, host: "github.com".into(), ..Instance::default() },
        ];
        cfg.save().unwrap();
        let (loaded, _) = Config::load().unwrap();
        assert_eq!(loaded.instances[0].comment_presets, presets);
        assert!(loaded.instances[1].comment_presets.is_empty());
        for (label, body) in [("", "aprovado"), ("  ", "aprovado"), ("Aprovado", ""), ("Aprovado", " \n\t")] {
            cfg.instances[0].comment_presets = vec![CommentPreset { label: label.into(), body: body.into() }];
            assert!(cfg.validate().is_err(), "invalid preset accepted: {label:?} {body:?}");
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn dir_uses_xdg_on_linux() {
        let _guard = env_lock();
        let (xdg, home) = (std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"));
        set_env("XDG_CONFIG_HOME", "/xdg");
        assert_eq!(dir().unwrap(), PathBuf::from("/xdg/pr-tracker"));
        set_env("XDG_CONFIG_HOME", "");
        set_env("HOME", "/home/u");
        assert_eq!(dir().unwrap(), PathBuf::from("/home/u/.config/pr-tracker"));
        set_env("XDG_CONFIG_HOME", xdg.unwrap_or_default());
        set_env("HOME", home.unwrap_or_default());
    }

    #[test]
    fn save_load_round_trip() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("cfg").join("config.toml"));
        let (mut cfg, created) = Config::load().unwrap();
        assert!(created);
        cfg.upsert_instance("", Instance { provider: Provider::GitLab, host: "https://gitlab.corp.io/".into(), ..Instance::default() }).unwrap();
        let duplicate = Instance { provider: Provider::GitLab, name: "gitlab.corp.io".into(), ..Instance::default() };
        assert!(cfg.upsert_instance("", duplicate).is_err(), "expected duplicate name error");
        cfg.set_repo_path("gitlab.corp.io", "grp/sub/proj", "~/code/proj");
        cfg.save().unwrap();

        let (mut got, created) = Config::load().unwrap();
        assert!(!created);
        let instance = got.instance("gitlab.corp.io").unwrap();
        assert_eq!((instance.host.as_str(), instance.provider), ("gitlab.corp.io", Provider::GitLab));
        assert_eq!(got.repo("gitlab.corp.io", "GRP/sub/proj").unwrap().path, "~/code/proj");
        assert_eq!(got.interval(), Duration::from_secs(300));
        // Renaming an instance carries its repository mappings along.
        let renamed = Instance { name: "work".into(), provider: Provider::GitLab, host: "gitlab.corp.io".into(), ..Instance::default() };
        got.upsert_instance("gitlab.corp.io", renamed).unwrap();
        assert!(got.repo("work", "grp/sub/proj").is_some(), "repo mapping not renamed");
        got.remove_instance("work");
        assert!(got.repos.is_empty(), "repo mappings not removed with instance");
    }

    #[test]
    fn validate_and_interval() {
        let mut cfg = Config { refresh_interval: "soon".into(), ..Config::default() };
        assert!(cfg.validate().is_err(), "expected invalid interval");
        cfg.refresh_interval = "1s".into();
        assert_eq!(cfg.interval(), Duration::from_secs(15));
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("1.5m"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("90"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn unknown_provider_is_rejected() {
        let cfg: Config = toml::from_str("[[instances]]\nname = \"x\"\nprovider = \"sourcehut\"\nhost = \"h\"\n").unwrap();
        assert_eq!(cfg.instances[0].provider, Provider::Unknown);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn track_all_repo_persists_without_local_path() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let mut cfg = Config::default();
        cfg.set_repo_track_all("work", "group/project", true);
        cfg.save().unwrap();

        let (mut got, _) = Config::load().unwrap();
        let repo = got.repo("work", "GROUP/project").unwrap();
        assert!(repo.track_all && repo.path.is_empty());
        got.set_repo_path("work", "group/project", "");
        assert!(got.repo("work", "group/project").is_some_and(|repo| repo.track_all), "clearing a path removed tracked repo");
        got.set_repo_track_all("work", "group/project", false);
        assert!(got.repo("work", "group/project").is_none(), "otherwise empty repo was not removed");
    }

    #[test]
    fn language_preference_round_trip() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        set_env("PR_TRACKER_CONFIG", dir.path().join("config.toml"));
        let (mut cfg, _) = Config::load().unwrap();
        assert_eq!(cfg.language, "system");
        for language in ["pt", "en", "system"] {
            cfg.language = language.into();
            cfg.save().unwrap();
            assert_eq!(Config::load().unwrap().0.language, language);
        }
        cfg.language = "fr".into();
        assert!(cfg.validate().is_err(), "unsupported language accepted");
        std::fs::write(cfg.file_path(), "refresh_interval = \"5m\"\nterminal = \"auto\"\n").unwrap();
        assert_eq!(Config::load().unwrap().0.language, "system", "legacy config");
    }

    #[test]
    fn theme_path_and_persistence() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        set_env("PR_TRACKER_CONFIG", &path);
        let (mut cfg, _) = Config::load().unwrap();
        assert_eq!(cfg.theme_path(), None);
        cfg.theme_file = "colors.toml".into();
        assert_eq!(cfg.theme_path(), Some(dir.path().join("colors.toml")));
        cfg.save().unwrap();
        assert_eq!(Config::load().unwrap().0.theme_file, "colors.toml");
    }
}
