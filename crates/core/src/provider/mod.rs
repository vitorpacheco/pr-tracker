//! Talks to code hosting platforms through their official CLIs (gh for GitHub,
//! glab for GitLab, tea for Gitea). pr-tracker handles no API tokens:
//! authentication is whatever the CLI is logged in with.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{Instance, Provider, Repo};
use crate::error::{Error, Result};
use crate::toolchain::{self, Ctx};

mod bitbucket;
mod gitea;
mod github;
mod gitlab;

pub use gitea::gitea_instances;

/// How the current user relates to an item (bitmask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Relation(pub u8);

impl Relation {
    pub const NONE: Relation = Relation(0);
    pub const REVIEW_REQUESTED: Relation = Relation(1);
    pub const AUTHORED: Relation = Relation(1 << 1);
    pub const ASSIGNED: Relation = Relation(1 << 2);
    pub const MENTIONED: Relation = Relation(1 << 3);

    pub fn contains(self, other: Relation) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for Relation {
    type Output = Relation;
    fn bitor(self, other: Relation) -> Relation {
        Relation(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for Relation {
    fn bitor_assign(&mut self, other: Relation) {
        self.0 |= other.0;
    }
}

/// Distinguishes pull/merge requests from issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Pr,
    Issue,
}

/// The aggregated pipeline result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiState {
    #[default]
    None,
    Success,
    Failure,
    Pending,
    Canceled,
}

impl CiState {
    /// The stable name used by `pr-tracker list` and status labels.
    pub fn as_str(self) -> &'static str {
        match self {
            CiState::None => "",
            CiState::Success => "success",
            CiState::Failure => "failure",
            CiState::Pending => "pending",
            CiState::Canceled => "canceled",
        }
    }
}

/// One pipeline job or status check.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub name: String,
    pub state: CiState,
    pub url: String,
}

/// Review decisions, normalized across providers.
pub const REVIEW_APPROVED: &str = "approved";
pub const REVIEW_CHANGES_REQUESTED: &str = "changes_requested";
pub const REVIEW_REQUIRED: &str = "review_required";

/// A pull request (GitHub, Gitea), merge request (GitLab) or an issue. Fields
/// about branches, reviews and pipelines only apply to [`Kind::Pr`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub kind: Kind,
    pub instance: String,
    pub provider: Provider,
    pub host: String,

    /// owner/name or group/.../project.
    pub repo: String,
    pub repo_url: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub draft: bool,

    pub source_branch: String,
    pub target_branch: String,
    pub from_fork: bool,
    /// Reported by the server; false can also mean unknown.
    pub behind_base: bool,
    /// GitHub reports whether the viewer can update a stale head.
    pub can_update_branch: bool,

    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,

    pub additions: i64,
    pub deletions: i64,
    pub files: i64,

    pub review: String,
    pub approved_by: Vec<String>,
    pub approved_by_me: bool,
    pub conflicts: bool,

    pub ci: CiState,
    pub checks: Vec<Check>,

    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub comments: i64,

    pub relations: Relation,
}

impl Item {
    pub fn is_issue(&self) -> bool {
        self.kind == Kind::Issue
    }

    /// Identifies an item across instances. GitLab numbers issues and merge
    /// requests independently, so the kind is part of the key.
    pub fn key(&self) -> String {
        format!("{}|{}|{}#{}", self.instance, self.repo, self.kind as u8, self.number)
    }

    /// The display reference ("#12" or "!12").
    pub fn reference(&self) -> String {
        if self.provider == Provider::GitLab && self.kind == Kind::Pr {
            format!("!{}", self.number)
        } else {
            format!("#{}", self.number)
        }
    }
}

/// One entry of a conversation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
    /// Set for PR reviews (approved, changes_requested, commented).
    pub review: String,
    /// Path and line locate inline code comments.
    pub path: String,
    pub line: i64,
}

/// The description and the comments of an item, oldest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Thread {
    pub body: String,
    pub comments: Vec<Comment>,
}

