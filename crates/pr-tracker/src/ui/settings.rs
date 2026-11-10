//! The editable copy of the configuration behind the settings dialog.

use std::collections::BTreeMap;

use pr_tracker_core::config::{CommentPreset, Config, Instance, Provider, Repo};
use rune::TextEdit;

pub const TOOLS: [&str; 5] = ["git", "gh", "glab", "tea", "hunk"];
/// Providers with an integration, offered when adding an instance.
pub const PROVIDERS: [Provider; 3] = [Provider::GitHub, Provider::GitLab, Provider::Gitea];
pub const LANGUAGES: [(&str, &str); 3] = [
    ("system", "Sistema"),
    ("en", "English"),
    ("pt", "Português"),
];
pub const MERGE_METHODS: [(&str, &str); 4] = [
    ("", "Padrão (merge)"),
    ("merge", "Merge"),
    ("squash", "Squash"),
    ("rebase", "Rebase"),
];
pub const TERMINALS: [(&str, &str); 4] = [
    ("auto", "Automático"),
    ("herdr", "herdr"),
    ("tmux", "tmux"),
    ("inline", "Janela de terminal"),
];
pub const DIFF_TOOLS: [(&str, &str); 2] = [("hunk", "hunk"), ("none", "git diff")];

/// Names one text field of the form, so a handler can find it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    ThemeFile,
    RefreshInterval,
    WorktreeDir,
    DesktopTerminal,
    Tool(usize),
    InstanceName(u64),
    InstanceHost(u64),
    PresetLabel(u64, u64),
    PresetBody(u64, u64),
    RepoName(u64),
    RepoPath(u64),
    RepoRemote(u64),
}

pub struct PresetForm {
    pub id: u64,
    pub label: TextEdit,
    pub body: TextEdit,
}

pub struct InstanceForm {
    pub id: u64,
    pub name: TextEdit,
    pub provider: Provider,
    pub host: TextEdit,
    pub merge_method: String,
    pub disabled: bool,
    pub auto_merge: bool,
    pub delete_branch: bool,
    pub presets: Vec<PresetForm>,
}

pub struct RepoForm {
    pub id: u64,
    /// The [`InstanceForm::id`] it belongs to, so renaming an instance in the
    /// same edit keeps its repositories.
    pub instance: u64,
    pub name: TextEdit,
    pub path: TextEdit,
    pub remote: TextEdit,
    pub track_all: bool,
    pub merge_method: String,
}

pub struct SettingsForm {
    base: Config,
    next_id: u64,
    pub theme_file: TextEdit,
    pub language: String,
    pub refresh_interval: TextEdit,
    pub worktree_dir: TextEdit,
    pub desktop_terminal: TextEdit,
    pub terminal: String,
    pub diff_tool: String,
    pub tools: Vec<TextEdit>,
    pub instances: Vec<InstanceForm>,
    pub repos: Vec<RepoForm>,
    /// The key of the select whose options are showing.
    pub open_select: Option<String>,
}

