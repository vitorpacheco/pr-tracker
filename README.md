# pr-tracker

Terminal and desktop interfaces for tracking **pull requests** (GitHub, Gitea),
**merge requests** (GitLab), **issues**, and their **comments** across multiple
instances, including self-hosted servers. Communication goes through the
`gh`, `glab`, and `tea` CLIs, using the authentication you already have configured.

- PR tabs: **Review** (review requested from you), **Mine**, **Assigned**, and
  **All**. Open issue tabs: **Assigned**, **Created**, and **Mentions**.
- Open the **conversation** (`v`) to see descriptions and comments as rendered
  Markdown. PR conversations also include reviews (approved / changes requested)
  and inline comments with `file:line`. Press `n` to **comment**.
- See **pipeline/check status** (success, failure, running, canceled) and jobs,
  alongside reviews, approvals, conflicts, and diff size.
- Refresh automatically at a configurable interval (**5 minutes** by default),
  or immediately with `r`.
- Use the keyboard or mouse: tabs, rows, menu items, buttons, and footer shortcuts
  are clickable. Shortcuts appear in the interface; `?` opens help in the TUI.
- Check out a PR in an isolated **worktree** or the **configured clone**, then
  approve, merge, or close without merging, optionally removing the worktree.
- Optional integrations: **hunk** for diffs; **herdr** or **tmux** to open a
  terminal or diff in a new tab.
- Both interfaces support **English** and **Portuguese**, following the computer's
  language by default. Choose a language in settings to override it.

## Requirements

