use std::path::Path;

use chrono::{DateTime, Utc};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer};

use super::{
    Accumulator, Check, CiState, Client, Comment, Item, Kind, MergeOptions, REVIEW_APPROVED,
    REVIEW_REQUIRED, Relation, Thread, merge_method, run, sort_comments,
};
use crate::config::{Instance, Provider};
use crate::error::{Error, Result};
use crate::msg;
use crate::toolchain::Ctx;

pub(super) struct GitLab {
    instance: Instance,
    tracked: Vec<String>,
}

macro_rules! merge_request_fragment {
    () => {
        r#"
# Labels/assignees are left out: GitLab caps query complexity at 200-250.
fragment mr on MergeRequest {
  iid title webUrl draft createdAt updatedAt userNotesCount
  sourceBranch targetBranch conflicts approved
  diffStatsSummary { additions deletions fileCount }
  author { username }
  project { fullPath webUrl }
  sourceProject { fullPath }
  approvedBy { nodes { username } }
  headPipeline {
    status
    jobs(first: 50) { nodes { name status webPath } }
  }
}"#
    };
}

const QUERY: &str = concat!(
    r#"
query {
  currentUser {
    username
    review: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    authored: authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
  }
}"#,
    merge_request_fragment!()
);

const TRACKED_QUERY: &str = concat!(
    r#"
query($repo: ID!, $cursor: String) {
  project(fullPath: $repo) {
    mergeRequests(state: opened, first: 50, after: $cursor, sort: UPDATED_DESC) {
      nodes { ...mr }
      pageInfo { hasNextPage endCursor }
    }
  }
}"#,
    merge_request_fragment!()
);

const ISSUES_QUERY: &str = r#"
query($me: String!) {
  assigned: issues(assigneeUsernames: [$me], state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...issue } }
  authored: issues(authorUsername: $me, state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...issue } }
  currentUser {
    todos(action: [mentioned, directly_addressed], type: [ISSUE, WORKITEM], state: [pending], first: 50) {
      nodes {
        project { webUrl }
        target {
          ... on Issue { ...issue }
          ... on WorkItem { iid title webUrl createdAt updatedAt state reference(full: true) author { username } }
        }
      }
    }
  }
}
fragment issue on Issue {
  iid title webUrl createdAt updatedAt state reference(full: true) userNotesCount
  author { username }
  labels(first: 10) { nodes { title } }
  assignees(first: 10) { nodes { username } }
}"#;

/// `%s` is the project field to read: `mergeRequest` or `issue`.
const THREAD_QUERY: &str = r#"
query($path: ID!, $iid: String!) {
  project(fullPath: $path) {
    %s(iid: $iid) {
      description
      notes(filter: ONLY_COMMENTS, last: 100) { nodes {
        body createdAt system
        author { username }
        position { filePath newLine oldLine }
      } }
    }
  }
}"#;

/// GraphQL answers `null` for anything it could not resolve; that reads as the
/// field's empty value instead of failing the whole response.
fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A connection's nodes, where both the list and its entries may be `null`
/// (for example an approver the viewer may not read). Entries that resolved
/// are kept.
fn nodes<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let nodes = Option::<Vec<Option<T>>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(nodes.into_iter().flatten().collect())
}

fn timestamp<'de, D>(deserializer: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    DateTime::parse_from_rfc3339(&text)
        .map(|time| Some(time.with_timezone(&Utc)))
        .map_err(de::Error::custom)
}