/// Controls how a PR is merged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeOptions {
    /// merge, squash or rebase.
    pub method: String,
    pub auto: bool,
    pub delete_branch: bool,
}

/// Implemented by every provider integration.
pub trait Client: Send + Sync {
    fn instance(&self) -> &Instance;
    /// The CLI binary the client depends on.
    fn tool(&self) -> &'static str;
    /// Open PRs and issues related to the current user.
    fn list(&self, ctx: &Ctx) -> Result<Vec<Item>>;
    /// The description and comments of an item.
    fn thread(&self, ctx: &Ctx, item: &Item) -> Result<Thread>;
    fn add_comment(&self, ctx: &Ctx, item: &Item, body: &str) -> Result<()>;
    fn approve(&self, ctx: &Ctx, pr: &Item) -> Result<()>;
    fn merge(&self, ctx: &Ctx, pr: &Item, options: &MergeOptions) -> Result<()>;
    /// Closes a PR without merging it. The source branch is kept.
    fn close(&self, ctx: &Ctx, pr: &Item) -> Result<()>;
    /// Changes the PR's readiness for review.
    fn set_draft(&self, ctx: &Ctx, pr: &Item, draft: bool) -> Result<()>;
    /// Incorporates the target branch into the source branch on the server.
    fn update_branch(&self, ctx: &Ctx, pr: &Item) -> Result<()>;
    /// Switches the clone at `dir` to the PR branch.
    fn checkout(&self, ctx: &Ctx, pr: &Item, dir: &Path) -> Result<()>;
    /// The server-side ref that always points to the PR head, available on the
    /// base repository even for forks.
    fn head_ref(&self, pr: &Item) -> String;
    fn auth_status(&self, ctx: &Ctx) -> Result<()>;
}

/// Catalog key of the error returned by providers without a CLI integration.
pub const ERR_NOT_SUPPORTED: &str = "provider ainda não suportado";

/// Where to get a CLI. The text is a catalog key.
pub fn install_hint(tool: &str) -> &str {
    match tool {
        "gh" => "https://cli.github.com (ex.: brew install gh, pacman -S github-cli, winget install GitHub.cli)",
        "glab" => "https://gitlab.com/gitlab-org/cli (ex.: brew install glab, pacman -S glab, winget install glab.glab)",
        "tea" => "https://gitea.com/gitea/tea (ex.: brew install tea, ou um binário das releases); precisa da 0.14 ou mais nova",
        "bkt" => "https://github.com/avivsinai/bitbucket-cli (integração ainda não implementada)",
        "hunk" => "https://github.com/modem-dev/hunk",
        other => other,
    }
}

/// Reports whether a CLI is on PATH.
pub fn tool_available(tool: &str) -> bool {
    toolchain::lookup(&Ctx::background(), tool).is_ok()
}

/// The repositories of `instance` that track every open pull/merge request.
fn tracked(instance: &Instance, repos: &[Repo]) -> Vec<String> {
    repos.iter().filter(|repo| repo.instance == instance.name && repo.track_all).map(|repo| repo.name.clone()).collect()
}

/// A provider client. Repository settings are optional, so callers that only
/// need authentication or tool metadata can pass none.
pub fn new(instance: &Instance, repos: &[Repo]) -> Arc<dyn Client> {
    let tracked = tracked(instance, repos);
    match instance.provider {
        Provider::GitHub => Arc::new(github::GitHub::new(instance.clone(), tracked)),
        Provider::GitLab => Arc::new(gitlab::GitLab::new(instance.clone(), tracked)),
        Provider::Gitea => Arc::new(gitea::Gitea::new(instance.clone(), tracked)),
        Provider::Bitbucket | Provider::Unknown => Arc::new(bitbucket::Bitbucket::new(instance.clone())),
    }
}

