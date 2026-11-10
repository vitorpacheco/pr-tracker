use std::path::Path;

use chrono::{DateTime, Utc};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};

use super::{
    Accumulator, Check, CiState, Client, Comment, Item, Kind, MergeOptions, REVIEW_APPROVED,
    REVIEW_CHANGES_REQUESTED, REVIEW_REQUIRED, Relation, Thread, merge_method, run, sort_comments,
};
use crate::config::{Instance, Provider};
use crate::error::{Error, Result};
use crate::msg;
use crate::toolchain::Ctx;

pub(super) struct GitHub {
    instance: Instance,
    tracked: Vec<String>,
}

// Both list queries must select the same pull request fields, so the fragment
// is written once.
macro_rules! pr_fragment {
    () => {
        r"fragment pr on PullRequest {
  number title url isDraft createdAt updatedAt
  comments { totalCount }
  labels(first: 10) { nodes { name } }
  assignees(first: 10) { nodes { login } }
  headRefName baseRefName isCrossRepository
  reviewDecision mergeable mergeStateStatus viewerCanUpdateBranch additions deletions changedFiles
  author { login }
  repository { nameWithOwner url }
  latestReviews(first: 30) { nodes { state author { login } } }
  commits(last: 1) { nodes { commit { statusCheckRollup {
    state
    contexts(first: 50) { nodes {
      __typename
      ... on CheckRun { name status conclusion detailsUrl }
      ... on StatusContext { context state targetUrl }
    } }
  } } } }
}"
    };
}

const GH_QUERY: &str = concat!(
    r"
query($review: String!, $authored: String!, $assigned: String!,
      $issAssigned: String!, $issAuthored: String!, $issMentioned: String!) {
  viewer { login }
  review: search(query: $review, type: ISSUE, first: 50) { nodes { ...pr } }
  authored: search(query: $authored, type: ISSUE, first: 50) { nodes { ...pr } }
  assigned: search(query: $assigned, type: ISSUE, first: 50) { nodes { ...pr } }
  issAssigned: search(query: $issAssigned, type: ISSUE, first: 50) { nodes { ...issue } }
  issAuthored: search(query: $issAuthored, type: ISSUE, first: 50) { nodes { ...issue } }
  issMentioned: search(query: $issMentioned, type: ISSUE, first: 50) { nodes { ...issue } }
}
fragment issue on Issue {
  number title url createdAt updatedAt
  author { login }
  repository { nameWithOwner url }
  comments { totalCount }
  labels(first: 10) { nodes { name } }
  assignees(first: 10) { nodes { login } }
}
",
    pr_fragment!()
);

const GH_TRACKED_QUERY: &str = concat!(
    r"
query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 50, after: $cursor) {
      nodes { ...pr }
      pageInfo { hasNextPage endCursor }
    }
  }
}
",
    pr_fragment!()
);

const GH_THREAD_QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issueOrPullRequest(number: $number) {
      ... on Issue {
        body
        comments(last: 100) { nodes { ...comment } }
      }
      ... on PullRequest {
        body
        comments(last: 100) { nodes { ...comment } }
        reviews(last: 50) { nodes {
          state body createdAt author { login }
          comments(first: 50) { nodes { ...comment path line originalLine } }
        } }
      }
    }
  }
}
fragment comment on Comment { body createdAt author { login } }";

/// GraphQL answers `null` for anything the viewer cannot read; such a value
/// decodes to the field's default instead of failing the whole response.
fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A connection's nodes, where both the list and its entries may be `null`
/// (for example pull requests hidden behind an organization's SSO).
fn nodes<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let nodes = Option::<Vec<Option<T>>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(nodes.into_iter().map(Option::unwrap_or_default).collect())
}

fn timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|text| {
            DateTime::parse_from_rfc3339(&text)
                .map(|time| time.with_timezone(&Utc))
                .map_err(D::Error::custom)
        })
        .transpose()
}

/// Decodes a whole response, treating a top-level `null` as an empty one.
fn decode<T: DeserializeOwned + Default>(output: &[u8]) -> serde_json::Result<T> {
    Ok(serde_json::from_slice::<Option<T>>(output)?.unwrap_or_default())
}

