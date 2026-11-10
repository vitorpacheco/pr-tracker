use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread;

use chrono::{DateTime, Utc};
use regex::bytes::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::{
    Accumulator, Check, CiState, Client, Comment, Item, Kind, MergeOptions, REVIEW_APPROVED,
    REVIEW_CHANGES_REQUESTED, REVIEW_REQUIRED, Relation, Thread, install_hint, merge_method, run,
    sort_comments, tool_available,
};
use crate::config::{Instance, Provider, normalize_host};
use crate::error::{Error, Result};
use crate::msg;
use crate::toolchain::Ctx;

/// Talks to a Gitea server through tea (https://gitea.com/gitea/tea).
/// Gitea has no GraphQL API, so the list is assembled from several REST calls:
/// the issue search is the only endpoint that filters by how the current user
/// relates to an item, but it answers with bare issues, so branches, diff stats,
/// reviews and CI need one round of requests per pull request.
pub(super) struct Gitea {
    instance: Instance,
    tracked: Vec<String>,
    session: Mutex<Session>,
}

/// What login discovery found. Both stay empty until the first request.
#[derive(Default)]
struct Session {
    /// tea login name that serves this host.
    login: String,
    /// Server URL as tea stored it, without a trailing slash.
    base: String,
}

/// Bounds how many pull requests are enriched at once. Gitea servers are
/// usually small and self-hosted; a refresh should not look like a burst of
/// traffic.
const GITEA_WORKERS: usize = 6;

// tea gained "tea api" in 0.12 and learned to accept a repository slug outside
// a clone in 0.14.1, which is what the pull commands here rely on.
const TEA_MIN_MAJOR: i64 = 0;
const TEA_MIN_MINOR: i64 = 14;

/// The page size of every list request: it matches what gh and glab use, and
/// Gitea truncates anything above max_response_items (50 by default) regardless.
const PAGE_SIZE: usize = 50;

fn eq_fold(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn invalid(err: serde_json::Error) -> Error {
    msg!("resposta inválida do tea: %w", Error::from(err))
}

// ---------- decoding ----------

/// Reads JSON `null` as the default value, like Go's decoder does.
fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A list where `null`, as the list or as one of its entries, is the default.
fn list<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let entries = Option::<Vec<Option<T>>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(entries.into_iter().map(Option::unwrap_or_default).collect())
}

/// Go's zero time (0001-01-01T00:00:00Z), which Gitea writes for unset dates.
const ZERO_TIME_SECONDS: i64 = -62_135_596_800;

fn timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&text)
        .map_err(serde::de::Error::custom)?
        .with_timezone(&Utc);
    let zero = parsed.timestamp() == ZERO_TIME_SECONDS && parsed.timestamp_subsec_nanos() == 0;
    Ok((!zero).then_some(parsed))
}

fn decode<T: DeserializeOwned + Default>(out: &[u8]) -> serde_json::Result<T> {
    Ok(serde_json::from_slice::<Option<T>>(out)?.unwrap_or_default())
}

fn decode_list<T: DeserializeOwned + Default>(out: &[u8]) -> serde_json::Result<Vec<T>> {
    let mut deserializer = serde_json::Deserializer::from_slice(out);
    let entries = list(&mut deserializer)?;
    deserializer.end()?;
    Ok(entries)
}

// ---------- login discovery ----------

/// One entry of "tea logins list --output json". tea prints every value as a
/// string, the default flag included.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TeaLogin {
    #[serde(deserialize_with = "nullable")]
    name: String,
    #[serde(deserialize_with = "nullable")]
    url: String,
    #[serde(deserialize_with = "nullable")]
    default: String,
}

/// Chooses the tea login that serves `host`. tea addresses servers by login
/// name, not by host like gh and glab do, so the mapping has to be discovered.
/// An instance named after a login wins, which is how a user pins one of
/// several accounts on the same host; otherwise the default login wins.
fn pick_login<'a>(logins: &'a [TeaLogin], host: &str, instance: &str) -> Option<&'a TeaLogin> {
    let host = normalize_host(host);
    let mut best = None;
    for login in logins {
        if !eq_fold(&normalize_host(&login.url), &host) {
            continue;
        }
        if login.name == instance {
            return Some(login);
        }
        if best.is_none() || login.default == "true" {
            best = Some(login);
        }
    }
    best
}

/// The version is styled, so the pattern skips lazily over whatever escape
/// sequences tea wrote around it.
static TEA_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Version:(?-u:.)*?([0-9]+)\.([0-9]+)\.([0-9]+)").expect("valid pattern")
});

/// Parses the major and minor version out of "tea --version".
fn tea_version(out: &[u8]) -> Option<(i64, i64)> {
    let captures = TEA_VERSION.captures(out)?;
    // The groups hold digits only, so parsing can fail on overflow alone.
    let number = |group: usize| {
        captures
            .get(group)
            .and_then(|found| std::str::from_utf8(found.as_bytes()).ok())
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(i64::MAX)
    };
    Some((number(1), number(2)))
}

// ---------- transport ----------

/// Why a request failed. The envelope stays apart from every other failure so
/// the authentication hint can be added when /user is the one that answers it.
enum Failure {
    /// The message of the `{"message": …}` envelope Gitea answers errors with.
    Api(String),
    Other(Error),
}