/// Decodes a CLI response, reading a top-level `null` as an empty one.
fn decode<T: DeserializeOwned + Default>(out: &[u8]) -> serde_json::Result<T> {
    Ok(serde_json::from_slice::<Option<T>>(out)?.unwrap_or_default())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de> + Default"))]
struct Response<T> {
    #[serde(deserialize_with = "nullable")]
    data: T,
    #[serde(deserialize_with = "nullable")]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GraphQlError {
    #[serde(deserialize_with = "nullable")]
    message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de> + Default"))]
struct Nodes<T> {
    #[serde(deserialize_with = "nodes")]
    nodes: Vec<T>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct User {
    #[serde(deserialize_with = "nullable")]
    username: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Label {
    #[serde(deserialize_with = "nullable")]
    title: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MergeRequest {
    #[serde(deserialize_with = "nullable")]
    iid: String,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    web_url: String,
    #[serde(deserialize_with = "nullable")]
    draft: bool,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    source_branch: String,
    #[serde(deserialize_with = "nullable")]
    target_branch: String,
    #[serde(deserialize_with = "nullable")]
    conflicts: bool,
    #[serde(deserialize_with = "nullable")]
    approved: bool,
    #[serde(rename = "userNotesCount", deserialize_with = "nullable")]
    notes: i64,
    #[serde(rename = "diffStatsSummary")]
    diff_stats: Option<DiffStats>,
    author: Option<User>,
    #[serde(deserialize_with = "nullable")]
    project: Project,
    source_project: Option<SourceProject>,
    #[serde(deserialize_with = "nullable")]
    approved_by: Nodes<User>,
    head_pipeline: Option<Pipeline>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DiffStats {
    #[serde(deserialize_with = "nullable")]
    additions: i64,
    #[serde(deserialize_with = "nullable")]
    deletions: i64,
    #[serde(deserialize_with = "nullable")]
    file_count: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Project {
    #[serde(deserialize_with = "nullable")]
    full_path: String,
    #[serde(deserialize_with = "nullable")]
    web_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SourceProject {
    #[serde(deserialize_with = "nullable")]
    full_path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Pipeline {
    #[serde(deserialize_with = "nullable")]
    status: String,
    #[serde(deserialize_with = "nullable")]
    jobs: Nodes<Job>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Job {
    #[serde(deserialize_with = "nullable")]
    name: String,
    #[serde(deserialize_with = "nullable")]
    status: String,
    #[serde(deserialize_with = "nullable")]
    web_path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Connection {
    #[serde(deserialize_with = "nodes")]
    nodes: Vec<MergeRequest>,
    #[serde(deserialize_with = "nullable")]
    page_info: PageInfo,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PageInfo {
    #[serde(deserialize_with = "nullable")]
    has_next_page: bool,
    #[serde(deserialize_with = "nullable")]
    end_cursor: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct UserData {
    current_user: Option<CurrentUser>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CurrentUser {
    #[serde(deserialize_with = "nullable")]
    username: String,
    #[serde(deserialize_with = "nullable")]
    review: Connection,
    #[serde(deserialize_with = "nullable")]
    authored: Connection,
    #[serde(deserialize_with = "nullable")]
    assigned: Connection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TrackedData {
    project: Option<TrackedProject>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TrackedProject {
    #[serde(deserialize_with = "nullable")]
    merge_requests: Connection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Issue {
    #[serde(deserialize_with = "nullable")]
    iid: String,
    #[serde(deserialize_with = "nullable")]
    title: String,
    #[serde(deserialize_with = "nullable")]
    web_url: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "timestamp")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    state: String,
    #[serde(deserialize_with = "nullable")]
    reference: String,
    #[serde(rename = "userNotesCount", deserialize_with = "nullable")]
    notes: i64,
    author: Option<User>,
    #[serde(deserialize_with = "nullable")]
    labels: Nodes<Label>,
    #[serde(deserialize_with = "nullable")]
    assignees: Nodes<User>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct IssuesData {
    #[serde(deserialize_with = "nullable")]
    assigned: Nodes<Issue>,
    #[serde(deserialize_with = "nullable")]
    authored: Nodes<Issue>,
    current_user: Option<TodoUser>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TodoUser {
    #[serde(deserialize_with = "nullable")]
    todos: Nodes<Todo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Todo {
    #[serde(deserialize_with = "nullable")]
    target: Issue,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadData {
    project: Option<ThreadProject>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ThreadProject {
    merge_request: Option<Discussion>,
    issue: Option<Discussion>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Discussion {
    #[serde(deserialize_with = "nullable")]
    description: String,
    #[serde(deserialize_with = "nullable")]
    notes: Nodes<Note>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Note {
    #[serde(deserialize_with = "nullable")]
    body: String,
    #[serde(deserialize_with = "timestamp")]
    created_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "nullable")]
    system: bool,
    author: Option<User>,
    position: Option<Position>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Position {
    #[serde(deserialize_with = "nullable")]
    file_path: String,
    #[serde(deserialize_with = "nullable")]
    new_line: i64,
    #[serde(deserialize_with = "nullable")]
    old_line: i64,
}

fn state(status: &str) -> CiState {
    match status.to_uppercase().as_str() {
        "SUCCESS" => CiState::Success,
        "FAILED" => CiState::Failure,
        "CANCELED" | "CANCELING" | "SKIPPED" => CiState::Canceled,
        "CREATED"
        | "WAITING_FOR_RESOURCE"
        | "PREPARING"
        | "PENDING"
        | "RUNNING"
        | "SCHEDULED"
        | "MANUAL"
        | "WAITING_FOR_CALLBACK" => CiState::Pending,
        _ => CiState::None,
    }
}

/// Escapes one URL path segment, so a project path travels as a single
/// segment of the REST endpoint.
fn path_escape(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~$&+=:@".contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    escaped
}

impl GitLab {
    pub(super) fn new(instance: Instance, tracked: Vec<String>) -> Self {
        Self { instance, tracked }
    }

    fn glab(&self, ctx: &Ctx, dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>> {
        run(
            ctx,
            dir,
            &[("GITLAB_HOST", self.instance.host.as_str())],
            "glab",
            args,
        )
    }

    fn graphql(&self, ctx: &Ctx, fields: &[&str]) -> Result<Vec<u8>> {
        let mut args = vec!["api", "graphql", "--hostname", self.instance.host.as_str()];
        for field in fields {
            args.extend(["-f", field]);
        }
        self.glab(ctx, None, &args)
    }

    /// The -R value accepted by glab; the full URL pins the host.
    fn repo_arg(&self, pr: &Item) -> String {
        if pr.repo_url.is_empty() {
            format!("https://{}/{}", self.instance.host, pr.repo)
        } else {
            pr.repo_url.clone()
        }
    }

    fn mr(
        &self,
        ctx: &Ctx,
        dir: Option<&Path>,
        action: &str,
        pr: &Item,
        flags: &[&str],
    ) -> Result<()> {
        let number = pr.number.to_string();
        let repo = self.repo_arg(pr);
        let mut args = vec!["mr", action, number.as_str(), "-R", repo.as_str()];
        args.extend_from_slice(flags);
        self.glab(ctx, dir, &args).map(drop)
    }

    fn unauthenticated(&self) -> Error {
        msg!(
            "glab não autenticado em %s. Rode: glab auth login --hostname %s",
            &self.instance.host,
            &self.instance.host
        )
    }

    /// Adds open issues assigned to, authored by or mentioning the user.
    fn list_issues(&self, ctx: &Ctx, me: &str, acc: &mut Accumulator) -> Result<()> {
        let out = self.graphql(
            ctx,
            &[&format!("query={ISSUES_QUERY}"), &format!("me={me}")],
        )?;
        let resp: Response<IssuesData> =
            decode(&out).map_err(|err| msg!("resposta inválida do glab: %w", Error::from(err)))?;
        if let Some(error) = resp.errors.first()
            && resp.data.current_user.is_none()
        {
            return Err(Error::external(format!(
                "glab graphql (issues): {}",
                error.message
            )));
        }
        for issue in resp.data.assigned.nodes {
            acc.add(self.convert_issue(issue), Relation::ASSIGNED);
        }
        for issue in resp.data.authored.nodes {
            acc.add(self.convert_issue(issue), Relation::AUTHORED);
        }
        if let Some(user) = resp.data.current_user {
            for todo in user.todos.nodes {
                if todo.target.iid.is_empty() || todo.target.state != "opened" {
                    continue;
                }
                acc.add(self.convert_issue(todo.target), Relation::MENTIONED);
            }
        }
        Ok(())
    }

    fn convert_issue(&self, issue: Issue) -> Item {
        let repo = match issue.reference.rfind('#') {
            Some(index) if index > 0 => &issue.reference[..index],
            _ => issue.reference.as_str(),
        };
        let repo_url = issue
            .web_url
            .split_once("/-/")
            .map_or(issue.web_url.as_str(), |(before, _)| before);
        Item {
            kind: Kind::Issue,
            instance: self.instance.name.clone(),
            provider: Provider::GitLab,
            host: self.instance.host.clone(),
            repo: repo.to_string(),
            repo_url: repo_url.to_string(),
            number: issue.iid.parse().unwrap_or(0),
            title: issue.title,
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            comments: issue.notes,
            labels: issue
                .labels
                .nodes
                .into_iter()
                .map(|label| label.title)
                .collect(),
            assignees: issue
                .assignees
                .nodes
                .into_iter()
                .map(|user| user.username)
                .collect(),
            author: issue
                .author
                .map(|author| author.username)
                .unwrap_or_default(),
            url: issue.web_url,
            ..Item::default()
        }
    }

    fn convert(&self, mr: MergeRequest, me: &str) -> Item {
        let me = me.to_lowercase();
        let mut pr = Item {
            instance: self.instance.name.clone(),
            provider: Provider::GitLab,
            host: self.instance.host.clone(),
            number: mr.iid.parse().unwrap_or(0),
            title: mr.title,
            url: mr.web_url,
            draft: mr.draft,
            source_branch: mr.source_branch,
            target_branch: mr.target_branch,
            created_at: mr.created_at,
            updated_at: mr.updated_at,
            conflicts: mr.conflicts,
            comments: mr.notes,
            from_fork: mr
                .source_project
                .is_some_and(|source| source.full_path != mr.project.full_path),
            repo: mr.project.full_path,
            repo_url: mr.project.web_url,
            author: mr.author.map(|author| author.username).unwrap_or_default(),
            review: if mr.approved {
                REVIEW_APPROVED
            } else {
                REVIEW_REQUIRED
            }
            .to_string(),
            ..Item::default()
        };
        if let Some(stats) = mr.diff_stats {
            pr.additions = stats.additions;
            pr.deletions = stats.deletions;
            pr.files = stats.file_count;
        }
        for approver in mr.approved_by.nodes {
            if approver.username.to_lowercase() == me {
                pr.approved_by_me = true;
            }
            pr.approved_by.push(approver.username);
        }
        if let Some(pipeline) = mr.head_pipeline {
            pr.ci = state(&pipeline.status);
            for job in pipeline.jobs.nodes {
                let url = if job.web_path.is_empty() {
                    String::new()
                } else {
                    format!("https://{}{}", self.instance.host, job.web_path)
                };
                pr.checks.push(Check {
                    name: job.name,
                    state: state(&job.status),
                    url,
                });
            }
        }
        pr
    }
}

impl Client for GitLab {
    fn instance(&self) -> &Instance {
        &self.instance
    }

    fn tool(&self) -> &'static str {
        "glab"
    }

    fn list(&self, ctx: &Ctx) -> Result<Vec<Item>> {
        let out = self.graphql(ctx, &[&format!("query={QUERY}")])?;
        let resp: Response<UserData> =
            decode(&out).map_err(|err| msg!("resposta inválida do glab: %w", Error::from(err)))?;
        let Some(user) = resp.data.current_user else {
            return Err(match resp.errors.first() {
                Some(error) => Error::external(format!("glab graphql: {}", error.message)),
                None => self.unauthenticated(),
            });
        };
        let me = user.username;
        let mut acc = Accumulator::default();
        for (relation, connection) in [
            (Relation::REVIEW_REQUESTED, user.review),
            (Relation::AUTHORED, user.authored),
            (Relation::ASSIGNED, user.assigned),
        ] {
            for mr in connection.nodes {
                acc.add(self.convert(mr, &me), relation);
            }
        }
        for repo in &self.tracked {
            let mut cursor = String::new();
            loop {
                let query = format!("query={TRACKED_QUERY}");
                let repo_field = format!("repo={repo}");
                let cursor_field = format!("cursor={cursor}");
                let mut fields = vec![query.as_str(), repo_field.as_str()];
                if !cursor.is_empty() {
                    fields.push(&cursor_field);
                }
                let out = self.graphql(ctx, &fields)?;
                let tracked: Response<TrackedData> = decode(&out).map_err(|err| {
                    msg!(
                        "resposta inválida do glab para %s: %w",
                        repo,
                        Error::from(err)
                    )
                })?;
                if let Some(error) = tracked.errors.first() {
                    return Err(Error::external(format!(
                        "glab graphql ({repo}): {}",
                        error.message
                    )));
                }
                let Some(project) = tracked.data.project else {
                    return Err(msg!(
                        "projeto %s não encontrado em %s",
                        repo,
                        &self.instance.host
                    ));
                };
                let page = project.merge_requests;
                for mr in page.nodes {
                    acc.add(self.convert(mr, &me), Relation::NONE);
                }
                let next = page.page_info.end_cursor;
                // A server that repeats its cursor must not loop forever.
                if !page.page_info.has_next_page || next.is_empty() || next == cursor {
                    break;
                }
                cursor = next;
            }
        }
        self.list_issues(ctx, &me, &mut acc)?;
        Ok(acc.list())
    }

    fn thread(&self, ctx: &Ctx, item: &Item) -> Result<Thread> {
        let field = if item.is_issue() {
            "issue"
        } else {
            "mergeRequest"
        };
        let out = self.graphql(
            ctx,
            &[
                &format!("query={}", THREAD_QUERY.replacen("%s", field, 1)),
                &format!("path={}", item.repo),
                &format!("iid={}", item.number),
            ],
        )?;
        let resp: Response<ThreadData> =
            decode(&out).map_err(|err| msg!("resposta inválida do glab: %w", Error::from(err)))?;
        let project = resp.data.project.unwrap_or_default();
        let Some(discussion) = project.merge_request.or(project.issue) else {
            return Err(match resp.errors.first() {
                Some(error) => Error::external(format!("glab graphql: {}", error.message)),
                None => msg!("%s%s não encontrado", &item.repo, item.reference()),
            });
        };
        let mut thread = Thread {
            body: discussion.description,
            comments: Vec::new(),
        };
        for note in discussion.notes.nodes {
            if note.system {
                continue;
            }
            let mut comment = Comment {
                body: note.body,
                created_at: note.created_at,
                author: note
                    .author
                    .map(|author| author.username)
                    .unwrap_or_default(),
                ..Comment::default()
            };
            if let Some(position) = note.position {
                comment.path = position.file_path;
                comment.line = if position.new_line == 0 {
                    position.old_line
                } else {
                    position.new_line
                };
            }
            thread.comments.push(comment);
        }
        sort_comments(&mut thread.comments);
        Ok(thread)
    }

    fn add_comment(&self, ctx: &Ctx, item: &Item, body: &str) -> Result<()> {
        let kind = if item.is_issue() {
            "issues"
        } else {
            "merge_requests"
        };
        let endpoint = format!(
            "projects/{}/{kind}/{}/notes",
            path_escape(&item.repo),
            item.number
        );
        let body = format!("body={body}");
        self.glab(
            ctx,
            None,
            &[
                "api",
                "--hostname",
                self.instance.host.as_str(),
                "--method",
                "POST",
                endpoint.as_str(),
                "-f",
                body.as_str(),
            ],
        )
        .map(drop)
    }

    fn approve(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.mr(ctx, None, "approve", pr, &[])
    }

    fn merge(&self, ctx: &Ctx, pr: &Item, options: &MergeOptions) -> Result<()> {
        let auto = format!("--auto-merge={}", options.auto);
        let mut flags = vec!["--yes", auto.as_str()];
        match merge_method(&options.method) {
            "squash" => flags.push("--squash"),
            "rebase" => flags.push("--rebase"),
            _ => {}
        }
        if options.delete_branch {
            flags.push("--remove-source-branch");
        }
        self.mr(ctx, None, "merge", pr, &flags)
    }

    fn close(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.mr(ctx, None, "close", pr, &[])
    }

    fn set_draft(&self, ctx: &Ctx, pr: &Item, draft: bool) -> Result<()> {
        let flag = if draft { "--draft" } else { "--ready" };
        self.mr(ctx, None, "update", pr, &[flag, "--yes"])
    }

    fn update_branch(&self, ctx: &Ctx, pr: &Item) -> Result<()> {
        self.mr(ctx, None, "rebase", pr, &[])
    }

    fn checkout(&self, ctx: &Ctx, pr: &Item, dir: &Path) -> Result<()> {
        // An empty directory means the current one, as it does for the CLI.
        let dir = (!dir.as_os_str().is_empty()).then_some(dir);
        self.mr(ctx, dir, "checkout", pr, &[])
    }

    fn head_ref(&self, pr: &Item) -> String {
        format!("refs/merge-requests/{}/head", pr.number)
    }

    fn auth_status(&self, ctx: &Ctx) -> Result<()> {
        // No GITLAB_HOST here: --hostname already selects the instance.
        match run(
            ctx,
            None,
            &[],
            "glab",
            &["auth", "status", "--hostname", self.instance.host.as_str()],
        ) {
            Ok(_) => Ok(()),
            Err(err) if err.is_missing_tool() => Err(err),
            Err(_) => Err(self.unauthenticated()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(name: &str, host: &str) -> Instance {
        Instance {
            name: name.into(),
            provider: Provider::GitLab,
            host: host.into(),
            ..Instance::default()
        }
    }

    fn corp() -> GitLab {
        GitLab::new(instance("corp", "gitlab.corp"), Vec::new())
    }

    #[test]
    fn states() {
        for (input, want) in [
            ("SUCCESS", CiState::Success),
            ("failed", CiState::Failure),
            ("RUNNING", CiState::Pending),
            ("MANUAL", CiState::Pending),
            ("CANCELED", CiState::Canceled),
            ("", CiState::None),
        ] {
            assert_eq!(state(input), want, "state({input:?})");
        }
    }

    #[test]
    fn convert() {
        let client = corp();
        let mr = MergeRequest {
            iid: "42".into(),
            project: Project {
                full_path: "grp/proj".into(),
                web_url: "https://gitlab.corp/grp/proj".into(),
            },
            source_project: Some(SourceProject {
                full_path: "me/proj".into(),
            }),
            ..MergeRequest::default()
        };
        let pr = client.convert(mr, "me");
        assert_eq!(pr.number, 42);
        assert!(pr.from_fork);
        assert_eq!(pr.reference(), "!42");
        assert_eq!(pr.review, REVIEW_REQUIRED);
        assert_eq!(client.repo_arg(&pr), "https://gitlab.corp/grp/proj");
        assert_eq!(client.head_ref(&pr), "refs/merge-requests/42/head");
    }

    #[test]
    fn convert_reads_approvals_stats_and_pipeline() {
        let mr: MergeRequest = serde_json::from_str(
            r#"{"iid":"7","title":null,"approved":true,"userNotesCount":4,
                "createdAt":"2026-01-02T03:04:05+01:00","updatedAt":null,
                "diffStatsSummary":{"additions":1,"deletions":2,"fileCount":3},
                "author":{"username":"ana"},"project":{"fullPath":"g/p","webUrl":"https://gitlab.corp/g/p"},
                "sourceProject":{"fullPath":"g/p"},"approvedBy":{"nodes":[{"username":"bob"},{"username":"ME"}]},
                "headPipeline":{"status":"RUNNING","jobs":{"nodes":[
                    {"name":"build","status":"SUCCESS","webPath":"/g/p/-/jobs/1"},
                    {"name":"test","status":"FAILED","webPath":null}]}}}"#,
        )
        .unwrap();
        let pr = corp().convert(mr, "me");
        assert!(!pr.from_fork);
        assert_eq!(pr.review, REVIEW_APPROVED);
        assert_eq!(pr.approved_by, ["bob", "ME"]);
        assert!(pr.approved_by_me);
        assert_eq!(
            (pr.additions, pr.deletions, pr.files, pr.comments),
            (1, 2, 3, 4)
        );
        assert_eq!(pr.author, "ana");
        assert_eq!(
            pr.created_at.unwrap().to_rfc3339(),
            "2026-01-02T02:04:05+00:00"
        );
        assert_eq!(pr.updated_at, None);
        assert_eq!(pr.ci, CiState::Pending);
        assert_eq!(
            pr.checks,
            [
                Check {
                    name: "build".into(),
                    state: CiState::Success,
                    url: "https://gitlab.corp/g/p/-/jobs/1".into()
                },
                Check {
                    name: "test".into(),
                    state: CiState::Failure,
                    url: String::new()
                },
            ]
        );
    }

    #[test]
    fn convert_issue() {
        let issue = Issue {
            iid: "9".into(),
            reference: "grp/sub/proj#9".into(),
            web_url: "https://gitlab.corp/grp/sub/proj/-/issues/9".into(),
            notes: 3,
            labels: Nodes {
                nodes: vec![Label {
                    title: "bug".into(),
                }],
            },
            ..Issue::default()
        };
        let item = corp().convert_issue(issue);
        assert_eq!(item.repo, "grp/sub/proj");
        assert_eq!(item.repo_url, "https://gitlab.corp/grp/sub/proj");
        assert_eq!(item.number, 9);
        assert!(item.is_issue());
        assert_eq!(item.reference(), "#9");
        assert_eq!(item.comments, 3);
        assert_eq!(item.labels.len(), 1);
        let mr = Item {
            kind: Kind::Pr,
            instance: "corp".into(),
            repo: "grp/sub/proj".into(),
            number: 9,
            ..Item::default()
        };
        assert_ne!(
            item.key(),
            mr.key(),
            "issue and MR with the same number must have different keys"
        );
    }

    #[test]
    fn invalid_timestamps_fail_decoding() {
        assert!(serde_json::from_str::<Issue>(r#"{"createdAt":"yesterday"}"#).is_err());
        assert!(serde_json::from_str::<Issue>(r#"{"createdAt":""}"#).is_err());
    }

    #[test]
    fn path_segments_are_escaped() {
        assert_eq!(path_escape("grp/sub proj"), "grp%2Fsub%20proj");
        assert_eq!(path_escape("a-b_c.d~e$&+=:@"), "a-b_c.d~e$&+=:@");
        assert_eq!(path_escape("é?;,"), "%C3%A9%3F%3B%2C");
    }

    #[cfg(unix)]
    mod cli {
        use std::collections::BTreeMap;
        use std::fs;
        use std::process::Command;

        use super::super::*;
        use super::instance;

        /// Appends GITLAB_HOST and the arguments of each call to `calls`, then
        /// answers with `response.N` for the Nth call, or fails when `fail`
        /// exists.
        const SCRIPT: &str = r#"#!/bin/sh
dir=$(dirname "$0")
printf '%s\037' "$GITLAB_HOST" "$@" >> "$dir/calls"
printf '\036' >> "$dir/calls"
if [ -f "$dir/fail" ]; then
  echo "permission denied" >&2
  exit 1
fi
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
if [ -f "$dir/response.$n" ]; then cat "$dir/response.$n"; else echo '{}'; fi
"#;

        #[derive(Debug, PartialEq)]
        struct Call {
            host: String,
            args: Vec<String>,
        }

        struct Fake {
            dir: tempfile::TempDir,
        }

        impl Fake {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                let script = dir.path().join("glab");
                // A child writes the script: a descriptor opened for writing in
                // this process could leak into a child forked by any other
                // test and make executing the script fail with ETXTBSY.
                let status = Command::new("sh")
                    .args(["-c", r#"printf '%s' "$2" > "$1" && chmod 755 "$1""#, "sh"])
                    .arg(&script)
                    .arg(SCRIPT)
                    .status()
                    .unwrap();
                assert!(status.success());
                Self { dir }
            }

            fn ctx(&self) -> Ctx {
                let script = self.dir.path().join("glab").to_string_lossy().into_owned();
                Ctx::background().with_paths(&BTreeMap::from([("glab".to_string(), script)]))
            }

            fn respond(&self, call: usize, body: &str) {
                fs::write(self.dir.path().join(format!("response.{call}")), body).unwrap();
            }

            fn fail(&self) {
                fs::write(self.dir.path().join("fail"), "").unwrap();
            }

            fn calls(&self) -> Vec<Call> {
                let log = fs::read_to_string(self.dir.path().join("calls")).unwrap();
                log.split_terminator('\u{1e}')
                    .map(|record| {
                        let mut fields = record.split_terminator('\u{1f}').map(str::to_string);
                        Call {
                            host: fields.next().unwrap_or_default(),
                            args: fields.collect(),
                        }
                    })
                    .collect()
            }
        }

        fn work() -> std::sync::Arc<dyn Client> {
            crate::provider::new(&instance("work", "git.company.com"), &[])
        }

        #[test]
        fn draft_commands() {
            for draft in [false, true] {
                let fake = Fake::new();
                let client = work();
                let ctx = fake.ctx();
                let item = Item {
                    repo: "team/sub/service".into(),
                    number: 42,
                    ..Item::default()
                };
                client.set_draft(&ctx, &item, draft).unwrap();
                let calls = fake.calls();
                assert_eq!(calls.len(), 1, "calls = {calls:?}");
                let flag = if draft { "--draft" } else { "--ready" };
                assert_eq!(
                    calls[0].args,
                    [
                        "mr",
                        "update",
                        "42",
                        "-R",
                        "https://git.company.com/team/sub/service",
                        flag,
                        "--yes"
                    ]
                );
                assert_eq!(calls[0].host, "git.company.com", "wrong GitLab host");
                fake.fail();
                assert!(
                    client.set_draft(&ctx, &item, draft).is_err(),
                    "CLI failure ignored"
                );
            }
        }

        #[test]
        fn update_branch_commands() {
            for fail in [false, true] {
                let fake = Fake::new();
                if fail {
                    fake.fail();
                }
                let item = Item {
                    repo: "team/service".into(),
                    number: 42,
                    ..Item::default()
                };
                let result = work().update_branch(&fake.ctx(), &item);
                assert_eq!(result.is_err(), fail, "result = {result:?}, failure {fail}");
                let calls = fake.calls();
                assert_eq!(calls.len(), 1, "calls = {calls:?}");
                assert_eq!(calls[0].host, "git.company.com", "wrong GitLab host");
                assert_eq!(
                    calls[0].args,
                    [
                        "mr",
                        "rebase",
                        "42",
                        "-R",
                        "https://git.company.com/team/service"
                    ]
                );
            }
        }

        #[test]
        fn cli_failures_keep_the_command_and_its_output() {
            let fake = Fake::new();
            fake.fail();
            let err = work().approve(&fake.ctx(), &Item::default()).unwrap_err();
            assert_eq!(err, Error::external("glab mr approve: permission denied"));
        }

        #[test]
        fn pull_request_commands() {
            let fake = Fake::new();
            let client = work();
            let ctx = fake.ctx();
            let pr = Item {
                repo: "team/service".into(),
                repo_url: "https://other.host/team/service".into(),
                number: 7,
                ..Item::default()
            };
            client.approve(&ctx, &pr).unwrap();
            client.close(&ctx, &pr).unwrap();
            client.checkout(&ctx, &pr, fake.dir.path()).unwrap();
            client.merge(&ctx, &pr, &MergeOptions::default()).unwrap();
            let squash = MergeOptions {
                method: "squash".into(),
                auto: true,
                delete_branch: true,
            };
            client.merge(&ctx, &pr, &squash).unwrap();
            let rebase = MergeOptions {
                method: "rebase".into(),
                ..MergeOptions::default()
            };
            client.merge(&ctx, &pr, &rebase).unwrap();
            let repo = "https://other.host/team/service";
            let want: [&[&str]; 6] = [
                &["mr", "approve", "7", "-R", repo],
                &["mr", "close", "7", "-R", repo],
                &["mr", "checkout", "7", "-R", repo],
                &[
                    "mr",
                    "merge",
                    "7",
                    "-R",
                    repo,
                    "--yes",
                    "--auto-merge=false",
                ],
                &[
                    "mr",
                    "merge",
                    "7",
                    "-R",
                    repo,
                    "--yes",
                    "--auto-merge=true",
                    "--squash",
                    "--remove-source-branch",
                ],
                &[
                    "mr",
                    "merge",
                    "7",
                    "-R",
                    repo,
                    "--yes",
                    "--auto-merge=false",
                    "--rebase",
                ],
            ];
            let calls = fake.calls();
            assert_eq!(calls.len(), want.len(), "calls = {calls:?}");
            for (call, want) in calls.iter().zip(want) {
                assert_eq!(call.args, want);
                assert_eq!(call.host, "git.company.com");
            }
        }

        #[test]
        fn comments_are_posted_to_the_escaped_project() {
            let fake = Fake::new();
            let client = work();
            let ctx = fake.ctx();
            let mut item = Item {
                repo: "team/sub/service".into(),
                number: 5,
                ..Item::default()
            };
            client.add_comment(&ctx, &item, "first\nsecond").unwrap();
            item.kind = Kind::Issue;
            client.add_comment(&ctx, &item, "x").unwrap();
            let calls = fake.calls();
            assert_eq!(
                calls[0].args,
                [
                    "api",
                    "--hostname",
                    "git.company.com",
                    "--method",
                    "POST",
                    "projects/team%2Fsub%2Fservice/merge_requests/5/notes",
                    "-f",
                    "body=first\nsecond"
                ]
            );
            assert_eq!(
                calls[1].args[5],
                "projects/team%2Fsub%2Fservice/issues/5/notes"
            );
        }

        #[test]
        fn auth_status_explains_how_to_log_in() {
            let fake = Fake::new();
            let client = work();
            client.auth_status(&fake.ctx()).unwrap();
            assert_eq!(
                fake.calls()[0].args,
                ["auth", "status", "--hostname", "git.company.com"]
            );
            fake.fail();
            let err = client.auth_status(&fake.ctx()).unwrap_err();
            assert_eq!(
                err.text("en"),
                "glab not authenticated on git.company.com. Run: glab auth login --hostname git.company.com"
            );
            let missing = fake
                .dir
                .path()
                .join("missing-glab")
                .to_string_lossy()
                .into_owned();
            let ctx =
                Ctx::background().with_paths(&BTreeMap::from([("glab".to_string(), missing)]));
            assert!(client.auth_status(&ctx).unwrap_err().is_missing_tool());
        }

        #[test]
        fn list_merges_related_tracked_and_issue_queries() {
            let fake = Fake::new();
            let mr = |iid: u32| {
                format!(
                    r#"{{"iid":"{iid}","title":"MR {iid}","project":{{"fullPath":"grp/all","webUrl":"https://git.company.com/grp/all"}}}}"#
                )
            };
            fake.respond(
                1,
                &format!(
                    r#"{{"data":{{"currentUser":{{"username":"me",
                        "review":{{"nodes":[{}]}},"authored":{{"nodes":[{}]}},"assigned":{{"nodes":null}}}}}}}}"#,
                    mr(1),
                    mr(1)
                ),
            );
            fake.respond(
                2,
                &format!(
                    r#"{{"data":{{"project":{{"mergeRequests":{{"nodes":[{},{}],
                        "pageInfo":{{"hasNextPage":true,"endCursor":"c1"}}}}}}}}}}"#,
                    mr(1),
                    mr(2)
                ),
            );
            // The repeated cursor ends the pagination although a next page is announced.
            fake.respond(
                3,
                &format!(
                    r#"{{"data":{{"project":{{"mergeRequests":{{"nodes":[{}],
                        "pageInfo":{{"hasNextPage":true,"endCursor":"c1"}}}}}}}}}}"#,
                    mr(3)
                ),
            );
            let issue = |iid: u32, state: &str| {
                format!(
                    r#"{{"iid":"{iid}","state":"{state}","reference":"grp/all#{iid}",
                        "webUrl":"https://git.company.com/grp/all/-/issues/{iid}","author":{{"username":"ana"}},
                        "assignees":{{"nodes":[{{"username":"me"}}]}}}}"#
                )
            };
            fake.respond(
                4,
                &format!(
                    r#"{{"errors":[{{"message":"partial"}}],"data":{{"assigned":{{"nodes":[{}]}},"authored":{{"nodes":[{}]}},
                        "currentUser":{{"todos":{{"nodes":[{{"target":{}}},{{"target":{}}},{{"target":null}},{{"target":{{}}}}]}}}}}}}}"#,
                    issue(1, "opened"),
                    issue(1, "opened"),
                    issue(2, "opened"),
                    issue(3, "closed")
                ),
            );
            let client = GitLab::new(instance("work", "git.company.com"), vec!["grp/all".into()]);
            let items = client.list(&fake.ctx()).unwrap();
            let got: Vec<(Kind, i64, Relation)> = items
                .iter()
                .map(|item| (item.kind, item.number, item.relations))
                .collect();
            assert_eq!(
                got,
                [
                    (Kind::Pr, 1, Relation::REVIEW_REQUESTED | Relation::AUTHORED),
                    (Kind::Pr, 2, Relation::NONE),
                    (Kind::Pr, 3, Relation::NONE),
                    (Kind::Issue, 1, Relation::ASSIGNED | Relation::AUTHORED),
                    (Kind::Issue, 2, Relation::MENTIONED),
                ]
            );
            assert_eq!(items[3].assignees, ["me"]);
            assert_eq!(items[3].author, "ana");

            let graphql = |fields: &[&str]| {
                let mut args: Vec<String> = ["api", "graphql", "--hostname", "git.company.com"]
                    .map(String::from)
                    .to_vec();
                for field in fields {
                    args.extend(["-f".to_string(), field.to_string()]);
                }
                Call {
                    host: "git.company.com".into(),
                    args,
                }
            };
            let tracked = format!("query={TRACKED_QUERY}");
            assert_eq!(
                fake.calls(),
                [
                    graphql(&[&format!("query={QUERY}")]),
                    graphql(&[&tracked, "repo=grp/all"]),
                    graphql(&[&tracked, "repo=grp/all", "cursor=c1"]),
                    graphql(&[&format!("query={ISSUES_QUERY}"), "me=me"]),
                ]
            );
        }

        #[test]
        fn list_reports_authentication_graphql_and_decoding_errors() {
            let client = GitLab::new(instance("work", "git.company.com"), vec!["grp/all".into()]);
            let list = |responses: &[&str]| {
                let fake = Fake::new();
                for (index, body) in responses.iter().enumerate() {
                    fake.respond(index + 1, body);
                }
                client.list(&fake.ctx()).unwrap_err()
            };
            let user = r#"{"data":{"currentUser":{"username":"me"}}}"#;

            let err = list(&["null"]);
            assert!(err.is("glab não autenticado em %s. Rode: glab auth login --hostname %s"));
            let err = list(&[r#"{"data":null,"errors":[{"message":"boom"}]}"#]);
            assert_eq!(err, Error::external("glab graphql: boom"));
            let err = list(&["not json"]);
            assert!(err.is("resposta inválida do glab: %w"));

            let err = list(&[user, "["]);
            assert!(err.is("resposta inválida do glab para %s: %w"));
            let err = list(&[
                user,
                r#"{"data":{"project":{}},"errors":[{"message":"boom"}]}"#,
            ]);
            assert_eq!(err, Error::external("glab graphql (grp/all): boom"));
            let err = list(&[user, r#"{"data":{"project":null}}"#]);
            assert_eq!(
                err.text("en"),
                "project grp/all not found on git.company.com"
            );

            let project = r#"{"data":{"project":{"mergeRequests":{"nodes":[]}}}}"#;
            let err = list(&[user, project, r#"{"errors":[{"message":"boom"}]}"#]);
            assert_eq!(err, Error::external("glab graphql (issues): boom"));
        }

        #[test]
        fn thread_keeps_user_notes_oldest_first() {
            let fake = Fake::new();
            fake.respond(
                1,
                r#"{"data":{"project":{"mergeRequest":{"description":"Body","notes":{"nodes":[
                    {"body":"later","createdAt":"2026-02-02T00:00:00Z","author":{"username":"bob"},
                     "position":{"filePath":"src/a.rs","newLine":null,"oldLine":12}},
                    {"body":"merged","createdAt":"2026-01-15T00:00:00Z","system":true},
                    {"body":"first","createdAt":"2026-01-01T00:00:00Z","author":null,
                     "position":{"filePath":"src/b.rs","newLine":3,"oldLine":9}}]}}}}}"#,
            );
            let client = work();
            let item = Item {
                repo: "team/service".into(),
                number: 8,
                provider: Provider::GitLab,
                ..Item::default()
            };
            let thread = client.thread(&fake.ctx(), &item).unwrap();
            assert_eq!(thread.body, "Body");
            let got: Vec<(&str, &str, &str, i64)> = thread
                .comments
                .iter()
                .map(|c| (c.author.as_str(), c.body.as_str(), c.path.as_str(), c.line))
                .collect();
            assert_eq!(
                got,
                [
                    ("", "first", "src/b.rs", 3),
                    ("bob", "later", "src/a.rs", 12)
                ]
            );
            let calls = fake.calls();
            assert_eq!(
                calls[0].args[4..],
                [
                    "-f",
                    &format!("query={}", THREAD_QUERY.replace("%s", "mergeRequest")),
                    "-f",
                    "path=team/service",
                    "-f",
                    "iid=8"
                ]
            );

            let issue = Item {
                kind: Kind::Issue,
                ..item.clone()
            };
            fake.respond(2, r#"{"data":{"project":{"issue":{"description":null}}}}"#);
            assert_eq!(
                client.thread(&fake.ctx(), &issue).unwrap(),
                Thread::default()
            );
            assert!(fake.calls()[1].args[5].contains("\n    issue(iid: $iid) {"));

            fake.respond(
                3,
                r#"{"data":{"project":null},"errors":[{"message":"boom"}]}"#,
            );
            let err = client.thread(&fake.ctx(), &item).unwrap_err();
            assert_eq!(err, Error::external("glab graphql: boom"));
            let err = client.thread(&fake.ctx(), &item).unwrap_err();
            assert_eq!(err.text("en"), "team/service!8 not found");
        }
    }

    #[test]
    fn null_connection_entries_do_not_reject_the_response() {
        let users: Nodes<User> =
            serde_json::from_str(r#"{"nodes":[null,{"username":"ana"}]}"#).unwrap();
        assert_eq!(
            users
                .nodes
                .iter()
                .map(|user| user.username.as_str())
                .collect::<Vec<_>>(),
            ["ana"]
        );
        let connection: Connection =
            serde_json::from_str(r#"{"nodes":[null],"pageInfo":null}"#).unwrap();
        assert!(connection.nodes.is_empty());
    }
}