#[derive(Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de> + Default"))]
struct Nodes<T> {
    #[serde(deserialize_with = "nodes")]
    nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhLogin {
    #[serde(deserialize_with = "nullable")]
    login: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhLabel {
    #[serde(deserialize_with = "nullable")]
    name: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhCount {
    #[serde(deserialize_with = "nullable")]
    total_count: i64,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhRepository {
    #[serde(deserialize_with = "nullable")]
    name_with_owner: String,
    #[serde(deserialize_with = "nullable")]
    url: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhReview {
    #[serde(deserialize_with = "nullable")]
    state: String,
    author: Option<GhLogin>,
}

/// A CheckRun or a StatusContext, told apart by `typename`.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhContext {
    #[serde(rename = "__typename", deserialize_with = "nullable")]
    typename: String,
    #[serde(deserialize_with = "nullable")]
    name: String,
    #[serde(deserialize_with = "nullable")]
    status: String,
    #[serde(deserialize_with = "nullable")]
    conclusion: String,
    #[serde(deserialize_with = "nullable")]
    details_url: String,
    #[serde(deserialize_with = "nullable")]
    context: String,
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    target_url: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhRollup {
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    contexts: Nodes<GhContext>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhCommit {
    status_check_rollup: Option<GhRollup>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhCommitNode {
    #[serde(deserialize_with = "nullable")]
    commit: GhCommit,
}

/// A pull request or, with the PR-only fields left at their defaults, an issue.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhPr {
    #[serde(deserialize_with = "nullable")]
    number: i64,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    url: String,
    #[serde(deserialize_with = "nullable")]
    is_draft: bool,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    head_ref_name: String,
    #[serde(deserialize_with = "nullable")]
    base_ref_name: String,
    #[serde(deserialize_with = "nullable")]
    is_cross_repository: bool,
    #[serde(deserialize_with = "nullable")]
    review_decision: String,
    #[serde(deserialize_with = "nullable")]
    mergeable: String,
    #[serde(deserialize_with = "nullable")]
    merge_state_status: String,
    #[serde(deserialize_with = "nullable")]
    viewer_can_update_branch: bool,
    #[serde(deserialize_with = "nullable")]
    additions: i64,
    #[serde(deserialize_with = "nullable")]
    deletions: i64,
    #[serde(deserialize_with = "nullable")]
    changed_files: i64,
    #[serde(deserialize_with = "nullable")]
    comments: GhCount,
    #[serde(deserialize_with = "nullable")]
    labels: Nodes<GhLabel>,
    #[serde(deserialize_with = "nullable")]
    assignees: Nodes<GhLogin>,
    author: Option<GhLogin>,
    #[serde(deserialize_with = "nullable")]
    repository: GhRepository,
    #[serde(deserialize_with = "nullable")]
    latest_reviews: Nodes<GhReview>,
    #[serde(deserialize_with = "nullable")]
    commits: Nodes<GhCommitNode>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhPageInfo {
    #[serde(deserialize_with = "nullable")]
    has_next_page: bool,
    #[serde(deserialize_with = "nullable")]
    end_cursor: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhPullRequests {
    #[serde(deserialize_with = "nodes")]
    nodes: Vec<GhPr>,
    #[serde(deserialize_with = "nullable")]
    page_info: GhPageInfo,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhError {
    #[serde(deserialize_with = "nullable")]
    message: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhData {
    #[serde(deserialize_with = "nullable")]
    viewer: GhLogin,
    #[serde(deserialize_with = "nullable")]
    review: Nodes<GhPr>,
    #[serde(deserialize_with = "nullable")]
    authored: Nodes<GhPr>,
    #[serde(deserialize_with = "nullable")]
    assigned: Nodes<GhPr>,
    #[serde(deserialize_with = "nullable")]
    iss_assigned: Nodes<GhPr>,
    #[serde(deserialize_with = "nullable")]
    iss_authored: Nodes<GhPr>,
    #[serde(deserialize_with = "nullable")]
    iss_mentioned: Nodes<GhPr>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhResponse {
    #[serde(deserialize_with = "nullable")]
    data: GhData,
    #[serde(deserialize_with = "nodes")]
    errors: Vec<GhError>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhTrackedRepository {
    #[serde(deserialize_with = "nullable")]
    pull_requests: GhPullRequests,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhTrackedData {
    repository: Option<GhTrackedRepository>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhTrackedResponse {
    #[serde(deserialize_with = "nullable")]
    data: GhTrackedData,
    #[serde(deserialize_with = "nodes")]
    errors: Vec<GhError>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhComment {
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    author: Option<GhLogin>,
    #[serde(deserialize_with = "nullable")]
    path: String,
    #[serde(deserialize_with = "nullable")]
    line: i64,
    #[serde(deserialize_with = "nullable")]
    original_line: i64,
}

impl GhComment {
    fn convert(self) -> Comment {
        Comment {
            author: self.author.map(|author| author.login).unwrap_or_default(),
            body: self.body,
            created_at: self.created_at,
            review: String::new(),
            path: self.path,
            line: if self.line == 0 {
                self.original_line
            } else {
                self.line
            },
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhThreadReview {
    #[serde(flatten)]
    comment: GhComment,
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    comments: Nodes<GhComment>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhThreadItem {
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "nullable")]
    comments: Nodes<GhComment>,
    #[serde(deserialize_with = "nullable")]
    reviews: Nodes<GhThreadReview>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhThreadRepository {
    #[serde(rename = "issueOrPullRequest")]
    item: Option<GhThreadItem>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhThreadData {
    #[serde(deserialize_with = "nullable")]
    repository: GhThreadRepository,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct GhThreadResponse {
    #[serde(deserialize_with = "nullable")]
    data: GhThreadData,
    #[serde(deserialize_with = "nodes")]
    errors: Vec<GhError>,
}

fn gh_state(state: &str) -> CiState {
    match state {
        "SUCCESS" | "NEUTRAL" => CiState::Success,
        "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
            CiState::Failure
        }
        "CANCELLED" | "SKIPPED" | "STALE" => CiState::Canceled,
        "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "WAITING" | "REQUESTED" => {
            CiState::Pending
        }
        _ => CiState::None,
    }
}

impl GitHub {
    pub(super) fn new(instance: Instance, tracked: Vec<String>) -> Self {
        Self { instance, tracked }
    }

    fn gh(&self, ctx: &Ctx, dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>> {
        run(
            ctx,
            dir,
            &[("GH_HOST", self.instance.host.as_str())],
            "gh",
            args,
        )
    }

    /// The -R value accepted by gh: HOST/OWNER/REPO.
    fn repo_arg(&self, pr: &Item) -> String {
        format!("{}/{}", self.instance.host, pr.repo)
    }

    fn convert(&self, node: GhPr, me: &str) -> Item {
        let mut pr = Item {
            instance: self.instance.name.clone(),
            provider: Provider::GitHub,
            host: self.instance.host.clone(),
            repo: node.repository.name_with_owner,
            repo_url: node.repository.url,
            number: node.number,
            title: node.title,
            url: node.url,
            author: node.author.map(|author| author.login).unwrap_or_default(),
            draft: node.is_draft,
            source_branch: node.head_ref_name,
            target_branch: node.base_ref_name,
            from_fork: node.is_cross_repository,
            behind_base: node.merge_state_status == "BEHIND" || node.viewer_can_update_branch,
            can_update_branch: node.viewer_can_update_branch,
            created_at: node.created_at,
            updated_at: node.updated_at,
            additions: node.additions,
            deletions: node.deletions,
            files: node.changed_files,
            conflicts: node.mergeable == "CONFLICTING",
            comments: node.comments.total_count,
            labels: node
                .labels
                .nodes
                .into_iter()
                .map(|label| label.name)
                .collect(),
            assignees: node
                .assignees
                .nodes
                .into_iter()
                .map(|assignee| assignee.login)
                .collect(),
            ..Item::default()
        };
        pr.review = match node.review_decision.as_str() {
            "APPROVED" => REVIEW_APPROVED,
            "CHANGES_REQUESTED" => REVIEW_CHANGES_REQUESTED,
            "REVIEW_REQUIRED" => REVIEW_REQUIRED,
            _ => "",
        }
        .to_string();
        for review in node.latest_reviews.nodes {
            let Some(author) = review.author.filter(|_| review.state == "APPROVED") else {
                continue;
            };
            if author.login.to_lowercase() == me.to_lowercase() {
                pr.approved_by_me = true;
            }
            pr.approved_by.push(author.login);
        }
        if let Some(roll) = node
            .commits
            .nodes
            .into_iter()
            .next()
            .and_then(|node| node.commit.status_check_rollup)
        {
            pr.ci = gh_state(&roll.state);
            for context in roll.contexts.nodes {
                pr.checks.push(if context.typename == "CheckRun" {
                    let state = if context.status == "COMPLETED" {
                        gh_state(&context.conclusion)
                    } else {
                        CiState::Pending
                    };
                    Check {
                        name: context.name,
                        state,
                        url: context.details_url,
                    }
                } else {
                    Check {
                        name: context.context,
                        state: gh_state(&context.state),
                        url: context.target_url,
                    }
                });
            }
        }
        pr
    }
}

impl Client for GitHub {
    fn instance(&self) -> &Instance {
        &self.instance
    }

    fn tool(&self) -> &'static str {
        "gh"
    }

    fn list(&self, ctx: &Ctx) -> Result<Vec<Item>> {
        const BASE: &str = "is:pr is:open archived:false sort:updated-desc ";
        const ISSUES: &str = "is:issue is:open archived:false sort:updated-desc ";
        let host = self.instance.host.as_str();
        let out = self.gh(
            ctx,
            None,
            &[
                "api",
                "graphql",
                "--hostname",
                host,
                "-f",
                &format!("query={GH_QUERY}"),
                "-f",
                &format!("review={BASE}review-requested:@me"),
                "-f",
                &format!("authored={BASE}author:@me"),
                "-f",
                &format!("assigned={BASE}assignee:@me"),
                "-f",
                &format!("issAssigned={ISSUES}assignee:@me"),
                "-f",
                &format!("issAuthored={ISSUES}author:@me"),
                "-f",
                &format!("issMentioned={ISSUES}mentions:@me"),
            ],
        )?;
        let response: GhResponse =
            decode(&out).map_err(|err| msg!("resposta inválida do gh: %w", Error::from(err)))?;
        let data = response.data;
        if let Some(error) = response.errors.first()
            && data.viewer.login.is_empty()
        {
            return Err(Error::external(format!("gh graphql: {}", error.message)));
        }
        let me = data.viewer.login;
        let mut acc = Accumulator::default();
        for (kind, relation, search) in [
            (Kind::Pr, Relation::REVIEW_REQUESTED, data.review),
            (Kind::Pr, Relation::AUTHORED, data.authored),
            (Kind::Pr, Relation::ASSIGNED, data.assigned),
            (Kind::Issue, Relation::ASSIGNED, data.iss_assigned),
            (Kind::Issue, Relation::AUTHORED, data.iss_authored),
            (Kind::Issue, Relation::MENTIONED, data.iss_mentioned),
        ] {
            for node in search.nodes {
                // search can return other node types as empty objects
                if node.number == 0 {
                    continue;
                }
                let mut item = self.convert(node, &me);
                item.kind = kind;
                acc.add(item, relation);
            }
        }
        let query = format!("query={GH_TRACKED_QUERY}");
        for repo in &self.tracked {
            let (owner, name) = match repo.split_once('/') {
                Some((owner, name))
                    if !owner.is_empty() && !name.is_empty() && !name.contains('/') =>
                {
                    (owner, name)
                }
                _ => {
                    return Err(msg!(
                        "repositório GitHub inválido %q (esperado owner/repo)",
                        repo
                    ));
                }
            };
            let (owner_arg, name_arg) = (format!("owner={owner}"), format!("name={name}"));
            let mut cursor = String::new();
            loop {
                let cursor_arg = format!("cursor={cursor}");
                let mut args = vec![
                    "api",
                    "graphql",
                    "--hostname",
                    host,
                    "-f",
                    &query,
                    "-f",
                    &owner_arg,
                    "-f",
                    &name_arg,
                ];
                if !cursor.is_empty() {
                    args.extend(["-f", cursor_arg.as_str()]);
                }
                let out = self.gh(ctx, None, &args)?;
                let tracked: GhTrackedResponse = decode(&out).map_err(|err| {
                    msg!(
                        "resposta inválida do gh para %s: %w",
                        repo,
                        Error::from(err)
                    )
                })?;
                if let Some(error) = tracked.errors.first() {
                    return Err(Error::external(format!(
                        "gh graphql ({repo}): {}",
                        error.message
                    )));
                }
                let Some(repository) = tracked.data.repository else {
                    return Err(msg!("repositório %s não encontrado em %s", repo, host));
                };
                let prs = repository.pull_requests;
                for node in prs.nodes {
                    if node.number != 0 {
                        acc.add(self.convert(node, &me), Relation::NONE);
                    }
                }
                let page = prs.page_info;
                if !page.has_next_page || page.end_cursor.is_empty() || page.end_cursor == cursor {
                    break;
                }
                cursor = page.end_cursor;
            }
        }
        Ok(acc.list())
    }

    fn thread(&self, ctx: &Ctx, item: &Item) -> Result<Thread> {
        let (owner, name) = item
            .repo
            .split_once('/')
            .unwrap_or((item.repo.as_str(), ""));
        let out = self.gh(
            ctx,
            None,
            &[
                "api",
                "graphql",
                "--hostname",
                &self.instance.host,
                "-f",
                &format!("query={GH_THREAD_QUERY}"),
                "-f",
                &format!("owner={owner}"),
                "-f",
                &format!("name={name}"),
                "-F",
                &format!("number={}", item.number),
            ],
        )?;
        let response: GhThreadResponse =
            decode(&out).map_err(|err| msg!("resposta inválida do gh: %w", Error::from(err)))?;
        let Some(source) = response.data.repository.item else {
            if let Some(error) = response.errors.first() {
                return Err(Error::external(format!("gh graphql: {}", error.message)));
            }
            return Err(msg!("%s%s não encontrado", &item.repo, item.reference()));
        };
        let mut thread = Thread {
            body: source.body,
            comments: Vec::new(),
        };
        thread
            .comments
            .extend(source.comments.nodes.into_iter().map(GhComment::convert));
        for review in source.reviews.nodes {
            // Reviews without a body and with no inline comments are just
            // "commented" markers left by inline replies; skip them.
            if !review.comment.body.is_empty()
                || review.state == "APPROVED"
                || review.state == "CHANGES_REQUESTED"
            {
                let mut comment = review.comment.convert();
                comment.review = review.state.to_lowercase();
                thread.comments.push(comment);
            }
            thread
                .comments
                .extend(review.comments.nodes.into_iter().map(GhComment::convert));
        }
        sort_comments(&mut thread.comments);
        Ok(thread)
    }

    fn add_comment(&self, ctx: &Ctx, item: &Item, body: &str) -> Result<()> {
        let sub = if item.is_issue() { "issue" } else { "pr" };
        self.gh(
            ctx,
            None,
            &[
                sub,
                "comment",
                &item.number.to_string(),
                "-R",
                &self.repo_arg(item),
                "--body",
                body,
            ],
        )?;
        Ok(())
    }

    fn approve(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.gh(
            ctx,
            None,
            &[
                "pr",
                "review",
                &pr.number.to_string(),
                "--approve",
                "-R",
                &self.repo_arg(pr),
            ],
        )?;
        Ok(())
    }

    fn merge(&self, ctx: &Ctx, pr: &Item, options: &MergeOptions) -> Result<()> {
        let (number, repo, method) = (
            pr.number.to_string(),
            self.repo_arg(pr),
            format!("--{}", merge_method(&options.method)),
        );
        let mut args = vec!["pr", "merge", &number, "-R", &repo, &method];
        if options.auto {
            args.push("--auto");
        }
        if options.delete_branch {
            args.push("--delete-branch");
        }
        self.gh(ctx, None, &args)?;
        Ok(())
    }

    /// Leaves the branch alone: gh's --delete-branch would also delete the
    /// local branch of whatever clone the command runs in.
    fn close(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.gh(
            ctx,
            None,
            &[
                "pr",
                "close",
                &pr.number.to_string(),
                "-R",
                &self.repo_arg(pr),
            ],
        )?;
        Ok(())
    }

    fn set_draft(&self, ctx: &Ctx, pr: &Item, draft: bool) -> Result<()> {
        let (number, repo) = (pr.number.to_string(), self.repo_arg(pr));
        let mut args = vec!["pr", "ready", &number, "-R", &repo];
        if draft {
            args.push("--undo");
        }
        self.gh(ctx, None, &args)?;
        Ok(())
    }

    fn update_branch(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.gh(
            ctx,
            None,
            &[
                "pr",
                "update-branch",
                &pr.number.to_string(),
                "-R",
                &self.repo_arg(pr),
            ],
        )?;
        Ok(())
    }

    fn checkout(&self, ctx: &Ctx, pr: &Item, dir: &Path) -> Result<()> {
        // An empty directory means the current one, which a child inherits.
        let dir = Some(dir).filter(|dir| !dir.as_os_str().is_empty());
        self.gh(
            ctx,
            dir,
            &[
                "pr",
                "checkout",
                &pr.number.to_string(),
                "-R",
                &self.repo_arg(pr),
            ],
        )?;
        Ok(())
    }

    fn head_ref(&self, pr: &Item) -> String {
        format!("refs/pull/{}/head", pr.number)
    }

    fn auth_status(&self, ctx: &Ctx) -> Result<()> {
        let host = &self.instance.host;
        match run(
            ctx,
            None,
            &[],
            "gh",
            &["auth", "status", "--hostname", host],
        ) {
            Ok(_) => Ok(()),
            Err(err) if err.is_missing_tool() => Err(err),
            Err(_) => Err(msg!(
                "gh não autenticado em %s. Rode: gh auth login --hostname %s",
                host,
                host
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(name: &str, host: &str) -> GitHub {
        GitHub::new(
            Instance {
                name: name.into(),
                provider: Provider::GitHub,
                host: host.into(),
                ..Instance::default()
            },
            Vec::new(),
        )
    }

    #[test]
    fn convert_maps_review_conflicts_and_approvals() {
        let github = client("ghe", "ghe.corp");
        let node = GhPr {
            number: 7,
            repository: GhRepository {
                name_with_owner: "o/r".into(),
                ..GhRepository::default()
            },
            review_decision: "CHANGES_REQUESTED".into(),
            mergeable: "CONFLICTING".into(),
            latest_reviews: Nodes {
                nodes: vec![GhReview {
                    state: "APPROVED".into(),
                    author: Some(GhLogin { login: "Me".into() }),
                }],
            },
            ..GhPr::default()
        };
        let pr = github.convert(node, "me");
        assert_eq!(pr.review, REVIEW_CHANGES_REQUESTED);
        assert!(pr.conflicts && pr.approved_by_me, "{pr:?}");
        assert_eq!(pr.approved_by, ["Me"]);
        assert_eq!(
            (pr.host.as_str(), pr.reference().as_str()),
            ("ghe.corp", "#7")
        );
        assert_eq!(github.repo_arg(&pr), "ghe.corp/o/r");
        assert_eq!(github.head_ref(&pr), "refs/pull/7/head");
    }

    #[test]
    fn states() {
        for (state, want) in [
            ("SUCCESS", CiState::Success),
            ("FAILURE", CiState::Failure),
            ("PENDING", CiState::Pending),
            ("SKIPPED", CiState::Canceled),
            ("", CiState::None),
        ] {
            assert_eq!(gh_state(state), want, "gh_state({state:?})");
        }
    }

    #[test]
    fn branch_update_state() {
        let github = client("", "");
        for (status, can, behind) in [
            ("BEHIND", false, true),
            ("CLEAN", true, true),
            ("CLEAN", false, false),
            ("UNKNOWN", false, false),
        ] {
            let node = GhPr {
                merge_state_status: status.into(),
                viewer_can_update_branch: can,
                ..GhPr::default()
            };
            let item = github.convert(node, "me");
            assert_eq!(
                (item.behind_base, item.can_update_branch),
                (behind, can),
                "{status} {can}"
            );
        }
    }

    #[test]
    fn null_values_decode_to_defaults() {
        let response: GhResponse = decode(
            br#"{"data":{"viewer":{"login":"me"},"review":{"nodes":[null,{"number":3,"title":null,"createdAt":null,
            "isDraft":null,"additions":null,"author":null,"labels":{"nodes":null},"assignees":null,"repository":null,
            "latestReviews":{"nodes":[{"state":"APPROVED","author":null}]},
            "commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}}]},"authored":null},"errors":null}"#,
        )
        .unwrap();
        let nodes = response.data.review.nodes;
        assert_eq!(
            nodes.iter().map(|node| node.number).collect::<Vec<_>>(),
            [0, 3]
        );
        let item = client("gh", "github.com").convert(
            nodes.into_iter().nth(1).unwrap(),
            &response.data.viewer.login,
        );
        assert_eq!(
            item,
            Item {
                instance: "gh".into(),
                host: "github.com".into(),
                number: 3,
                ..Item::default()
            }
        );
        assert!(
            decode::<GhResponse>(b"null")
                .unwrap()
                .data
                .viewer
                .login
                .is_empty()
        );
        assert!(
            decode::<GhResponse>(br#"{"data":{"review":{"nodes":[{"createdAt":"yesterday"}]}}}"#)
                .is_err()
        );
    }

    #[test]
    fn convert_reads_checks_and_timestamps() {
        let node: GhPr = decode(
            br#"{"number":4,"createdAt":"2024-05-01T10:00:00+02:00","updatedAt":"2024-05-02T08:00:00Z",
            "labels":{"nodes":[{"name":"bug"}]},"assignees":{"nodes":[{"login":"ana"}]},"author":{"login":"bob"},
            "reviewDecision":"REVIEW_REQUIRED","comments":{"totalCount":2},"changedFiles":5,
            "latestReviews":{"nodes":[{"state":"COMMENTED","author":{"login":"me"}},{"state":"APPROVED","author":{"login":"ana"}}]},
            "commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE","contexts":{"nodes":[
              {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":null,"detailsUrl":"u1"},
              {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"u2"},
              {"__typename":"StatusContext","context":"deploy","state":"ERROR","targetUrl":"u3"}]}}}}]}}"#,
        )
        .unwrap();
        let item = client("gh", "github.com").convert(node, "me");
        assert_eq!(
            item.created_at.unwrap().to_rfc3339(),
            "2024-05-01T08:00:00+00:00"
        );
        assert_eq!(
            item.updated_at.unwrap().to_rfc3339(),
            "2024-05-02T08:00:00+00:00"
        );
        assert_eq!(
            (item.labels, item.assignees, item.author.as_str()),
            (vec!["bug".to_string()], vec!["ana".to_string()], "bob")
        );
        assert_eq!(
            (item.review.as_str(), item.comments, item.files),
            (REVIEW_REQUIRED, 2, 5)
        );
        assert_eq!(
            (item.approved_by, item.approved_by_me),
            (vec!["ana".to_string()], false)
        );
        assert_eq!(item.ci, CiState::Failure);
        let check = |name: &str, state, url: &str| Check {
            name: name.into(),
            state,
            url: url.into(),
        };
        assert_eq!(
            item.checks,
            [
                check("build", CiState::Pending, "u1"),
                check("lint", CiState::Success, "u2"),
                check("deploy", CiState::Failure, "u3")
            ]
        );
    }

    #[cfg(unix)]
    mod cli {
        use std::collections::BTreeMap;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;
        use std::process::Command;
        use std::time::Duration;

        use super::super::*;
        use super::client;

        const SCRIPT: &str = r#"#!/bin/sh
[ "$1" = --probe ] && exit 0
dir=$(dirname "$0")
{
  printf 'host=%s\n' "$GH_HOST"
  for arg in "$@"; do printf 'arg=%s\n' "$arg"; done
  echo end
} >> "$dir/calls"
if [ -e "$dir/fail" ]; then
  echo 'permission denied' >&2
  exit 1
fi
n=$(grep -c '^end$' "$dir/calls")
if [ -e "$dir/response.$n" ]; then cat "$dir/response.$n"; else echo '{}'; fi
"#;

        #[derive(Debug, Default, PartialEq)]
        struct Call {
            host: String,
            args: Vec<String>,
        }

        /// A stand-in for gh that records its invocations. These tests never
        /// run an authenticated CLI or make network requests.
        struct FakeGh {
            dir: tempfile::TempDir,
        }

        impl FakeGh {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                let script = dir.path().join("gh");
                fs::write(&script, SCRIPT).unwrap();
                fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
                // A concurrent test forking while the script was open for
                // writing keeps it "text file busy" until that child execs.
                for attempt in 0.. {
                    match Command::new(&script).arg("--probe").status() {
                        Err(err)
                            if err.kind() == std::io::ErrorKind::ExecutableFileBusy
                                && attempt < 200 =>
                        {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        status => {
                            assert!(status.unwrap().success());
                            break;
                        }
                    }
                }
                Self { dir }
            }

            fn path(&self, name: &str) -> PathBuf {
                self.dir.path().join(name)
            }

            fn ctx(&self) -> Ctx {
                let script = self.path("gh").to_string_lossy().into_owned();
                Ctx::background().with_paths(&BTreeMap::from([("gh".to_string(), script)]))
            }

            fn fail(&self) {
                fs::write(self.path("fail"), "").unwrap();
            }

            /// What the `n`th invocation (1-based) prints.
            fn respond(&self, n: usize, body: &str) {
                fs::write(self.path(&format!("response.{n}")), body).unwrap();
            }

            fn calls(&self) -> Vec<Call> {
                let log = fs::read_to_string(self.path("calls")).unwrap();
                let mut calls = Vec::new();
                let mut call = Call::default();
                for line in log.lines() {
                    if let Some(host) = line.strip_prefix("host=") {
                        call.host = host.to_string();
                    } else if let Some(arg) = line.strip_prefix("arg=") {
                        call.args.push(arg.to_string());
                    } else if line == "end" {
                        calls.push(std::mem::take(&mut call));
                    } else if let Some(arg) = call.args.last_mut() {
                        // A continuation line of a multi-line argument.
                        arg.push('\n');
                        arg.push_str(line);
                    }
                }
                calls
            }
        }

        #[test]
        fn draft_commands() {
            for draft in [false, true] {
                let gh = FakeGh::new();
                let instance = Instance {
                    name: "work".into(),
                    provider: Provider::GitHub,
                    host: "git.company.com".into(),
                    ..Instance::default()
                };
                let client = crate::provider::new(&instance, &[]);
                assert_eq!(client.tool(), "gh");
                let item = Item {
                    repo: "team/sub/service".into(),
                    number: 42,
                    ..Item::default()
                };
                client.set_draft(&gh.ctx(), &item, draft).unwrap();
                let calls = gh.calls();
                assert_eq!(calls.len(), 1, "{calls:?}");
                let mut want = vec![
                    "pr",
                    "ready",
                    "42",
                    "-R",
                    "git.company.com/team/sub/service",
                ];
                if draft {
                    want.push("--undo");
                }
                assert_eq!(calls[0].host, "git.company.com", "wrong GitHub host");
                assert_eq!(calls[0].args, want);
                gh.fail();
                assert!(
                    client.set_draft(&gh.ctx(), &item, draft).is_err(),
                    "CLI failure ignored"
                );
            }
        }

        #[test]
        fn update_branch_commands() {
            for fail in [false, true] {
                let gh = FakeGh::new();
                if fail {
                    gh.fail();
                }
                let instance = Instance {
                    name: "work".into(),
                    provider: Provider::GitHub,
                    host: "git.company.com".into(),
                    ..Instance::default()
                };
                let client = crate::provider::new(&instance, &[]);
                let item = Item {
                    repo: "team/service".into(),
                    number: 42,
                    ..Item::default()
                };
                let result = client.update_branch(&gh.ctx(), &item);
                assert_eq!(result.is_err(), fail, "{result:?}");
                if let Err(err) = result {
                    assert_eq!(err.text("en"), "gh pr update-branch: permission denied");
                }
                let calls = gh.calls();
                assert_eq!(calls.len(), 1, "{calls:?}");
                assert_eq!(calls[0].host, instance.host, "wrong GitHub host");
                assert_eq!(
                    calls[0].args,
                    [
                        "pr",
                        "update-branch",
                        "42",
                        "-R",
                        "git.company.com/team/service"
                    ]
                );
            }
        }

        #[test]
        fn action_commands() {
            let gh = FakeGh::new();
            let github = client("work", "ghe.corp");
            let ctx = gh.ctx();
            let pr = Item {
                repo: "o/r".into(),
                number: 7,
                ..Item::default()
            };
            let issue = Item {
                kind: Kind::Issue,
                ..pr.clone()
            };
            github.add_comment(&ctx, &pr, "looks good\nthanks").unwrap();
            github.add_comment(&ctx, &issue, "x").unwrap();
            github.approve(&ctx, &pr).unwrap();
            github.merge(&ctx, &pr, &MergeOptions::default()).unwrap();
            github
                .merge(
                    &ctx,
                    &pr,
                    &MergeOptions {
                        method: "squash".into(),
                        auto: true,
                        delete_branch: true,
                    },
                )
                .unwrap();
            github.close(&ctx, &pr).unwrap();
            github.checkout(&ctx, &pr, gh.dir.path()).unwrap();
            github.checkout(&ctx, &pr, Path::new("")).unwrap();
            github.auth_status(&ctx).unwrap();
            let calls = gh.calls();
            let args: Vec<Vec<&str>> = calls
                .iter()
                .map(|call| call.args.iter().map(String::as_str).collect())
                .collect();
            let want: [&[&str]; 9] = [
                &[
                    "pr",
                    "comment",
                    "7",
                    "-R",
                    "ghe.corp/o/r",
                    "--body",
                    "looks good\nthanks",
                ],
                &["issue", "comment", "7", "-R", "ghe.corp/o/r", "--body", "x"],
                &["pr", "review", "7", "--approve", "-R", "ghe.corp/o/r"],
                &["pr", "merge", "7", "-R", "ghe.corp/o/r", "--merge"],
                &[
                    "pr",
                    "merge",
                    "7",
                    "-R",
                    "ghe.corp/o/r",
                    "--squash",
                    "--auto",
                    "--delete-branch",
                ],
                &["pr", "close", "7", "-R", "ghe.corp/o/r"],
                &["pr", "checkout", "7", "-R", "ghe.corp/o/r"],
                &["pr", "checkout", "7", "-R", "ghe.corp/o/r"],
                &["auth", "status", "--hostname", "ghe.corp"],
            ];
            assert_eq!(args, want);
            assert!(
                calls[..8].iter().all(|call| call.host == "ghe.corp"),
                "{calls:?}"
            );
        }

        #[test]
        fn auth_status_distinguishes_a_missing_cli_from_a_failed_login() {
            let gh = FakeGh::new();
            gh.fail();
            let github = client("work", "ghe.corp");
            let err = github.auth_status(&gh.ctx()).unwrap_err();
            assert_eq!(
                err.text("en"),
                "gh not authenticated on ghe.corp. Run: gh auth login --hostname ghe.corp"
            );
            let missing = BTreeMap::from([(
                "gh".to_string(),
                gh.path("missing-gh").to_string_lossy().into_owned(),
            )]);
            let err = github
                .auth_status(&Ctx::background().with_paths(&missing))
                .unwrap_err();
            assert!(err.is_missing_tool(), "{err:?}");
        }

        #[test]
        fn list_merges_searches_and_pages_tracked_repositories() {
            let gh = FakeGh::new();
            let pr = |number: i64| {
                format!(r#"{{"number":{number},"repository":{{"nameWithOwner":"o/r"}}}}"#)
            };
            // Errors next to a known viewer are partial failures (for example
            // SSO-protected results) and must not discard the rest.
            gh.respond(
                1,
                &format!(
                    r#"{{"data":{{"viewer":{{"login":"me"}},"review":{{"nodes":[{one},{{}},null]}},
                    "authored":{{"nodes":[{one}]}},"issAssigned":{{"nodes":[{five}]}}}},
                    "errors":[{{"message":"partial"}}]}}"#,
                    one = pr(1),
                    five = pr(5)
                ),
            );
            gh.respond(
                2,
                &format!(
                    r#"{{"data":{{"repository":{{"pullRequests":{{"nodes":[{},{}],"pageInfo":{{"hasNextPage":true,"endCursor":"C1"}}}}}}}}}}"#,
                    pr(1),
                    pr(9)
                ),
            );
            // A server that keeps returning the same cursor must not loop forever.
            gh.respond(
                3,
                &format!(
                    r#"{{"data":{{"repository":{{"pullRequests":{{"nodes":[{}],"pageInfo":{{"hasNextPage":true,"endCursor":"C1"}}}}}}}}}}"#,
                    pr(10)
                ),
            );
            let instance = Instance {
                name: "work".into(),
                provider: Provider::GitHub,
                host: "ghe.corp".into(),
                ..Instance::default()
            };
            let github = GitHub::new(instance, vec!["o/r".into()]);
            let items = github.list(&gh.ctx()).unwrap();
            let got: Vec<_> = items
                .iter()
                .map(|item| (item.number, item.kind, item.relations))
                .collect();
            assert_eq!(
                got,
                [
                    (1, Kind::Pr, Relation::REVIEW_REQUESTED | Relation::AUTHORED),
                    (5, Kind::Issue, Relation::ASSIGNED),
                    (9, Kind::Pr, Relation::NONE),
                    (10, Kind::Pr, Relation::NONE),
                ]
            );
            assert!(items.iter().all(|item| item.instance == "work"
                && item.host == "ghe.corp"
                && item.repo == "o/r"));

            let calls = gh.calls();
            assert_eq!(calls.len(), 3, "{calls:?}");
            assert!(calls.iter().all(|call| call.host == "ghe.corp"));
            let query = format!("query={GH_QUERY}");
            let want: [&str; 18] = [
                "api",
                "graphql",
                "--hostname",
                "ghe.corp",
                "-f",
                &query,
                "-f",
                "review=is:pr is:open archived:false sort:updated-desc review-requested:@me",
                "-f",
                "authored=is:pr is:open archived:false sort:updated-desc author:@me",
                "-f",
                "assigned=is:pr is:open archived:false sort:updated-desc assignee:@me",
                "-f",
                "issAssigned=is:issue is:open archived:false sort:updated-desc assignee:@me",
                "-f",
                "issAuthored=is:issue is:open archived:false sort:updated-desc author:@me",
                "-f",
                "issMentioned=is:issue is:open archived:false sort:updated-desc mentions:@me",
            ];
            assert_eq!(calls[0].args, want);
            let tracked = format!("query={GH_TRACKED_QUERY}");
            let page = [
                "api",
                "graphql",
                "--hostname",
                "ghe.corp",
                "-f",
                &tracked,
                "-f",
                "owner=o",
                "-f",
                "name=r",
            ];
            assert_eq!(calls[1].args, page);
            assert_eq!(calls[2].args, [&page[..], &["-f", "cursor=C1"]].concat());
        }

        #[test]
        fn list_reports_invalid_and_missing_repositories() {
            let list = |tracked: &str, responses: &[&str]| {
                let gh = FakeGh::new();
                for (index, body) in responses.iter().enumerate() {
                    gh.respond(index + 1, body);
                }
                let instance = Instance {
                    name: "work".into(),
                    provider: Provider::GitHub,
                    host: "ghe.corp".into(),
                    ..Instance::default()
                };
                let tracked = if tracked.is_empty() {
                    Vec::new()
                } else {
                    vec![tracked.to_string()]
                };
                GitHub::new(instance, tracked)
                    .list(&gh.ctx())
                    .unwrap_err()
                    .text("en")
            };
            let viewer = r#"{"data":{"viewer":{"login":"me"}}}"#;
            assert_eq!(
                list("", &[r#"{"errors":[{"message":"bad credentials"}]}"#]),
                "gh graphql: bad credentials"
            );
            assert!(list("", &["not json"]).starts_with("invalid gh response: "));
            for repo in ["service", "/service", "team/", "team/sub/service"] {
                assert_eq!(
                    list(repo, &[viewer]),
                    format!("invalid GitHub repository {repo:?} (expected owner/repo)")
                );
            }
            assert_eq!(
                list("o/r", &[viewer, r#"{"data":{"repository":null}}"#]),
                "repository o/r not found on ghe.corp"
            );
            assert_eq!(
                list(
                    "o/r",
                    &[
                        viewer,
                        r#"{"data":{"repository":null},"errors":[{"message":"no access"}]}"#
                    ]
                ),
                "gh graphql (o/r): no access"
            );
            assert!(list("o/r", &[viewer, "oops"]).starts_with("invalid gh response for o/r: "));
        }

        #[test]
        fn thread_orders_comments_and_skips_empty_review_markers() {
            let gh = FakeGh::new();
            gh.respond(
                1,
                r#"{"data":{"repository":{"issueOrPullRequest":{"body":"description",
                "comments":{"nodes":[{"body":"third","createdAt":"2024-01-03T00:00:00Z","author":{"login":"ana"}}]},
                "reviews":{"nodes":[
                  {"state":"COMMENTED","body":"","createdAt":"2024-01-01T00:00:00Z","author":{"login":"bob"},
                   "comments":{"nodes":[{"body":"inline","createdAt":"2024-01-02T00:00:00Z","author":null,
                                         "path":"main.go","line":null,"originalLine":12}]}},
                  {"state":"APPROVED","body":"","createdAt":"2024-01-04T00:00:00Z","author":{"login":"bob"},"comments":{"nodes":[]}},
                  {"state":"COMMENTED","body":"first","createdAt":"2023-12-31T00:00:00Z","author":{"login":"cy"},"comments":null}
                ]}}}}}"#,
            );
            let github = client("work", "ghe.corp");
            let item = Item {
                repo: "o/r".into(),
                number: 7,
                ..Item::default()
            };
            let thread = github.thread(&gh.ctx(), &item).unwrap();
            assert_eq!(thread.body, "description");
            let got: Vec<_> = thread
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
                got,
                [
                    ("cy", "first", "commented", "", 0),
                    ("", "inline", "", "main.go", 12),
                    ("ana", "third", "", "", 0),
                    ("bob", "", "approved", "", 0),
                ]
            );
            let calls = gh.calls();
            let query = format!("query={GH_THREAD_QUERY}");
            let want = [
                "api",
                "graphql",
                "--hostname",
                "ghe.corp",
                "-f",
                &query,
                "-f",
                "owner=o",
                "-f",
                "name=r",
                "-F",
                "number=7",
            ];
            assert_eq!(calls[0].args, want);
            assert_eq!(calls[0].host, "ghe.corp");
        }

        #[test]
        fn thread_reports_a_missing_item() {
            let github = client("work", "ghe.corp");
            let item = Item {
                repo: "o/r".into(),
                number: 7,
                ..Item::default()
            };
            let gh = FakeGh::new();
            assert_eq!(
                github.thread(&gh.ctx(), &item).unwrap_err().text("en"),
                "o/r#7 not found"
            );
            let gh = FakeGh::new();
            gh.respond(
                1,
                r#"{"data":{"repository":null},"errors":[{"message":"Could not resolve"}]}"#,
            );
            assert_eq!(
                github.thread(&gh.ctx(), &item).unwrap_err().text("en"),
                "gh graphql: Could not resolve"
            );
        }
    }
}
