// Package provider talks to code hosting platforms through their official CLIs
// (gh for GitHub, glab for GitLab). No API tokens are handled by pr-tracker:
// authentication is whatever the CLI is logged in with.
package provider

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

// Relation is how the current user relates to a pull request (bitmask).
type Relation uint8

const (
	ReviewRequested Relation = 1 << iota
	Authored
	Assigned
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

// PR is a pull request (GitHub) or merge request (GitLab).
type PR struct {
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

	Relations Relation
}

// Key identifies a PR across instances.
func (p *PR) Key() string { return fmt.Sprintf("%s|%s#%d", p.Instance, p.Repo, p.Number) }

// Ref is the display reference ("#12" or "!12").
func (p *PR) Ref() string {
	if p.Provider == config.GitLab {
		return fmt.Sprintf("!%d", p.Number)
	}
	return fmt.Sprintf("#%d", p.Number)
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
	List(ctx context.Context) ([]PR, error)
	Approve(ctx context.Context, pr *PR) error
	Merge(ctx context.Context, pr *PR, opts MergeOptions) error
	// Checkout switches the clone at dir to the PR branch.
	Checkout(ctx context.Context, pr *PR, dir string) error
	// HeadRef is the server-side ref that always points to the PR head,
	// available on the base repository even for forks.
	HeadRef(pr *PR) string
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
	case "bkt":
		return "https://github.com/avivsinai/bitbucket-cli (integração ainda não implementada)"
	case "hunk":
		return "https://github.com/modem-dev/hunk"
	}
	return tool
}

// ToolAvailable reports whether a CLI is on PATH.
func ToolAvailable(tool string) bool {
	_, err := exec.LookPath(tool)
	return err == nil
}

// New returns the client for an instance.
func New(in config.Instance) Client {
	switch in.Provider {
	case config.GitHub:
		return &github{in: in}
	case config.GitLab:
		return &gitlab{in: in}
	default:
		return &bitbucket{in: in}
	}
}

// run executes a CLI and returns stdout; stderr is folded into the error.
func run(ctx context.Context, dir string, env []string, name string, args ...string) ([]byte, error) {
	if !ToolAvailable(name) {
		return nil, &MissingToolError{Tool: name}
	}
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), env...)
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