impl Failure {
    fn into_error(self, endpoint: &str) -> Error {
        match self {
            Failure::Api(message) => msg!("tea api %s: %w", endpoint, Error::external(message)),
            Failure::Other(err) => err,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Envelope {
    #[serde(deserialize_with = "nullable")]
    message: String,
}

/// Inspects a response body. tea exits 0 even when the server answers 4xx, so
/// the envelope is the only way to notice a failed request.
fn api_error(out: &[u8]) -> Option<String> {
    let out = out.trim_ascii();
    if out.first() != Some(&b'{') {
        return None;
    }
    let envelope: Envelope = serde_json::from_slice(out).ok()?;
    (!envelope.message.is_empty()).then_some(envelope.message)
}

// ---------- payloads ----------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtUser {
    #[serde(deserialize_with = "nullable")]
    login: String,
    #[serde(deserialize_with = "nullable")]
    username: String,
}

impl GtUser {
    fn name(&self) -> &str {
        if self.login.is_empty() {
            &self.username
        } else {
            &self.login
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtLabel {
    #[serde(deserialize_with = "nullable")]
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtRepo {
    #[serde(deserialize_with = "nullable")]
    full_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtPullRef {
    #[serde(deserialize_with = "nullable")]
    draft: bool,
}

/// What /repos/issues/search returns, for pull requests as much as for issues:
/// no branches, no diff stats, no reviews and no CI.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtIssue {
    #[serde(deserialize_with = "nullable")]
    number: i64,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "nullable")]
    html_url: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    comments: i64,
    #[serde(deserialize_with = "nullable")]
    user: GtUser,
    #[serde(deserialize_with = "list")]
    labels: Vec<GtLabel>,
    #[serde(deserialize_with = "list")]
    assignees: Vec<GtUser>,
    repository: Option<GtRepo>,
    pull_request: Option<GtPullRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtBranch {
    #[serde(rename = "ref", deserialize_with = "nullable")]
    name: String,
    #[serde(deserialize_with = "nullable")]
    sha: String,
    repo: Option<GtRepo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtPull {
    #[serde(deserialize_with = "nullable")]
    draft: bool,
    #[serde(deserialize_with = "nullable")]
    mergeable: bool,
    #[serde(deserialize_with = "nullable")]
    additions: i64,
    #[serde(deserialize_with = "nullable")]
    deletions: i64,
    #[serde(deserialize_with = "nullable")]
    changed_files: i64,
    head: Option<GtBranch>,
    base: Option<GtBranch>,
}

/// The list representation returned by /repos/{owner}/{repo}/pulls. The
/// detail-only fields are filled later by the same enrichment pass used for
/// relation searches.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtRepoPull {
    #[serde(deserialize_with = "nullable")]
    number: i64,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "nullable")]
    html_url: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    comments: i64,
    #[serde(deserialize_with = "nullable")]
    user: GtUser,
    #[serde(deserialize_with = "list")]
    labels: Vec<GtLabel>,
    #[serde(deserialize_with = "list")]
    assignees: Vec<GtUser>,
    #[serde(deserialize_with = "nullable")]
    draft: bool,
}

impl GtRepoPull {
    fn into_issue(self, repo: &str) -> GtIssue {
        GtIssue {
            number: self.number,
            title: self.title,
            body: self.body,
            html_url: self.html_url,
            created_at: self.created_at,
            updated_at: self.updated_at,
            comments: self.comments,
            user: self.user,
            labels: self.labels,
            assignees: self.assignees,
            repository: Some(GtRepo {
                full_name: repo.to_string(),
            }),
            pull_request: Some(GtPullRef { draft: self.draft }),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtReview {
    #[serde(deserialize_with = "nullable")]
    id: i64,
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "nullable")]
    user: GtUser,
    #[serde(deserialize_with = "nullable")]
    dismissed: bool,
    #[serde(deserialize_with = "nullable")]
    comments_count: i64,
    #[serde(deserialize_with = "timestamp")]
    submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtReviewComment {
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "nullable")]
    path: String,
    #[serde(deserialize_with = "nullable")]
    position: i64,
    #[serde(deserialize_with = "nullable")]
    original_position: i64,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    user: GtUser,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtComment {
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    user: GtUser,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtCommitStatus {
    #[serde(deserialize_with = "nullable")]
    status: String,
    #[serde(deserialize_with = "nullable")]
    context: String,
    #[serde(deserialize_with = "nullable")]
    description: String,
    #[serde(deserialize_with = "nullable")]
    target_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GtStatus {
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    total_count: i64,
    #[serde(deserialize_with = "list")]
    statuses: Vec<GtCommitStatus>,
}

// ---------- list ----------

struct Search {
    kind: Kind,
    relation: Relation,
    query: &'static str,
}

/// The six queries behind the PR and issue tabs.
const SEARCHES: [Search; 6] = [
    Search {
        kind: Kind::Pr,
        relation: Relation::REVIEW_REQUESTED,
        query: "type=pulls&review_requested=true",
    },
    Search {
        kind: Kind::Pr,
        relation: Relation::AUTHORED,
        query: "type=pulls&created=true",
    },
    Search {
        kind: Kind::Pr,
        relation: Relation::ASSIGNED,
        query: "type=pulls&assigned=true",
    },
    Search {
        kind: Kind::Issue,
        relation: Relation::ASSIGNED,
        query: "type=issues&assigned=true",
    },
    Search {
        kind: Kind::Issue,
        relation: Relation::AUTHORED,
        query: "type=issues&created=true",
    },
    Search {
        kind: Kind::Issue,
        relation: Relation::MENTIONED,
        query: "type=issues&mentioned=true",
    },
];

impl Gitea {
    pub(super) fn new(instance: Instance, tracked: Vec<String>) -> Self {
        Self {
            instance,
            tracked,
            session: Mutex::default(),
        }
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Also guards against a different program called tea: the name is generic
    /// enough to collide with unrelated tools on PATH.
    fn check_version(&self, ctx: &Ctx) -> Result<()> {
        let out = run(ctx, None, &[], "tea", &["--version"])?;
        let Some((major, minor)) = tea_version(&out) else {
            return Err(msg!(
                "o tea encontrado no PATH não parece ser o CLI do Gitea: %s",
                String::from_utf8_lossy(&out).trim()
            ));
        };
        if major < TEA_MIN_MAJOR || (major == TEA_MIN_MAJOR && minor < TEA_MIN_MINOR) {
            return Err(msg!(
                "tea %d.%d é antigo demais; a integração com Gitea precisa da %d.%d ou mais nova. Veja: %s",
                major,
                minor,
                TEA_MIN_MAJOR,
                TEA_MIN_MINOR,
                install_hint("tea")
            ));
        }
        Ok(())
    }

    /// Finds (once) the tea login for this instance. The lock is held while tea
    /// runs so concurrent callers wait for one discovery instead of repeating it.
    fn resolve(&self, ctx: &Ctx) -> Result<String> {
        let mut session = self.session();
        if !session.login.is_empty() {
            return Ok(session.login.clone());
        }
        self.check_version(ctx)?;
        let out = run(
            ctx,
            None,
            &[],
            "tea",
            &["logins", "list", "--output", "json"],
        )?;
        let logins: Vec<TeaLogin> = decode_list(&out).map_err(invalid)?;
        let Some(login) = pick_login(&logins, &self.instance.host, &self.instance.name) else {
            return Err(msg!(
                "tea não tem login para %s. Rode: tea login add --url https://%s",
                &self.instance.host,
                &self.instance.host
            ));
        };
        session.login = login.name.clone();
        session.base = login.url.trim_end_matches('/').to_string();
        Ok(session.login.clone())
    }

    /// Prefers the URL tea stored, so servers on plain HTTP or on a non-default
    /// port keep working.
    fn server_url(&self) -> String {
        let session = self.session();
        if session.base.is_empty() {
            format!("https://{}", self.instance.host)
        } else {
            session.base.clone()
        }
    }

    /// Makes an authenticated request through tea. Flags go before the endpoint
    /// because tea stops parsing them at the first positional argument.
    fn request(
        &self,
        ctx: &Ctx,
        method: &str,
        endpoint: &str,
        fields: &[&str],
    ) -> std::result::Result<Vec<u8>, Failure> {
        let login = self.resolve(ctx).map_err(Failure::Other)?;
        let mut args = vec!["api", "--login", &login];
        if !method.is_empty() {
            args.extend(["--method", method]);
        }
        args.extend(fields);
        args.push(endpoint);
        let out = run(ctx, None, &[], "tea", &args).map_err(Failure::Other)?;
        match api_error(&out) {
            Some(message) => Err(Failure::Api(message)),
            None => Ok(out),
        }
    }

    fn api(&self, ctx: &Ctx, method: &str, endpoint: &str, fields: &[&str]) -> Result<Vec<u8>> {
        self.request(ctx, method, endpoint, fields)
            .map_err(|failure| failure.into_error(endpoint))
    }

    /// Doubles as the authentication check: when tea has no usable token the
    /// search endpoints still answer 200 with whatever is public, which would
    /// look like an empty list instead of an error. /user does not.
    fn current_user(&self, ctx: &Ctx) -> Result<String> {
        let out = match self.request(ctx, "", "/user", &[]) {
            Ok(out) => out,
            Err(Failure::Api(message)) => {
                return Err(msg!(
                    "tea não autenticado em %s (%s). Rode: tea login add --url %s",
                    &self.instance.host,
                    message,
                    self.server_url()
                ));
            }
            Err(Failure::Other(err)) => return Err(err),
        };
        let user: GtUser = decode(&out).map_err(invalid)?;
        Ok(user.name().to_string())
    }

    fn search(&self, ctx: &Ctx, search: &Search) -> Result<Vec<GtIssue>> {
        let endpoint = format!(
            "/repos/issues/search?state=open&limit={PAGE_SIZE}&{}",
            search.query
        );
        let out = self.api(ctx, "", &endpoint, &[])?;
        decode_list(&out).map_err(invalid)
    }

    fn convert(&self, issue: GtIssue, repo: &str, kind: Kind) -> Item {
        Item {
            kind,
            instance: self.instance.name.clone(),
            provider: Provider::Gitea,
            host: self.instance.host.clone(),
            repo: repo.to_string(),
            repo_url: format!("{}/{repo}", self.server_url()),
            number: issue.number,
            title: issue.title,
            url: issue.html_url,
            author: issue.user.name().to_string(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            comments: issue.comments,
            labels: issue.labels.into_iter().map(|label| label.name).collect(),
            assignees: issue
                .assignees
                .iter()
                .map(|user| user.name().to_string())
                .collect(),
            draft: issue.pull_request.is_some_and(|pull| pull.draft),
            // Gitea aggregates no review decision; enrich replaces this with
            // what the reviews actually say.
            review: if kind == Kind::Pr {
                REVIEW_REQUIRED.to_string()
            } else {
                String::new()
            },
            ..Item::default()
        }
    }

    /// Fills in what the search leaves out. There is no bulk endpoint for any
    /// of it, so this costs up to three requests per pull request. Failures are
    /// tolerated on purpose: a row without CI beats losing the whole list.
    fn enrich(&self, ctx: &Ctx, items: &mut [Item], me: &str) {
        let pulls = items.iter().filter(|item| !item.is_issue()).count();
        let queue = Mutex::new(items.iter_mut().filter(|item| !item.is_issue()));
        thread::scope(|scope| {
            for _ in 0..pulls.min(GITEA_WORKERS) {
                scope.spawn(|| {
                    while ctx.err().is_none() {
                        let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
                        let Some(pr) = next else { return };
                        self.fill_pull(ctx, pr, me);
                    }
                });
            }
        });
    }

    fn fill_pull(&self, ctx: &Ctx, pr: &mut Item, me: &str) {
        let endpoint = format!("/repos/{}/pulls/{}", pr.repo, pr.number);
        let Ok(out) = self.api(ctx, "", &endpoint, &[]) else {
            return;
        };
        let Ok(detail) = decode::<GtPull>(&out) else {
            return;
        };
        apply_pull(pr, &detail);
        if let Some(sha) = detail
            .head
            .as_ref()
            .map(|head| &head.sha)
            .filter(|sha| !sha.is_empty())
        {
            let endpoint = format!("/repos/{}/commits/{sha}/status", pr.repo);
            if let Ok(out) = self.api(ctx, "", &endpoint, &[])
                && let Ok(status) = decode::<GtStatus>(&out)
            {
                self.apply_status(pr, &status);
            }
        }
        let endpoint = format!("/repos/{}/pulls/{}/reviews", pr.repo, pr.number);
        if let Ok(out) = self.api(ctx, "", &endpoint, &[])
            && let Ok(reviews) = decode_list::<GtReview>(&out)
        {
            apply_reviews(pr, &reviews, me);
        }
    }

    fn apply_status(&self, pr: &mut Item, status: &GtStatus) {
        if status.total_count == 0 {
            return;
        }
        pr.ci = gt_state(&status.state);
        for entry in &status.statuses {
            let name = if entry.context.is_empty() {
                &entry.description
            } else {
                &entry.context
            };
            pr.checks.push(Check {
                name: name.clone(),
                state: gt_state(&entry.status),
                url: self.absolute(&entry.target_url),
            });
        }
    }

    /// Completes the relative target_url that Gitea Actions write.
    fn absolute(&self, url: &str) -> String {
        if url.starts_with('/') {
            format!("{}{url}", self.server_url())
        } else {
            url.to_string()
        }
    }

    /// Adds the review bodies and the inline code comments, which Gitea keeps
    /// out of the issue comment list.
    fn append_reviews(&self, ctx: &Ctx, item: &Item, thread: &mut Thread) -> Result<()> {
        let endpoint = format!("/repos/{}/pulls/{}/reviews", item.repo, item.number);
        let out = self.api(ctx, "", &endpoint, &[])?;
        let reviews: Vec<GtReview> = decode_list(&out).map_err(invalid)?;
        for review in reviews {
            // A draft review, visible only to its author.
            if eq_fold(&review.state, "PENDING") {
                continue;
            }
            let state = gt_review_state(&review);
            if !review.body.is_empty()
                || state == REVIEW_APPROVED
                || state == REVIEW_CHANGES_REQUESTED
            {
                thread.comments.push(Comment {
                    author: review.user.name().to_string(),
                    body: review.body,
                    created_at: review.submitted_at,
                    review: state.to_string(),
                    ..Comment::default()
                });
            }
            if review.comments_count == 0 {
                continue;
            }
            let endpoint = format!(
                "/repos/{}/pulls/{}/reviews/{}/comments",
                item.repo, item.number, review.id
            );
            let out = self.api(ctx, "", &endpoint, &[])?;
            let inline: Vec<GtReviewComment> = decode_list(&out).map_err(invalid)?;
            for comment in inline {
                let line = if comment.position == 0 {
                    comment.original_position
                } else {
                    comment.position
                };
                thread.comments.push(Comment {
                    author: comment.user.name().to_string(),
                    body: comment.body,
                    created_at: comment.created_at,
                    path: comment.path,
                    line,
                    ..Comment::default()
                });
            }
        }
        Ok(())
    }
}

/// Copies the fields only the pull request endpoint carries.
fn apply_pull(pr: &mut Item, detail: &GtPull) {
    pr.draft = detail.draft;
    pr.additions = detail.additions;
    pr.deletions = detail.deletions;
    pr.files = detail.changed_files;
    if let Some(head) = &detail.head {
        pr.source_branch = head.name.clone();
    }
    if let Some(base) = &detail.base {
        pr.target_branch = base.name.clone();
    }
    let repo = |branch: &Option<GtBranch>| branch.as_ref().and_then(|branch| branch.repo.clone());
    if let (Some(head), Some(base)) = (repo(&detail.head), repo(&detail.base)) {
        pr.from_fork = !eq_fold(&head.full_name, &base.full_name);
    }
    // Gitea also reports drafts, and pull requests whose conflict check is
    // still running, as not mergeable. Drafts are filtered out here; a check in
    // flight resolves itself on the next refresh.
    pr.conflicts = !detail.mergeable && !detail.draft;
}

fn gt_state(state: &str) -> CiState {
    match state.to_lowercase().as_str() {
        "success" | "warning" => CiState::Success,
        "failure" | "error" => CiState::Failure,
        "skipped" => CiState::Canceled,
        "pending" => CiState::Pending,
        _ => CiState::None,
    }
}

/// Derives the review state Gitea does not aggregate. Reviews come back oldest
/// first, so the last one of each user is the one that counts.
fn apply_reviews(pr: &mut Item, reviews: &[GtReview], me: &str) {
    let mut approved: HashMap<&str, bool> = HashMap::new();
    let mut order = Vec::new();
    for review in reviews {
        let name = review.user.name();
        if name.is_empty() || review.dismissed {
            continue;
        }
        // Plain comments and unsubmitted drafts decide nothing.
        let approval = match review.state.to_uppercase().as_str() {
            "APPROVED" => true,
            "REQUEST_CHANGES" => false,
            _ => continue,
        };
        if approved.insert(name, approval).is_none() {
            order.push(name);
        }
    }
    let mut changes = false;
    pr.approved_by.clear();
    pr.approved_by_me = false;
    for name in order {
        if !approved[name] {
            changes = true;
            continue;
        }
        pr.approved_by.push(name.to_string());
        if eq_fold(name, me) {
            pr.approved_by_me = true;
        }
    }
    pr.review = if changes {
        REVIEW_CHANGES_REQUESTED
    } else if pr.approved_by.is_empty() {
        REVIEW_REQUIRED
    } else {
        REVIEW_APPROVED
    }
    .to_string();
}

/// Maps a Gitea review to the vocabulary the UI renders.
fn gt_review_state(review: &GtReview) -> &'static str {
    if review.dismissed {
        return "dismissed";
    }
    match review.state.to_uppercase().as_str() {
        "APPROVED" => REVIEW_APPROVED,
        "REQUEST_CHANGES" => REVIEW_CHANGES_REQUESTED,
        "COMMENT" => "commented",
        _ => "",
    }
}

// ---------- draft ----------

#[derive(Default, Deserialize)]
#[serde(default)]
struct GtDraftPayload {
    #[serde(deserialize_with = "nullable")]
    title: String,
    draft: Option<bool>,
    content_version: Option<i64>,
}

struct DraftState {
    title: String,
    draft: bool,
    content_version: Option<i64>,
}

fn decode_draft_state(data: &[u8]) -> Result<DraftState> {
    let payload: GtDraftPayload = decode(data).map_err(invalid)?;
    match payload.draft {
        Some(draft) if !payload.title.is_empty() => Ok(DraftState {
            title: payload.title,
            draft,
            content_version: payload.content_version,
        }),
        _ => Err(msg!("resposta do Gitea sem título ou estado de draft")),
    }
}

fn gitea_ready_title(original: &str) -> Result<&str> {
    const PREFIXES: [&str; 4] = ["WIP:", "[WIP]", "Draft:", "[Draft]"];
    let mut title = original;
    // The prefixes are ASCII, so a byte-wise match always ends on a character
    // boundary.
    while let Some(prefix) = PREFIXES.iter().find(|prefix| {
        title
            .as_bytes()
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    }) {
        title = title[prefix.len()..].trim_start_matches([' ', '\t']);
    }
    if title == original || title.trim().is_empty() {
        return Err(msg!(
            "prefixo de draft do Gitea não reconhecido; altere o estado no servidor"
        ));
    }
    Ok(title)
}

// ---------- client ----------

impl Client for Gitea {
    fn instance(&self) -> &Instance {
        &self.instance
    }

    fn tool(&self) -> &'static str {
        "tea"
    }

    fn list(&self, ctx: &Ctx) -> Result<Vec<Item>> {
        let me = self.current_user(ctx)?;
        let found: Vec<Result<Vec<GtIssue>>> = thread::scope(|scope| {
            let searches: Vec<_> = SEARCHES
                .iter()
                .map(|search| scope.spawn(move || self.search(ctx, search)))
                .collect();
            searches
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        });
        let mut acc = Accumulator::default();
        let mut results = Vec::with_capacity(found.len());
        for result in found {
            results.push(result?);
        }
        for (search, issues) in SEARCHES.iter().zip(results) {
            for mut issue in issues {
                let Some(repo) = issue.repository.take() else {
                    continue;
                };
                acc.add(
                    self.convert(issue, &repo.full_name, search.kind),
                    search.relation,
                );
            }
        }
        for repo in &self.tracked {
            let mut seen = HashSet::new();
            for page in 1.. {
                let endpoint =
                    format!("/repos/{repo}/pulls?state=open&limit={PAGE_SIZE}&page={page}");
                let out = self.api(ctx, "", &endpoint, &[])?;
                let pulls: Vec<GtRepoPull> = decode_list(&out).map_err(|err| {
                    msg!(
                        "resposta inválida do tea para %s: %w",
                        repo,
                        Error::from(err)
                    )
                })?;
                let count = pulls.len();
                let mut new_page_item = false;
                for pull in pulls {
                    new_page_item |= seen.insert(pull.number);
                    acc.add(
                        self.convert(pull.into_issue(repo), repo, Kind::Pr),
                        Relation::NONE,
                    );
                }
                // A server that ignores the page parameter would repeat the
                // same pull requests forever.
                if count < PAGE_SIZE || !new_page_item {
                    break;
                }
            }
        }
        let mut items = acc.list();
        self.enrich(ctx, &mut items, &me);
        Ok(items)
    }

    fn thread(&self, ctx: &Ctx, item: &Item) -> Result<Thread> {
        // Pull requests are issues in Gitea, so the description and the plain
        // comments come from the same endpoints for both kinds.
        let endpoint = format!("/repos/{}/issues/{}", item.repo, item.number);
        let out = self.api(ctx, "", &endpoint, &[])?;
        let head: GtIssue = decode(&out).map_err(invalid)?;
        let mut thread = Thread {
            body: head.body,
            comments: Vec::new(),
        };
        let endpoint = format!("/repos/{}/issues/{}/comments", item.repo, item.number);
        let out = self.api(ctx, "", &endpoint, &[])?;
        let comments: Vec<GtComment> = decode_list(&out).map_err(invalid)?;
        for comment in comments {
            thread.comments.push(Comment {
                author: comment.user.name().to_string(),
                body: comment.body,
                created_at: comment.created_at,
                ..Comment::default()
            });
        }
        if !item.is_issue() {
            self.append_reviews(ctx, item, &mut thread)?;
        }
        sort_comments(&mut thread.comments);
        Ok(thread)
    }

    fn add_comment(&self, ctx: &Ctx, item: &Item, body: &str) -> Result<()> {
        let endpoint = format!("/repos/{}/issues/{}/comments", item.repo, item.number);
        self.api(ctx, "POST", &endpoint, &["-f", &format!("body={body}")])?;
        Ok(())
    }

    fn approve(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        let login = self.resolve(ctx)?;
        let number = pr.number.to_string();
        run(
            ctx,
            None,
            &[],
            "tea",
            &[
                "pulls", "approve", &number, "--repo", &pr.repo, "--login", &login,
            ],
        )?;
        Ok(())
    }

    /// Goes through the API because "tea pulls merge" only exposes --style,
    /// while the instance settings also offer deleting the source branch and
    /// merging as soon as the checks pass.
    fn merge(&self, ctx: &Ctx, pr: &Item, options: &MergeOptions) -> Result<()> {
        let method = merge_method(&options.method);
        let endpoint = format!("/repos/{}/pulls/{}/merge", pr.repo, pr.number);
        let fields = [
            // Gitea binds the field as "Do"; 1.23 and newer also accept "do".
            "-f",
            &format!("Do={method}"),
            "-f",
            &format!("do={method}"),
            "-F",
            &format!("delete_branch_after_merge={}", options.delete_branch),
            "-F",
            &format!("merge_when_checks_succeed={}", options.auto),
        ];
        self.api(ctx, "POST", &endpoint, &fields)?;
        Ok(())
    }

    /// Closes the pull request and leaves the source branch alone.
    fn close(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        let login = self.resolve(ctx)?;
        let number = pr.number.to_string();
        run(
            ctx,
            None,
            &[],
            "tea",
            &[
                "pulls", "close", &number, "--repo", &pr.repo, "--login", &login,
            ],
        )?;
        Ok(())
    }

    /// Gitea derives draft from the title. Fetch the current title before
    /// changing its prefix so a cached item cannot overwrite a newer title.
    fn set_draft(&self, ctx: &Ctx, pr: &Item, draft: bool) -> Result<()> {
        let endpoint = format!("/repos/{}/pulls/{}", pr.repo, pr.number);
        let data = self.api(ctx, "GET", &endpoint, &[])?;
        let state = decode_draft_state(&data)?;
        if state.draft == draft {
            return Ok(());
        }
        let title = if draft {
            format!("WIP: {}", state.title)
        } else {
            gitea_ready_title(&state.title)?.to_string()
        };
        let title = format!("title={title}");
        let version = state
            .content_version
            .map(|version| format!("content_version={version}"));
        let mut fields = vec!["-f", title.as_str()];
        if let Some(version) = &version {
            fields.extend(["-F", version.as_str()]);
        }
        let data = self.api(ctx, "PATCH", &endpoint, &fields)?;
        let updated = decode_draft_state(&data)?;
        if updated.draft != draft {
            return Err(msg!(
                "o Gitea não aplicou o estado de draft solicitado; verifique os prefixos configurados no servidor"
            ));
        }
        Ok(())
    }

    fn update_branch(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        let endpoint = format!("/repos/{}/pulls/{}/update?style=merge", pr.repo, pr.number);
        self.api(ctx, "POST", &endpoint, &[])?;
        Ok(())
    }

    /// Runs inside the clone so tea resolves the repository from its remote,
    /// the same way gh and glab do.
    fn checkout(&self, ctx: &Ctx, pr: &Item, dir: &Path) -> Result<()> {
        let login = self.resolve(ctx)?;
        let number = pr.number.to_string();
        run(
            ctx,
            Some(dir),
            &[],
            "tea",
            &["pulls", "checkout", &number, "--branch", "--login", &login],
        )?;
        Ok(())
    }

    /// The ref Gitea keeps on the base repository, forks included.
    fn head_ref(&self, pr: &Item) -> String {
        format!("refs/pull/{}/head", pr.number)
    }

    fn auth_status(&self, ctx: &Ctx) -> Result<()> {
        self.current_user(ctx).map(|_| ())
    }
}

/// One instance per tea login. Gitea is self-hosted far more often than not,
/// so there is no SaaS host worth probing blindly: what the user already
/// logged tea into is the useful seed.
pub fn gitea_instances(ctx: &Ctx) -> Vec<Instance> {
    if !tool_available("tea") {
        return Vec::new();
    }
    let Ok(out) = run(
        ctx,
        None,
        &[],
        "tea",
        &["logins", "list", "--output", "json"],
    ) else {
        return Vec::new();
    };
    let logins: Vec<TeaLogin> = decode_list(&out).unwrap_or_default();
    logins
        .into_iter()
        .filter_map(|login| {
            let host = normalize_host(&login.url);
            (!host.is_empty()).then(|| Instance {
                name: login.name,
                provider: Provider::Gitea,
                host,
                ..Instance::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(name: &str, host: &str) -> Instance {
        Instance {
            name: name.into(),
            provider: Provider::Gitea,
            host: host.into(),
            ..Instance::default()
        }
    }

    fn client_with(instance: Instance, login: &str, base: &str) -> Gitea {
        let client = Gitea::new(instance, Vec::new());
        *client.session() = Session {
            login: login.into(),
            base: base.into(),
        };
        client
    }

    fn gitea_client() -> Gitea {
        client_with(instance("corp", "git.corp"), "", "https://git.corp")
    }

    fn pr() -> Item {
        Item {
            kind: Kind::Pr,
            ..Item::default()
        }
    }

    /// Trimmed from a real /repos/issues/search answer.
    const SEARCH_PAYLOAD: &str = r#"{
  "number": 6,
  "title": "reescreve a aplicação",
  "html_url": "https://git.corp/acme/app/pulls/6",
  "created_at": "2026-09-19T10:00:00-03:00",
  "updated_at": "2026-09-20T09:07:13-03:00",
  "comments": 3,
  "user": {"login": "ana", "username": "ana"},
  "labels": [{"name": "bug"}, {"name": "go"}],
  "assignees": [{"login": "bruno", "username": "bruno"}],
  "repository": {"id": 3, "name": "app", "owner": "acme", "full_name": "acme/app"},
  "pull_request": {"merged": false, "draft": true, "html_url": "https://git.corp/acme/app/pulls/6"}
}"#;

    #[test]
    fn client_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Gitea>();
    }

    #[test]
    fn convert() {
        let issue: GtIssue = serde_json::from_str(SEARCH_PAYLOAD).unwrap();
        let repo = issue.repository.clone().unwrap().full_name;
        let client = gitea_client();
        let pr = client.convert(issue.clone(), &repo, Kind::Pr);
        assert_eq!(pr.repo, "acme/app");
        assert_eq!(pr.repo_url, "https://git.corp/acme/app");
        assert_eq!((pr.number, pr.reference().as_str()), (6, "#6"));
        assert_eq!(pr.author, "ana");
        assert_eq!(pr.labels, ["bug", "go"]);
        assert_eq!(pr.assignees, ["bruno"]);
        assert!(pr.draft, "draft not carried over from pull_request");
        assert_eq!(pr.comments, 3);
        assert_eq!(
            pr.updated_at.map(|at| at.to_rfc3339()).as_deref(),
            Some("2026-09-20T12:07:13+00:00")
        );
        // The search says nothing about reviews, so a pull request starts as
        // pending until enrichment says otherwise.
        assert_eq!(pr.review, REVIEW_REQUIRED);
        let issue = client.convert(issue, &repo, Kind::Issue);
        assert!(issue.is_issue());
        assert_eq!(issue.review, "");
        assert_ne!(
            issue.key(),
            pr.key(),
            "issue and pull request must not share a key"
        );
        assert_eq!(client.head_ref(&pr), "refs/pull/6/head");
    }

    #[test]
    fn apply_pull_branches_forks_and_conflicts() {
        // (name, payload, conflicts, fork)
        let cases = [
            (
                "mesmo repo, sem conflito",
                r#"{"draft": false, "mergeable": true, "additions": 9, "deletions": 5, "changed_files": 2,
                    "head": {"ref": "feat", "sha": "abc", "repo": {"full_name": "acme/app"}},
                    "base": {"ref": "main", "sha": "def", "repo": {"full_name": "acme/app"}}}"#,
                false,
                false,
            ),
            (
                "conflito",
                r#"{"draft": false, "mergeable": false,
                    "head": {"ref": "feat", "repo": {"full_name": "acme/app"}},
                    "base": {"ref": "main", "repo": {"full_name": "acme/app"}}}"#,
                true,
                false,
            ),
            // Gitea reports every draft as not mergeable; flagging them all as
            // conflicting would be wrong.
            (
                "draft não é conflito",
                r#"{"draft": true, "mergeable": false,
                    "head": {"ref": "feat", "repo": {"full_name": "acme/app"}},
                    "base": {"ref": "main", "repo": {"full_name": "acme/app"}}}"#,
                false,
                false,
            ),
            (
                "fork",
                r#"{"draft": false, "mergeable": true,
                    "head": {"ref": "feat", "repo": {"full_name": "ana/app"}},
                    "base": {"ref": "main", "repo": {"full_name": "acme/app"}}}"#,
                false,
                true,
            ),
        ];
        for (name, payload, conflicts, fork) in cases {
            let detail: GtPull = serde_json::from_str(payload).unwrap();
            let mut pr = pr();
            apply_pull(&mut pr, &detail);
            assert_eq!((pr.conflicts, pr.from_fork), (conflicts, fork), "{name}");
            assert_eq!(
                (pr.source_branch.as_str(), pr.target_branch.as_str()),
                ("feat", "main"),
                "{name}"
            );
        }
    }

    #[test]
    fn apply_pull_diff_stats() {
        let detail: GtPull = serde_json::from_str(
            r#"{"mergeable": true, "additions": 9809, "deletions": 5715, "changed_files": 75}"#,
        )
        .unwrap();
        let mut pr = Item::default();
        apply_pull(&mut pr, &detail);
        assert_eq!((pr.additions, pr.deletions, pr.files), (9809, 5715, 75));
    }

    #[test]
    fn apply_reviews_aggregates_the_decision() {
        // (name, reviews, review, approved by, approved by me)
        let cases: [(&str, &str, &str, &[&str], bool); 6] = [
            ("sem reviews", "[]", REVIEW_REQUIRED, &[], false),
            (
                "aprovado por mim, ignorando maiúsculas",
                r#"[{"state": "APPROVED", "user": {"login": "Me"}}]"#,
                REVIEW_APPROVED,
                &["Me"],
                true,
            ),
            // The last review of each user decides, so an approval that came
            // after a rejection wins.
            (
                "última review do usuário vence",
                r#"[{"state": "REQUEST_CHANGES", "user": {"login": "ana"}},
                    {"state": "APPROVED", "user": {"login": "ana"}}]"#,
                REVIEW_APPROVED,
                &["ana"],
                false,
            ),
            (
                "alterações pedidas por um revisor bastam",
                r#"[{"state": "APPROVED", "user": {"login": "ana"}},
                    {"state": "REQUEST_CHANGES", "user": {"login": "bruno"}}]"#,
                REVIEW_CHANGES_REQUESTED,
                &["ana"],
                false,
            ),
            (
                "review descartada não conta",
                r#"[{"state": "REQUEST_CHANGES", "dismissed": true, "user": {"login": "ana"}}]"#,
                REVIEW_REQUIRED,
                &[],
                false,
            ),
            (
                "comentário não decide nada",
                r#"[{"state": "COMMENT", "user": {"login": "ana"}}]"#,
                REVIEW_REQUIRED,
                &[],
                false,
            ),
        ];
        for (name, reviews, review, approved_by, approved_by_me) in cases {
            let reviews: Vec<GtReview> = decode_list(reviews.as_bytes()).unwrap();
            let mut pr = pr();
            apply_reviews(&mut pr, &reviews, "me");
            assert_eq!(
                (pr.review.as_str(), pr.approved_by_me),
                (review, approved_by_me),
                "{name}"
            );
            assert_eq!(pr.approved_by, approved_by, "{name}");
        }
    }

    /// Trimmed from a real /commits/{sha}/status answer: Gitea Actions write a
    /// relative target_url.
    const STATUS_PAYLOAD: &str = r#"{
  "state": "pending",
  "total_count": 2,
  "statuses": [
    {"status": "success", "context": "ci / lint (push)", "target_url": "/acme/app/actions/runs/1/jobs/1"},
    {"status": "pending", "context": "ci / test (push)", "target_url": "/acme/app/actions/runs/1/jobs/2"}
  ]
}"#;

    #[test]
    fn apply_status() {
        let status: GtStatus = serde_json::from_str(STATUS_PAYLOAD).unwrap();
        let mut item = pr();
        gitea_client().apply_status(&mut item, &status);
        assert_eq!((item.ci, item.checks.len()), (CiState::Pending, 2));
        assert_eq!(item.checks[0].name, "ci / lint (push)");
        assert_eq!(item.checks[0].state, CiState::Success);
        assert_eq!(
            item.checks[0].url, "https://git.corp/acme/app/actions/runs/1/jobs/1",
            "relative target_url was not completed"
        );

        let empty: GtStatus =
            serde_json::from_str(r#"{"state": "pending", "total_count": 0, "statuses": []}"#)
                .unwrap();
        let mut item = pr();
        gitea_client().apply_status(&mut item, &empty);
        assert_eq!(
            item.ci,
            CiState::None,
            "a commit with no statuses must have no CI"
        );
        assert!(item.checks.is_empty());
    }

    #[test]
    fn states() {
        for (input, want) in [
            ("success", CiState::Success),
            ("warning", CiState::Success),
            ("failure", CiState::Failure),
            ("error", CiState::Failure),
            ("skipped", CiState::Canceled),
            ("pending", CiState::Pending),
            ("", CiState::None),
        ] {
            assert_eq!(gt_state(input), want, "gt_state({input:?})");
        }
    }

    #[test]
    fn review_state() {
        let review = |state: &str, dismissed| GtReview {
            state: state.into(),
            dismissed,
            ..GtReview::default()
        };
        for (review, want) in [
            (review("APPROVED", false), REVIEW_APPROVED),
            (review("REQUEST_CHANGES", false), REVIEW_CHANGES_REQUESTED),
            (review("COMMENT", false), "commented"),
            (review("APPROVED", true), "dismissed"),
        ] {
            assert_eq!(gt_review_state(&review), want, "{review:?}");
        }
    }

    /// tea exits 0 even when the server answers 4xx, so the envelope is the
    /// only signal that a request failed.
    #[test]
    fn api_error_envelope() {
        let message =
            api_error(br#"{"message":"token is required","url":"https://git.corp/api/swagger"}"#);
        assert_eq!(message.as_deref(), Some("token is required"));
        let failure = Failure::Api(message.unwrap());
        assert!(
            matches!(&failure, Failure::Api(message) if message == "token is required"),
            "the envelope must stay recognizable so the auth hint can be added"
        );
        assert_eq!(
            failure.into_error("/user").text("en"),
            "tea api /user: token is required"
        );
        for body in [
            "",
            "[]",
            r#"[{"number":1}]"#,
            r#"{"login":"ana"}"#,
            "null",
            r#"{"message":null}"#,
        ] {
            assert_eq!(api_error(body.as_bytes()), None, "api_error({body:?})");
        }
    }

    #[test]
    fn pick_login_prefers_the_pinned_then_the_default_account() {
        let login = |name: &str, url: &str, default: &str| TeaLogin {
            name: name.into(),
            url: url.into(),
            default: default.into(),
        };
        let logins = [
            login("pessoal", "https://git.corp", ""),
            login("corp", "https://git.corp/", "true"),
            login("outro", "https://git.outra.org", ""),
        ];
        let name = |host, instance| pick_login(&logins, host, instance).map(|l| l.name.as_str());
        // An instance named after a login pins that account.
        assert_eq!(name("git.corp", "pessoal"), Some("pessoal"));
        // Otherwise the default login for the host wins, trailing slash and all.
        assert_eq!(name("git.corp", "sem-nome"), Some("corp"));
        assert_eq!(name("https://git.outra.org", "x"), Some("outro"));
        assert_eq!(
            name("git.desconhecido", "x"),
            None,
            "a host tea does not know must not match a login"
        );
    }

    #[test]
    fn tea_version_parsing() {
        // tea styles the version, so the escape sequences have to be tolerated.
        assert_eq!(
            tea_version(b"Version: \x1b[1m0.15.1\x1b[0m\tgolang: 1.26.5\tgo-sdk: v1.2.0"),
            Some((0, 15))
        );
        assert_eq!(tea_version(b"Version: 1.2.3"), Some((1, 2)));
        // Another program called tea must not be mistaken for the Gitea CLI.
        assert_eq!(
            tea_version(b"tea 1.0.0\nthe package manager"),
            None,
            "a foreign tea binary must not parse as the Gitea CLI"
        );
    }

    #[test]
    fn absolute() {
        let client = gitea_client();
        assert_eq!(
            client.absolute("/acme/app/actions/runs/1"),
            "https://git.corp/acme/app/actions/runs/1"
        );
        assert_eq!(
            client.absolute("https://ci.corp/build/1"),
            "https://ci.corp/build/1",
            "absolute URLs must be left alone"
        );
        // Without a resolved login the host from the configuration is the fallback.
        let plain = Gitea::new(instance("", "git.corp"), Vec::new());
        assert_eq!(plain.server_url(), "https://git.corp");
    }

    #[test]
    fn null_and_zero_values_decode_like_missing_ones() {
        let issue: GtIssue = serde_json::from_str(
            r#"{"number": 1, "title": null, "body": null, "created_at": null,
                "updated_at": "0001-01-01T00:00:00Z", "user": null, "labels": null,
                "assignees": [null, {"username": "ana"}], "repository": null, "pull_request": null}"#,
        )
        .unwrap();
        assert_eq!((issue.title.as_str(), issue.user.name()), ("", ""));
        assert_eq!((issue.created_at, issue.updated_at), (None, None));
        assert!(issue.labels.is_empty() && issue.repository.is_none());
        let names: Vec<_> = issue.assignees.iter().map(GtUser::name).collect();
        assert_eq!(names, ["", "ana"]);
        assert!(decode_list::<GtReview>(b"null").unwrap().is_empty());
        assert!(decode_list::<GtReview>(b"").is_err());
        assert!(serde_json::from_str::<GtComment>(r#"{"created_at": "ontem"}"#).is_err());
    }

    #[test]
    fn ready_title_strips_every_known_prefix() {
        assert_eq!(gitea_ready_title("[wip] New title").unwrap(), "New title");
        assert_eq!(gitea_ready_title("WIP: Draft:\t[draft]é").unwrap(), "é");
        for title in ["New title", "Custom: New title", "WIP: ", "é"] {
            assert!(gitea_ready_title(title).is_err(), "{title:?}");
        }
    }

    #[cfg(unix)]
    mod cli {
        use std::collections::BTreeMap;
        use std::fs;
        use std::process::Command;

        use super::*;

        /// A stand-in for tea: a shell script that records each invocation and
        /// answers with canned responses.
        struct FakeTea {
            dir: tempfile::TempDir,
            ctx: Ctx,
        }

        impl FakeTea {
            /// `respond` is the shell fragment that prints the answer; it can
            /// read "$dir" (the fake's own directory) and the arguments.
            fn new(respond: &str) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("tea");
                // One line per invocation: the shell may flush each line on its
                // own, and concurrent calls must not interleave their arguments.
                let script = format!(
                    "#!/bin/sh\ndir=$(dirname \"$0\")\nseparator=$(printf '\\037')\nrecord=\"\"\n\
                     for arg in \"$@\"; do record=\"$record$arg$separator\"; done\n\
                     printf '%s\\n' \"$record\" >> \"$dir/calls\"\n{respond}\n"
                );
                // A child writes the script: a descriptor opened for writing in
                // this process could leak into a concurrently forked child and
                // make executing the script fail with ETXTBSY.
                let status = Command::new("sh")
                    .args(["-c", r#"printf '%s' "$2" > "$1" && chmod 755 "$1""#, "sh"])
                    .arg(&path)
                    .arg(script)
                    .status()
                    .unwrap();
                assert!(status.success());
                let paths =
                    BTreeMap::from([("tea".to_string(), path.to_str().unwrap().to_string())]);
                Self {
                    dir,
                    ctx: Ctx::background().with_paths(&paths),
                }
            }

            fn file(&self, name: &str, content: &str) {
                fs::write(self.dir.path().join(name), content).unwrap();
            }

            fn calls(&self) -> Vec<Vec<String>> {
                let log = fs::read_to_string(self.dir.path().join("calls")).unwrap_or_default();
                log.lines()
                    .map(|line| line.split_terminator('\x1f').map(str::to_string).collect())
                    .collect()
            }
        }

        /// Answers by HTTP method from the files GET and PATCH, and fails every
        /// call once a file named fail exists.
        const BY_METHOD: &str = r#"
if [ -e "$dir/fail" ]; then echo 'permission denied' >&2; exit 1; fi
method=""; previous=""
for arg in "$@"; do
  if [ "$previous" = "--method" ]; then method=$arg; fi
  previous=$arg
done
case "$method" in
  GET) cat "$dir/GET" ;;
  PATCH) cat "$dir/PATCH" ;;
  *) echo '{}' ;;
esac
"#;

        fn strings(args: &[&str]) -> Vec<String> {
            args.iter().map(|arg| arg.to_string()).collect()
        }

        #[test]
        fn draft_transitions() {
            // (name, before, after, title, draft, fail, calls)
            let cases = [
                (
                    "draft",
                    r#"{"title":"New title","draft":false,"content_version":7}"#,
                    r#"{"title":"WIP: New title","draft":true}"#,
                    "WIP: New title",
                    true,
                    false,
                    2,
                ),
                (
                    "ready",
                    r#"{"title":"[wip] New title","draft":true}"#,
                    r#"{"title":"New title","draft":false}"#,
                    "New title",
                    false,
                    false,
                    2,
                ),
                (
                    "already ready",
                    r#"{"title":"New title","draft":false}"#,
                    "",
                    "",
                    false,
                    false,
                    1,
                ),
                (
                    "custom prefix",
                    r#"{"title":"Custom: New title","draft":true}"#,
                    "",
                    "",
                    false,
                    true,
                    1,
                ),
                (
                    "missing state",
                    r#"{"title":"New title"}"#,
                    "",
                    "",
                    true,
                    true,
                    1,
                ),
                (
                    "unrecognized WIP marker",
                    r#"{"title":"New title","draft":false}"#,
                    r#"{"title":"WIP: New title","draft":false}"#,
                    "WIP: New title",
                    true,
                    true,
                    2,
                ),
            ];
            for (name, before, after, title, draft, fail, want_calls) in cases {
                let tea = FakeTea::new(BY_METHOD);
                tea.file("GET", before);
                tea.file("PATCH", after);
                let client = client_with(
                    instance("corp", "git.corp"),
                    "corp-account",
                    "https://git.corp",
                );
                let item = Item {
                    repo: "team/service".into(),
                    number: 9,
                    title: "Stale cached title".into(),
                    ..Item::default()
                };
                let result = client.set_draft(&tea.ctx, &item, draft);
                assert_eq!(result.is_err(), fail, "{name}: {result:?}");
                let calls = tea.calls();
                assert_eq!(calls.len(), want_calls, "{name}: {calls:?}");
                assert_eq!(
                    calls[0],
                    strings(&[
                        "api",
                        "--login",
                        "corp-account",
                        "--method",
                        "GET",
                        "/repos/team/service/pulls/9"
                    ]),
                    "{name}"
                );
                if want_calls == 2 {
                    let title = format!("title={title}");
                    let mut want = vec![
                        "api",
                        "--login",
                        "corp-account",
                        "--method",
                        "PATCH",
                        "-f",
                        &title,
                    ];
                    if before.contains("content_version") {
                        want.extend(["-F", "content_version=7"]);
                    }
                    want.push("/repos/team/service/pulls/9");
                    assert_eq!(calls[1], strings(&want), "{name}");
                }
            }
        }

        #[test]
        fn update_branch_command() {
            for fail in [false, true] {
                let tea = FakeTea::new(BY_METHOD);
                if fail {
                    tea.file("fail", "");
                }
                let client = client_with(instance("work", "git.company.com"), "work", "");
                let item = Item {
                    repo: "team/service".into(),
                    number: 42,
                    ..Item::default()
                };
                let result = client.update_branch(&tea.ctx, &item);
                assert_eq!(result.is_err(), fail, "{result:?}");
                assert_eq!(
                    tea.calls(),
                    [strings(&[
                        "api",
                        "--login",
                        "work",
                        "--method",
                        "POST",
                        "/repos/team/service/pulls/42/update?style=merge"
                    ])]
                );
            }
        }

        /// Answers like a small server: one pull request found by two searches,
        /// one issue, and a tracked repository with a second pull request whose
        /// details cannot be read.
        const SERVER: &str = r#"
endpoint=""
for arg in "$@"; do endpoint=$arg; done
case "$endpoint" in
  --version) printf 'Version: \033[1m0.15.1\033[0m\tgolang: 1.26.5\n' ;;
  json) echo '[{"name":"other","url":"https://git.other","default":"true"},{"name":"main","url":"http://git.corp:3000/","default":"false"}]' ;;
  /user) echo '{"login":"me"}' ;;
  *"type=pulls&review_requested=true"|*"type=pulls&created=true") cat "$dir/search" ;;
  *"type=issues&assigned=true") echo '[{"number":7,"title":"bug","repository":{"full_name":"acme/app"}},{"number":9}]' ;;
  "/repos/acme/app/pulls?state=open&limit=50&page=1") echo '[{"number":8,"title":"tracked","user":{"username":"bruno"}},{"number":6,"title":"dup"}]' ;;
  /repos/acme/app/pulls/6) echo '{"mergeable":false,"additions":3,"head":{"ref":"feat","sha":"abc","repo":{"full_name":"ana/app"}},"base":{"ref":"main","repo":{"full_name":"acme/app"}}}' ;;
  /repos/acme/app/commits/abc/status) cat "$dir/status" ;;
  /repos/acme/app/pulls/6/reviews) echo '[{"id":1,"state":"APPROVED","body":"lgtm","comments_count":1,"submitted_at":"2026-09-20T10:00:00Z","user":{"login":"Me"}},{"id":2,"state":"PENDING","body":"rascunho","comments_count":1,"user":{"login":"me"}},{"id":3,"state":"COMMENT","user":{"login":"ana"}}]' ;;
  /repos/acme/app/pulls/6/reviews/1/comments) echo '[{"body":"aqui","path":"main.go","position":0,"original_position":12,"created_at":"2026-09-20T09:00:00Z","user":{"login":"Me"}}]' ;;
  /repos/acme/app/pulls/8) echo '{"message":"not found"}' ;;
  /repos/acme/app/issues/6) echo '{"number":6,"body":"descrição"}' ;;
  /repos/acme/app/issues/6/comments) echo '[{"body":"depois","created_at":"2026-09-21T09:00:00Z","user":{"login":"ana"}}]' ;;
  *) echo '[]' ;;