| Tool | Purpose | Required |
|---|---|---|
| `git` | Worktrees and checkout | Yes |
| [`gh`](https://cli.github.com) | GitHub instances (github.com and GHES) | For GitHub |
| [`glab`](https://gitlab.com/gitlab-org/cli) | GitLab instances (gitlab.com and self-hosted) | For GitLab |
| [`tea`](https://gitea.com/gitea/tea) (0.14+) | Gitea instances (gitea.com and self-hosted) | For Gitea |
| [`hunk`](https://github.com/modem-dev/hunk) | View diffs (falls back to `git diff`) | No |
| `herdr` / `tmux` | Open a terminal or diff in a new tab | No |

If a CLI is missing, the interface shows a warning with an installation link.
`pr-tracker doctor` lists missing tools. See [Gitea support](docs/gitea.md) for
what `tea` covers and what requires direct API calls. See
[Bitbucket support](docs/bitbucket.md) for the deferred integration: Bitbucket
has no official CLI.

## Installation

Download the TUI binary or the `pr-tracker-desktop` GUI package from
[releases](https://github.com/vitorpacheco/pr-tracker/releases). Stable releases
are published for `v*` tags; `nightly` is updated on every commit to `main`.
Both executables are published for Linux, macOS, and Windows on `amd64` and
`arm64`. You can also build from source:

```sh
go install github.com/vitorpacheco/pr-tracker@latest
# Or from a clone (installs in ~/.local/bin; override with PREFIX=...):
make install
```

## Usage

```sh
pr-tracker                 # terminal interface
pr-tracker doctor          # check CLIs, authentication, and paths
pr-tracker list            # print PRs to stdout
pr-tracker instance add --provider gitlab --host gitlab.company.com --name work --merge-method squash
pr-tracker instance add --provider gitea --host git.company.com --name gitea
pr-tracker instance list
pr-tracker repo set --instance work --repo group/sub/project --path ~/code/project
```

On first launch, the configuration file is created and seeded with `github.com`
and `gitlab.com` if `gh` and `glab` are authenticated on those hosts. An instance
is also added for each server where `tea` is already logged in. Gitea is commonly
self-hosted, so no default host is probed.

Authenticate self-hosted instances through their CLI:

```sh
gh auth login --hostname github.company.com
glab auth login --hostname gitlab.company.com
tea login add --url https://git.company.com --token <token>
```

`tea` identifies servers by **login name**, not host. pr-tracker discovers the
login from its URL. If a host has multiple accounts, give the instance the same
name as the login you want to use.

### Keyboard shortcuts

| Key | Action |
|---|---|
| `↑↓` `j k`, `g G`, `pgup pgdn` | Navigate |
| `1`–`4` / `5`–`7`, `tab` | PR / issue tabs |
| `/` | Filter by title, repository, author, or branch |
| `enter` / click | Open the item's action menu |
| `v` | View the conversation (description, comments, and reviews) |
| `n` | Write a comment (`ctrl+s` sends, `esc` cancels) |
| `w` | Check out in a worktree, or update an existing one |
| `c` | Check out in the configured local clone (`gh pr checkout` / `glab mr checkout` / `tea pulls checkout`) |
| `d` | Open the diff in hunk (or `git diff`) inside the worktree |
| `t` | Open a terminal in the worktree |
| `a` / `A` | Approve / approve and remove the worktree |
| `m` / `M` | Merge / merge and remove the worktree |
| `X` / `C` | Close without merging / close without merging and remove the worktree (keeps the branch) |
| `x` | Remove the worktree without approving |
| `o` | Open in the browser |
| `R` | Toggle tracking all PRs/MRs in the repository |
| `p` | Set the repository's local folder |
| `r` | Refresh now |
| `i` | Instances (`n` new, `e` edit, `space` enable/disable, `t` test authentication, `D` remove) |
| `s` | Settings |
| `?` | Help |
| `q` | Quit |

These shortcuts describe the TUI. See [desktop development](docs/desktop-development.md)
for GUI shortcuts.

Issues support `v`, `n`, and `o`. Worktree, checkout, approval, merge, and close
actions apply only to PRs. In the conversation screen, navigate with `↑↓`,
`space`/`pgdn`, `g`/`G`, or the mouse wheel; `r` reloads and `esc` goes back.

Approval, merge, close, checkout in the local clone, and removal ask for
confirmation. If a worktree has uncommitted changes, removal asks for a second
confirmation before forcing it.

## Worktrees

Worktrees live in `~/.pr-tracker/worktrees/<repo>-<number>-<hash>`. The stable,
eight-character hash is derived from host, repository, and number, preventing
collisions across instances and repositories with the same name.

- The PR head is fetched using a server ref that also works for forks:
  `refs/pull/N/head` on GitHub and Gitea, or `refs/merge-requests/N/head` on GitLab.
  The worktree uses the local branch `pr-tracker/<N>-<hash>`.
- For PRs from the same repository, the local branch tracks `origin/<branch>`,
  so `git pull` works.
- Removing a worktree also deletes the local branch and ref created by pr-tracker.

The local clone comes from the `[[repos]]` mapping (press `p` or use
`pr-tracker repo set`). Without a mapping, pr-tracker searches `clone_roots`
using `<root>/<repo>`, `<root>/<owner>/<repo>`, and
`<root>/<host>/<owner>/<repo>`. It accepts a folder only if one of its remotes
points to the repository. If no clone is found, it asks for a folder.

## Terminal: herdr, tmux, or inline

`terminal = "auto"` selects **herdr** when running inside it (`HERDR_ENV=1`,
opening a tab through `herdr tab create` and `herdr pane run`). Otherwise, it
selects **tmux** when `$TMUX` is set (`tmux new-window`). Outside both, the TUI
is suspended and the terminal or diff runs in its place until you exit.

## Configuration

| OS | File |
|---|---|
| Linux | `$XDG_CONFIG_HOME/pr-tracker/config.toml` (defaults to `~/.config/pr-tracker/config.toml`) |
| macOS | `~/.config/pr-tracker/config.toml` |
| Windows | `%USERPROFILE%\.config\pr-tracker\config.toml` |

`PR_TRACKER_CONFIG` overrides this path. Example:

```toml
language = "system"          # system | en | pt (shared by TUI and GUI)
refresh_interval = "5m"       # 90s, 5m, 1h… (minimum 15s)
terminal = "auto"             # auto | herdr | tmux | inline
diff_tool = "hunk"            # hunk | git
# worktree_dir = "~/.pr-tracker/worktrees"
clone_roots = ["~/code", "~/work"]

[[instances]]
  name = "github.com"
  provider = "github"
  host = "github.com"
  merge_method = "squash"     # merge | squash | rebase

[[instances]]
  name = "gitea"              # match the tea login name when multiple logins exist
  provider = "gitea"
  host = "git.company.com"

[[instances]]
  name = "work"
  provider = "gitlab"
  host = "gitlab.company.com"
  merge_method = "merge"
  auto_merge = true           # merge once the pipeline passes
  delete_branch = true        # delete the source branch after merging

[[repos]]
  instance = "work"
  name = "group/sub/project"
  path = "~/work/project"
  remote = "origin"           # optional; detected from the URL
  track_all = true            # include all open MRs, regardless of relation
  merge_method = "squash"     # optional; overrides the instance setting
```

### Interface language

In the TUI, press `s` and change **Language**. In the GUI, open **Settings** and
choose **Language**. Options are **System**, **English**, and **Português**.
Saving applies the choice immediately to that interface and persists it in the
shared configuration file. The other interface reads the preference on its next
launch. Existing configuration files without `language` continue to follow the
system.

Automatic detection respects `LC_ALL`, then `LC_MESSAGES`, then `LANG`. For a
non-`C`/`POSIX` locale, the `LANGUAGE` preference list can select the message
language. When
these are absent, macOS uses its preferred language and Windows uses the user's
display language. The browser demo uses the browser's language. Portuguese regional
variants such as `pt_BR.UTF-8` and `pt-PT` select Portuguese; English variants,
`C`/`POSIX`, unknown languages, and unavailable locale settings fall back to
English. Repository titles, descriptions, comments, and external tool output
retain their original language.

In a PR/MR menu, `R` toggles `track_all` for that repository. The repository
entry persists even without a local folder mapping. When enabled, **All**
includes every open PR/MR from the repository; **Review**, **Mine**, and
**Assigned** still show only their respective relations.

## How data is fetched

Each refresh makes a small number of GraphQL calls per instance:

- **GitHub:** one `gh api graphql --hostname <host>` call contains three PR
  searches (`review-requested:@me`, `author:@me`, `assignee:@me`, including the
  latest commit's `statusCheckRollup`) and three issue searches (`assignee:@me`,
  `author:@me`, `mentions:@me`). Each repository with `track_all` adds paginated
  calls to its pull request connection.
- **GitLab:** two `glab api graphql --hostname <host>` calls fetch
  `currentUser.{reviewRequested,authored,assigned}MergeRequests`, including
  `headPipeline` and jobs, and `issues(assigneeUsernames|authorUsername)` plus
  pending mention to-dos (`mentioned`, `directly_addressed`). MRs omit labels to
  stay within GitLab's query complexity limit. Each repository with `track_all`
  adds paginated calls to its merge request connection.
- **Gitea:** no GraphQL; REST through `tea api` performs six searches against
  `/repos/issues/search` for the same tab relations. Since search returns raw
  issues, each PR requires up to three additional calls for details, commit
  checks, and reviews, with at most six concurrent calls. A failed call leaves
  that row without CI data instead of failing the whole list. Repositories with
  `track_all` also query `/repos/{owner}/{repo}/pulls` with pagination.

### Local cache

The interfaces keep the latest successfully fetched complete snapshot of each
instance in SQLite. When pr-tracker starts, cached data appears immediately while
a remote refresh runs in the background. If the server or network is unavailable,
previous data remains visible alongside an error warning.

The database is a disposable cache, not the source of truth. Each successful
refresh atomically replaces that instance's snapshot; failures never erase the
previous snapshot. By default, it lives in the system cache directory
(`$XDG_CACHE_HOME/pr-tracker/cache.db` or `~/.cache/pr-tracker/cache.db` on Linux).
`PR_TRACKER_CACHE` overrides the path. You can delete the file while the app is
closed; the next refresh rebuilds it. Permissions are `0600` on Unix; on Windows,
it inherits the user's cache directory ACLs. It contains item metadata, but no
credentials or conversations loaded on demand.

Conversations are loaded only when opened (`v`):

- GitHub: `issueOrPullRequest`, including comments, reviews, and inline comments.
- GitLab: `notes(filter: ONLY_COMMENTS)`, excluding system notes.

Comments use `gh pr|issue comment` and
`glab api POST projects/:id/(merge_requests|issues)/:iid/notes`.
Actions use `gh pr review/merge/close/checkout -R host/owner/repo` and
`glab mr approve/merge/close/checkout -R <project URL>`.

## Development

```sh
make          # list available commands
make check    # gofmt verification + go vet + tests
make run ARGS=doctor
make vuln     # govulncheck: known dependency vulnerabilities
make dist     # Linux, macOS, and Windows binaries in ./dist
make package  # dist + .tar.gz/.zip + checksums.txt (release artifacts)
```

Interface messages are translated through `internal/i18n/en.json`, shared by
Go and Svelte. Portuguese source messages serve as catalog keys. Keep dynamic
repository content outside translation calls and add coverage for language
selection when changing the configuration or interface adapters.

### CI and releases (GitHub Actions)

| Workflow | Trigger | Purpose |
|---|---|---|
| `ci.yml` | PRs and pushes to other branches | Clean `go mod tidy`, lint, tests, CLI and GUI packages for all six OS/architecture combinations, and `govulncheck` |
| `desktop.yml` | Called by CI or manually | Validate the frontend and build native GUI packages for Linux, macOS, and Windows on `amd64` and `arm64` |
| `nightly.yml` | Push to `main` | Run CI and recreate the `nightly` pre-release with CLI and GUI, pointing to the new commit (`nightly-YYYYMMDD-<sha>`) |
| `release.yml` | Push of a `v*` tag | Run CI and publish CLI and GUI with generated notes; suffixed tags such as `v1.0.0-rc.1` become pre-releases |
| `vulncheck.yml` | Every Monday and manually | Run `govulncheck` on `main` to catch new advisories without requiring a commit |

Dependabot updates Go dependencies and actions weekly. To publish a release:

```sh
git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0
```

Packages: `internal/config` (configuration and paths), `internal/provider`
(hosting integrations), `internal/gitops` (clones and worktrees), `internal/launch`
(herdr/tmux/inline/browser), `internal/app` (shared application logic),
`internal/i18n` (language detection and translations), and `internal/ui`
(Bubble Tea v2).

## Desktop interface (in development)

The Wails/Svelte GUI lives in `desktop/` and shares the TUI backend. On Linux
with GTK 3 and WebKitGTK 4.1, run `make desktop-install desktop-run` to build and
open the native GUI. To explore sample data in the browser, run
`make desktop-dev` and open `http://127.0.0.1:5173/?demo=1`.

See [desktop development](docs/desktop-development.md) for requirements,
configuration, shortcuts, tests, and current cross-platform validation limits.
