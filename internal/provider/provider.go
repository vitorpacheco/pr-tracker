// Package provider talks to code hosting platforms through their official CLIs
// (gh for GitHub, glab for GitLab, tea for Gitea). No API tokens are handled by
// pr-tracker: authentication is whatever the CLI is logged in with.
package provider

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

// Relation is how the current user relates to a pull request (bitmask).
type Relation uint8

const (
	ReviewRequested Relation = 1 << iota
	Authored
	Assigned
	Mentioned
)

// Kind distinguishes pull/merge requests from issues.
type Kind uint8

const (
	KindPR Kind = iota
	KindIssue
)

// CIState is the aggregated pipeline result.
type CIState string

const (
	CINone     CIState = ""
	CISuccess  CIState = "success"
	CIFailure  CIState = "failure"
	CIPending  CIState = "pending"
	CICanceled CIState = "canceled"
)

// Check is one pipeline job or status check.
type Check struct {
	Name  string
	State CIState
	URL   string
}

// Review decisions, normalized across providers.
const (
	ReviewApproved         = "approved"
	ReviewChangesRequested = "changes_requested"
	ReviewRequired         = "review_required"
)

// Item is a pull request (GitHub), merge request (GitLab) or an issue.
// Fields about branches, reviews and pipelines only apply to KindPR.
type Item struct {
	Kind     Kind
	Instance string
	Provider config.Provider
	Host     string

	Repo    string // owner/name or group/.../project
	RepoURL string
	Number  int
	Title   string
	URL     string
	Author  string
	Draft   bool

	SourceBranch string
	TargetBranch string
	FromFork     bool

	CreatedAt time.Time
	UpdatedAt time.Time

	Additions, Deletions, Files int

	Review       string
	ApprovedBy   []string
	ApprovedByMe bool
	Conflicts    bool

	CI     CIState
	Checks []Check

	Labels    []string
	Assignees []string
	Comments  int

	Relations Relation
}

// IsIssue reports whether the item is an issue.
func (p *Item) IsIssue() bool { return p.Kind == KindIssue }

// Key identifies an item across instances. GitLab numbers issues and merge
// requests independently, so the kind is part of the key.
func (p *Item) Key() string {
	return fmt.Sprintf("%s|%s|%d#%d", p.Instance, p.Repo, p.Kind, p.Number)
}

// Ref is the display reference ("#12" or "!12").
func (p *Item) Ref() string {
	if p.Provider == config.GitLab && p.Kind == KindPR {
		return fmt.Sprintf("!%d", p.Number)
	}
	return fmt.Sprintf("#%d", p.Number)
}

// Comment is one entry of a conversation.
type Comment struct {
	Author    string
	Body      string
	CreatedAt time.Time
	// Review is set for PR reviews (approved, changes_requested, commented).
	Review string
	// Path and Line locate inline code comments.
	Path string
	Line int
}

// Thread is the description and the comments of an item, oldest first.
type Thread struct {
	Body     string
	Comments []Comment
}

// MergeOptions controls how a PR is merged.
type MergeOptions struct {
	Method       string // merge, squash, rebase
	Auto         bool
	DeleteBranch bool
}

// Client is implemented by every provider integration.
type Client interface {
	Instance() config.Instance
	// Tool is the CLI binary the client depends on.
	Tool() string
	// List returns open PRs and issues related to the current user.
	List(ctx context.Context) ([]Item, error)
	// Thread loads the description and comments of an item.
	Thread(ctx context.Context, it *Item) (*Thread, error)
	// AddComment posts a comment on an item.
	AddComment(ctx context.Context, it *Item, body string) error
	Approve(ctx context.Context, pr *Item) error
	Merge(ctx context.Context, pr *Item, opts MergeOptions) error
	// Close closes a PR without merging it. The source branch is kept.
	Close(ctx context.Context, pr *Item) error
	// Checkout switches the clone at dir to the PR branch.
	Checkout(ctx context.Context, pr *Item, dir string) error
	// HeadRef is the server-side ref that always points to the PR head,
	// available on the base repository even for forks.
	HeadRef(pr *Item) string
	AuthStatus(ctx context.Context) error
}

// ErrNotSupported is returned by providers without a CLI integration yet.
var ErrNotSupported = errors.New("provider ainda não suportado")

// MissingToolError reports that a required CLI is not installed.
type MissingToolError struct{ Tool string }

func (e *MissingToolError) Error() string {
	return fmt.Sprintf("%s não encontrado no PATH. Instale: %s", e.Tool, InstallHint(e.Tool))
}

// InstallHint returns where to get a CLI.
func InstallHint(tool string) string {
	switch tool {
	case "gh":
		return "https://cli.github.com (ex.: brew install gh, pacman -S github-cli, winget install GitHub.cli)"
	case "glab":
		return "https://gitlab.com/gitlab-org/cli (ex.: brew install glab, pacman -S glab, winget install glab.glab)"
	case "tea":
		return "https://gitea.com/gitea/tea (ex.: brew install tea, ou um binário das releases); precisa da 0.14 ou mais nova"
	case "bkt":
		return "https://github.com/avivsinai/bitbucket-cli (integração ainda não implementada)"
	case "hunk":
		return "https://github.com/modem-dev/hunk"
	}
	return tool
}

// ToolAvailable reports whether a CLI is on PATH.
func ToolAvailable(tool string) bool {
	_, err := toolchain.Lookup(context.Background(), tool)
	return err == nil
}

// New returns a provider client. Repository settings are optional so callers
// that only need authentication or tool metadata do not need a full Config.
func New(in config.Instance, repos ...config.Repo) Client {
	var tracked []string
	for _, repo := range repos {
		if repo.Instance == in.Name && repo.TrackAll {
			tracked = append(tracked, repo.Name)
		}
	}
	switch in.Provider {
	case config.GitHub:
		return &github{in: in, tracked: tracked}
	case config.GitLab:
		return &gitlab{in: in, tracked: tracked}
	case config.Gitea:
		return &gitea{in: in, tracked: tracked}
	default:
		return &bitbucket{in: in}
	}
}

// run executes a CLI and returns stdout; stderr is folded into the error.
func run(ctx context.Context, dir string, env []string, name string, args ...string) ([]byte, error) {
	path, err := toolchain.Lookup(ctx, name)
	if err != nil {
		return nil, &MissingToolError{Tool: name}
	}
	cmd := exec.CommandContext(ctx, path, args...)
	cmd.Dir = dir
	cmd.Env = append(toolchain.Environment(ctx), env...)
	// Never let a CLI block on an interactive prompt.
	cmd.Env = append(cmd.Env, "GH_PROMPT_DISABLED=1", "GLAB_NO_PROMPT=1")
	cmd.Stdin = nil
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = strings.TrimSpace(stdout.String())
		}
		if msg == "" {
			msg = err.Error()
		}
		return stdout.Bytes(), fmt.Errorf("%s %s: %s", name, firstArg(args), msg)
	}
	return stdout.Bytes(), nil
}

func firstArg(args []string) string {
	if len(args) >= 2 {
		return args[0] + " " + args[1]
	}
	if len(args) == 1 {
		return args[0]
	}
	return ""
}

func mergeMethod(m string) string {
	switch m {
	case "squash", "rebase":
		return m
	}
	return "merge"
}