esac
"#;

        fn server() -> FakeTea {
            let tea = FakeTea::new(SERVER);
            tea.file("search", &format!("[{SEARCH_PAYLOAD}]"));
            tea.file("status", STATUS_PAYLOAD);
            tea
        }

        #[test]
        fn list_resolves_the_login_once_and_enriches_pull_requests() {
            let tea = server();
            let client = Gitea::new(instance("main", "git.corp:3000"), vec!["acme/app".into()]);
            let items = client.list(&tea.ctx).unwrap();
            let summary: Vec<_> = items
                .iter()
                .map(|item| (item.kind, item.number, item.relations))
                .collect();
            assert_eq!(
                summary,
                [
                    (Kind::Pr, 6, Relation::REVIEW_REQUESTED | Relation::AUTHORED),
                    (Kind::Issue, 7, Relation::ASSIGNED),
                    (Kind::Pr, 8, Relation::NONE),
                ]
            );
            let pr = &items[0];
            assert_eq!(pr.repo_url, "http://git.corp:3000/acme/app");
            assert_eq!(
                (pr.source_branch.as_str(), pr.target_branch.as_str()),
                ("feat", "main")
            );
            assert!(pr.from_fork && pr.conflicts && !pr.draft);
            assert_eq!((pr.ci, pr.checks.len()), (CiState::Pending, 2));
            assert_eq!(
                pr.checks[0].url,
                "http://git.corp:3000/acme/app/actions/runs/1/jobs/1"
            );
            assert_eq!(pr.review, REVIEW_APPROVED);
            assert!(pr.approved_by_me);
            // Enrichment failures leave the row as the list returned it.
            let tracked = &items[2];
            assert_eq!(
                (tracked.author.as_str(), tracked.review.as_str()),
                ("bruno", REVIEW_REQUIRED)
            );
            assert_eq!(tracked.ci, CiState::None);

            let calls = tea.calls();
            assert_eq!(
                calls[..3],
                [
                    strings(&["--version"]),
                    strings(&["logins", "list", "--output", "json"]),
                    strings(&["api", "--login", "main", "/user"]),
                ]
            );
            let count = |args: &[&str]| calls.iter().filter(|call| **call == strings(args)).count();
            assert_eq!(count(&["--version"]), 1);
            assert_eq!(count(&["logins", "list", "--output", "json"]), 1);
            for search in &SEARCHES {
                let endpoint = format!("/repos/issues/search?state=open&limit=50&{}", search.query);
                assert_eq!(
                    count(&["api", "--login", "main", &endpoint]),
                    1,
                    "{endpoint}"
                );
            }
            assert_eq!(
                count(&[
                    "api",
                    "--login",
                    "main",
                    "/repos/acme/app/pulls?state=open&limit=50&page=1"
                ]),
                1
            );
            // Six searches, one tracked page, three requests for #6, one for #8.
            assert_eq!(calls.len(), 3 + 6 + 1 + 3 + 1);
        }

        #[test]
        fn thread_merges_comments_reviews_and_inline_comments() {
            let tea = server();
            let client = client_with(instance("main", "git.corp"), "main", "");
            let item = Item {
                repo: "acme/app".into(),
                number: 6,
                ..Item::default()
            };
            let thread = client.thread(&tea.ctx, &item).unwrap();
            assert_eq!(thread.body, "descrição");
            let summary: Vec<_> = thread
                .comments
                .iter()
                .map(|c| {
                    (
                        c.author.as_str(),
                        c.body.as_str(),
                        c.review.as_str(),
                        c.path.as_str(),
                        c.line,
                    )
                })
                .collect();
            assert_eq!(
                summary,
                [
                    ("Me", "aqui", "", "main.go", 12),
                    ("Me", "lgtm", REVIEW_APPROVED, "", 0),
                    ("ana", "depois", "", "", 0),
                ]
            );
            let issue = Item {
                kind: Kind::Issue,
                ..item
            };
            assert_eq!(client.thread(&tea.ctx, &issue).unwrap().comments.len(), 1);
        }

        #[test]
        fn rejected_token_gets_the_login_hint() {
            let tea = FakeTea::new(r#"echo '{"message":"token is required"}'"#);
            let client = client_with(instance("corp", "git.corp"), "corp", "http://git.corp");
            let err = client.auth_status(&tea.ctx).unwrap_err();
            assert_eq!(
                err.text("en"),
                "tea not authenticated on git.corp (token is required). Run: tea login add --url http://git.corp"
            );
            let err = client
                .add_comment(&tea.ctx, &Item::default(), "oi")
                .unwrap_err();
            assert_eq!(
                err.text("en"),
                "tea api /repos//issues/0/comments: token is required"
            );
        }

        #[test]
        fn old_or_foreign_tea_is_rejected() {
            let client = || Gitea::new(instance("corp", "git.corp"), Vec::new());
            let old = FakeTea::new("echo 'Version: 0.13.9'");
            let err = client().auth_status(&old.ctx).unwrap_err();
            assert!(
                err.text("en")
                    .starts_with("tea 0.13 is too old; Gitea integration requires 0.14 or newer."),
                "{err}"
            );
            let foreign = FakeTea::new("echo 'tea 1.0.0'");
            let err = client().auth_status(&foreign.ctx).unwrap_err();
            assert_eq!(
                err.text("en"),
                "the tea found in PATH does not appear to be the Gitea CLI: tea 1.0.0"
            );
        }

        #[test]
        fn actions_use_the_documented_commands() {
            let tea = FakeTea::new("echo '{}'");
            let client = client_with(instance("corp", "git.corp"), "corp", "");
            let item = Item {
                repo: "acme/app".into(),
                number: 4,
                ..Item::default()
            };
            let options = MergeOptions {
                method: "squash".into(),
                auto: true,
                delete_branch: false,
            };
            client.add_comment(&tea.ctx, &item, "bom dia").unwrap();
            client.approve(&tea.ctx, &item).unwrap();
            client.merge(&tea.ctx, &item, &options).unwrap();
            client.close(&tea.ctx, &item).unwrap();
            client.checkout(&tea.ctx, &item, tea.dir.path()).unwrap();
            assert_eq!(
                tea.calls(),
                [
                    strings(&[
                        "api",
                        "--login",
                        "corp",
                        "--method",
                        "POST",
                        "-f",
                        "body=bom dia",
                        "/repos/acme/app/issues/4/comments"
                    ]),
                    strings(&[
                        "pulls", "approve", "4", "--repo", "acme/app", "--login", "corp"
                    ]),
                    strings(&[
                        "api",
                        "--login",
                        "corp",
                        "--method",
                        "POST",
                        "-f",
                        "Do=squash",
                        "-f",
                        "do=squash",
                        "-F",
                        "delete_branch_after_merge=false",
                        "-F",
                        "merge_when_checks_succeed=true",
                        "/repos/acme/app/pulls/4/merge"
                    ]),
                    strings(&[
                        "pulls", "close", "4", "--repo", "acme/app", "--login", "corp"
                    ]),
                    strings(&["pulls", "checkout", "4", "--branch", "--login", "corp"]),
                ]
            );
        }
    }
}