impl SettingsForm {
    pub fn new(cfg: &Config) -> SettingsForm {
        let mut form = SettingsForm {
            base: cfg.clone(),
            next_id: 1,
            theme_file: TextEdit::new(cfg.theme_file.clone()),
            language: if cfg.language.is_empty() {
                "system".into()
            } else {
                cfg.language.clone()
            },
            refresh_interval: TextEdit::new(cfg.refresh_interval.clone()),
            worktree_dir: TextEdit::new(cfg.worktree_dir.clone()),
            desktop_terminal: TextEdit::new(cfg.desktop_terminal.clone()),
            terminal: cfg.terminal.clone(),
            diff_tool: cfg.diff_tool.clone(),
            tools: TOOLS
                .iter()
                .map(|tool| TextEdit::new(cfg.tool_paths.get(*tool).cloned().unwrap_or_default()))
                .collect(),
            instances: Vec::new(),
            repos: Vec::new(),
            open_select: None,
        };
        for instance in &cfg.instances {
            let id = form.id();
            let presets = instance
                .comment_presets
                .iter()
                .map(|preset| PresetForm {
                    id: form.id(),
                    label: TextEdit::new(preset.label.clone()),
                    body: TextEdit::new(preset.body.clone()),
                })
                .collect();
            form.instances.push(InstanceForm {
                id,
                name: TextEdit::new(instance.name.clone()),
                provider: instance.provider,
                host: TextEdit::new(instance.host.clone()),
                merge_method: instance.merge_method.clone(),
                disabled: instance.disabled,
                auto_merge: instance.auto_merge,
                delete_branch: instance.delete_branch,
                presets,
            });
        }
        for repo in &cfg.repos {
            // A mapping whose instance is gone keeps pointing at nothing and is
            // reported when saving, as the file itself would be.
            let instance = cfg
                .instances
                .iter()
                .position(|instance| instance.name == repo.instance)
                .map_or(0, |index| form.instances[index].id);
            let id = form.id();
            form.repos.push(RepoForm {
                id,
                instance,
                name: TextEdit::new(repo.name.clone()),
                path: TextEdit::new(repo.path.clone()),
                remote: TextEdit::new(repo.remote.clone()),
                track_all: repo.track_all,
                merge_method: repo.merge_method.clone(),
            });
        }
        form
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn instance_mut(&mut self, id: u64) -> Option<&mut InstanceForm> {
        self.instances.iter_mut().find(|instance| instance.id == id)
    }

    pub fn repo_mut(&mut self, id: u64) -> Option<&mut RepoForm> {
        self.repos.iter_mut().find(|repo| repo.id == id)
    }

    pub fn edit(&mut self, field: Field) -> Option<&mut TextEdit> {
        match field {
            Field::ThemeFile => Some(&mut self.theme_file),
            Field::RefreshInterval => Some(&mut self.refresh_interval),
            Field::WorktreeDir => Some(&mut self.worktree_dir),
            Field::DesktopTerminal => Some(&mut self.desktop_terminal),
            Field::Tool(index) => self.tools.get_mut(index),
            Field::InstanceName(id) => self.instance_mut(id).map(|instance| &mut instance.name),
            Field::InstanceHost(id) => self.instance_mut(id).map(|instance| &mut instance.host),
            Field::PresetLabel(instance, id) => self
                .preset_mut(instance, id)
                .map(|preset| &mut preset.label),
            Field::PresetBody(instance, id) => {
                self.preset_mut(instance, id).map(|preset| &mut preset.body)
            }
            Field::RepoName(id) => self.repo_mut(id).map(|repo| &mut repo.name),
            Field::RepoPath(id) => self.repo_mut(id).map(|repo| &mut repo.path),
            Field::RepoRemote(id) => self.repo_mut(id).map(|repo| &mut repo.remote),
        }
    }

    fn preset_mut(&mut self, instance: u64, id: u64) -> Option<&mut PresetForm> {
        self.instance_mut(instance)?
            .presets
            .iter_mut()
            .find(|preset| preset.id == id)
    }

    pub fn add_instance(&mut self) {
        let id = self.id();
        self.instances.push(InstanceForm {
            id,
            name: TextEdit::new(""),
            provider: Provider::GitHub,
            host: TextEdit::new(""),
            merge_method: "merge".into(),
            disabled: false,
            auto_merge: false,
            delete_branch: false,
            presets: Vec::new(),
        });
    }

    /// Removing an instance removes the repositories mapped to it.
    pub fn remove_instance(&mut self, id: u64) {
        self.instances.retain(|instance| instance.id != id);
        self.repos.retain(|repo| repo.instance != id);
    }

    pub fn add_preset(&mut self, instance: u64) {
        let id = self.id();
        if let Some(instance) = self.instance_mut(instance) {
            instance.presets.push(PresetForm {
                id,
                label: TextEdit::new(""),
                body: TextEdit::new(""),
            });
        }
    }

    pub fn remove_preset(&mut self, instance: u64, id: u64) {
        if let Some(instance) = self.instance_mut(instance) {
            instance.presets.retain(|preset| preset.id != id);
        }
    }

    pub fn add_repo(&mut self) {
        let id = self.id();
        self.repos.push(RepoForm {
            id,
            instance: self.instances.first().map_or(0, |instance| instance.id),
            name: TextEdit::new(""),
            path: TextEdit::new(""),
            remote: TextEdit::new(""),
            track_all: false,
            merge_method: String::new(),
        });
    }

    pub fn remove_repo(&mut self, id: u64) {
        self.repos.retain(|repo| repo.id != id);
    }

    /// The name an instance will be saved under, as shown in the repository
    /// instance picker.
    pub fn instance_label(&self, id: u64) -> String {
        self.instances
            .iter()
            .find(|instance| instance.id == id)
            .map(|instance| instance.name.text.trim().to_string())
            .unwrap_or_default()
    }

    /// The configuration this form describes. Fields the dialog does not show
    /// keep their current values.
    pub fn to_config(&self) -> Config {
        let mut cfg = self.base.clone();
        cfg.language = self.language.clone();
        cfg.theme_file = self.theme_file.text.trim().to_string();
        cfg.refresh_interval = self.refresh_interval.text.trim().to_string();
        cfg.worktree_dir = self.worktree_dir.text.trim().to_string();
        cfg.desktop_terminal = self.desktop_terminal.text.trim().to_string();
        cfg.terminal = self.terminal.clone();
        cfg.diff_tool = self.diff_tool.clone();
        cfg.tool_paths = TOOLS
            .iter()
            .zip(&self.tools)
            .filter(|(_, edit)| !edit.text.trim().is_empty())
            .map(|(tool, edit)| (tool.to_string(), edit.text.trim().to_string()))
            .collect::<BTreeMap<_, _>>();
        cfg.instances = self
            .instances
            .iter()
            .map(|instance| Instance {
                name: instance.name.text.trim().to_string(),
                provider: instance.provider,
                host: instance.host.text.trim().to_string(),
                disabled: instance.disabled,
                merge_method: instance.merge_method.clone(),
                auto_merge: instance.auto_merge,
                delete_branch: instance.delete_branch,
                comment_presets: instance
                    .presets
                    .iter()
                    .map(|preset| CommentPreset {
                        label: preset.label.text.clone(),
                        body: preset.body.text.clone(),
                    })
                    .collect(),
            })
            .collect();
        cfg.repos = self
            .repos
            .iter()
            .map(|repo| Repo {
                instance: self.instance_label(repo.instance),
                name: repo.name.text.trim().to_string(),
                path: repo.path.text.trim().to_string(),
                remote: repo.remote.text.trim().to_string(),
                track_all: repo.track_all,
                merge_method: repo.merge_method.clone(),
            })
            .collect();
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        let mut cfg = Config::default();
        cfg.clone_roots = vec!["~/code".into()];
        cfg.tool_paths.insert("gh".into(), "/opt/gh".into());
        cfg.instances = vec![Instance {
            name: "work".into(),
            provider: Provider::GitLab,
            host: "git.corp".into(),
            comment_presets: vec![CommentPreset {
                label: "Ok".into(),
                body: "aprovado".into(),
            }],
            ..Instance::default()
        }];
        cfg.repos = vec![Repo {
            instance: "work".into(),
            name: "g/p".into(),
            path: "~/p".into(),
            track_all: true,
            ..Repo::default()
        }];
        cfg
    }

    #[test]
    fn an_untouched_form_describes_the_same_configuration() {
        let cfg = sample();
        assert_eq!(SettingsForm::new(&cfg).to_config(), cfg);
    }

    #[test]
    fn renaming_an_instance_carries_its_repositories() {
        let mut form = SettingsForm::new(&sample());
        let id = form.instances[0].id;
        form.edit(Field::InstanceName(id))
            .unwrap()
            .replace_text("job");
        let cfg = form.to_config();
        assert_eq!(
            (
                cfg.instances[0].name.as_str(),
                cfg.repos[0].instance.as_str()
            ),
            ("job", "job")
        );
        form.remove_instance(id);
        assert!(
            form.to_config().repos.is_empty(),
            "repositories outlived their instance"
        );
    }

    #[test]
    fn rows_are_added_removed_and_found_by_id() {
        let mut form = SettingsForm::new(&sample());
        form.add_instance();
        let added = form.instances[1].id;
        form.add_preset(added);
        let preset = form.instances[1].presets[0].id;
        form.edit(Field::PresetLabel(added, preset))
            .unwrap()
            .replace_text("Revisar");
        form.add_repo();
        let repo = form.repos[1].id;
        form.edit(Field::RepoName(repo))
            .unwrap()
            .replace_text(" org/new ");
        let cfg = form.to_config();
        assert_eq!(cfg.instances[1].comment_presets[0].label, "Revisar");
        assert_eq!(
            (cfg.repos[1].name.as_str(), cfg.repos[1].instance.as_str()),
            ("org/new", "work")
        );
        form.remove_preset(added, preset);
        form.remove_repo(repo);
        assert!(
            form.edit(Field::PresetLabel(added, preset)).is_none()
                && form.edit(Field::RepoName(repo)).is_none()
        );
        assert!(form.edit(Field::Tool(1)).is_some() && form.edit(Field::Tool(9)).is_none());
    }

    #[test]
    fn blank_tool_paths_are_not_saved() {
        let mut form = SettingsForm::new(&sample());
        form.edit(Field::Tool(1)).unwrap().replace_text("  ");
        form.edit(Field::Tool(0))
            .unwrap()
            .replace_text("/usr/bin/git");
        assert_eq!(
            form.to_config().tool_paths,
            BTreeMap::from([("git".to_string(), "/usr/bin/git".to_string())])
        );
    }
}