/// Runs a CLI and returns its stdout; stderr is folded into the error.
fn run(ctx: &Ctx, dir: Option<&Path>, env: &[(&str, &str)], name: &str, args: &[&str]) -> Result<Vec<u8>> {
    let path = toolchain::lookup(ctx, name)?;
    let mut cmd = toolchain::command(ctx, &path);
    cmd.args(args).envs(env.iter().copied());
    // Never let a CLI block on an interactive prompt.
    cmd.env("GH_PROMPT_DISABLED", "1").env("GLAB_NO_PROMPT", "1");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let output = toolchain::output(ctx, &mut cmd)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.is_empty() {
        message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    if message.is_empty() {
        message = output.status.to_string();
    }
    Err(Error::external(format!("{name} {}: {message}", first_arg(args))))
}

fn first_arg(args: &[&str]) -> String {
    match args {
        [first, second, ..] => format!("{first} {second}"),
        [first] => first.to_string(),
        [] => String::new(),
    }
}

fn merge_method(method: &str) -> &str {
    match method {
        "squash" | "rebase" => method,
        _ => "merge",
    }
}

fn sort_comments(comments: &mut [Comment]) {
    comments.sort_by_key(|comment| comment.created_at);
}

/// Merges items returned by several queries, OR-ing their relations.
#[derive(Default)]
struct Accumulator {
    order: Vec<String>,
    by_key: HashMap<String, Item>,
}

impl Accumulator {
    fn add(&mut self, mut item: Item, relation: Relation) {
        let key = item.key();
        if let Some(previous) = self.by_key.get_mut(&key) {
            previous.relations |= relation;
            return;
        }
        item.relations = relation;
        self.by_key.insert(key.clone(), item);
        self.order.push(key);
    }

    fn list(mut self) -> Vec<Item> {
        self.order.iter().filter_map(|key| self.by_key.remove(key)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_merges_relations() {
        let mut acc = Accumulator::default();
        let pr = Item { instance: "gh".into(), repo: "o/r".into(), number: 1, ..Item::default() };
        acc.add(pr.clone(), Relation::REVIEW_REQUESTED);
        acc.add(pr, Relation::ASSIGNED);
        acc.add(Item { instance: "gh".into(), repo: "o/r".into(), number: 2, ..Item::default() }, Relation::AUTHORED);
        acc.add(Item { instance: "gh".into(), repo: "o/r".into(), number: 3, ..Item::default() }, Relation::NONE);
        let got = acc.list();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].relations, Relation::REVIEW_REQUESTED | Relation::ASSIGNED);
        assert_eq!(got[1].relations, Relation::AUTHORED);
        assert_eq!(got[2].relations, Relation::NONE);
    }

    #[test]
    fn tracked_repos_are_scoped_to_the_instance() {
        let instance = Instance { name: "work".into(), provider: Provider::GitLab, host: "gitlab.example.com".into(), ..Instance::default() };
        let repo = |instance: &str, name: &str, track_all| Repo { instance: instance.into(), name: name.into(), track_all, ..Repo::default() };
        let repos = [repo("work", "group/all", true), repo("work", "group/related-only", false), repo("other", "group/other", true)];
        assert_eq!(tracked(&instance, &repos), ["group/all"]);
    }

    #[test]
    fn missing_tool_is_reported_with_an_install_hint() {
        let paths = std::collections::BTreeMap::from([("gh".to_string(), "/nonexistent/pr-tracker-test-cli".to_string())]);
        let ctx = Ctx::background().with_paths(&paths);
        let err = run(&ctx, None, &[], "gh", &["auth", "status"]).unwrap_err();
        assert!(err.is_missing_tool());
        assert!(err.text("en").contains("https://cli.github.com"), "{}", err.text("en"));
    }

    #[test]
    fn keys_and_references_distinguish_kinds_and_providers() {
        let mut item = Item { instance: "work".into(), repo: "g/p".into(), number: 12, provider: Provider::GitLab, ..Item::default() };
        assert_eq!((item.key().as_str(), item.reference().as_str()), ("work|g/p|0#12", "!12"));
        item.kind = Kind::Issue;
        assert_eq!((item.key().as_str(), item.reference().as_str()), ("work|g/p|1#12", "#12"));
    }
}
