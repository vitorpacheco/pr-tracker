// Package gitops manages local clones and per-PR worktrees.
package gitops

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

// Git runs git in dir and returns trimmed stdout.
func Git(ctx context.Context, dir string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0")
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = err.Error()
		}
		return "", fmt.Errorf("git %s: %s", args[0], msg)
	}
	return strings.TrimSpace(stdout.String()), nil
}

// Hash is a short, stable identifier for a PR, used in worktree folder names.
func Hash(pr *provider.Item) string {
	sum := sha256.Sum256([]byte(pr.Host + "|" + pr.Repo + "|" + fmt.Sprint(pr.Number)))
	return hex.EncodeToString(sum[:])[:8]
}

var unsafe = regexp.MustCompile(`[^A-Za-z0-9._-]+`)

// WorktreePath is the deterministic worktree folder of a PR:
// <worktree_dir>/<repo>-<number>-<hash>.
func WorktreePath(cfg *config.Config, pr *provider.Item) (string, error) {
	root, err := cfg.Worktrees()
	if err != nil {
		return "", err
	}
	name := unsafe.ReplaceAllString(filepath.Base(pr.Repo), "_")
	return filepath.Join(root, fmt.Sprintf("%s-%d-%s", name, pr.Number, Hash(pr))), nil
}

// WorktreeExists reports whether the PR worktree folder exists.
func WorktreeExists(cfg *config.Config, pr *provider.Item) (string, bool) {
	p, err := WorktreePath(cfg, pr)
	if err != nil {
		return "", false
	}
	st, err := os.Stat(p)
	return p, err == nil && st.IsDir()
}

// LocalBranch is the branch created for a PR worktree.
func LocalBranch(pr *provider.Item) string {
	return fmt.Sprintf("pr-tracker/%d-%s", pr.Number, Hash(pr))
}

func headRef(pr *provider.Item) string {
	return fmt.Sprintf("refs/pr-tracker/heads/%d-%s", pr.Number, Hash(pr))
}

// ResolveClone finds the local clone of the PR repository: the configured
// path first, then clone_roots (<root>/<name> and <root>/<owner>/<name>),
// accepting a candidate only if one of its remotes points to the repo.
func ResolveClone(ctx context.Context, cfg *config.Config, pr *provider.Item) (path, remote string, ok bool) {
	if r, found := cfg.Repo(pr.Instance, pr.Repo); found && r.Path != "" {
		remote = r.Remote
		if remote == "" {
			remote, _ = matchRemote(ctx, config.ExpandHome(r.Path), pr)
		}
		if remote == "" {
			remote = "origin"
		}
		return config.ExpandHome(r.Path), remote, true
	}
	base := filepath.Base(pr.Repo)
	for _, root := range cfg.CloneRoots {
		root = config.ExpandHome(root)
		for _, cand := range []string{
			filepath.Join(root, base),
			filepath.Join(root, filepath.FromSlash(pr.Repo)),
			filepath.Join(root, pr.Host, filepath.FromSlash(pr.Repo)),
		} {
			if rem, found := matchRemote(ctx, cand, pr); found {
				return cand, rem, true
			}
		}
	}
	return "", "", false
}

// matchRemote returns the remote of dir whose URL points at the PR repo.
func matchRemote(ctx context.Context, dir string, pr *provider.Item) (string, bool) {
	if st, err := os.Stat(dir); err != nil || !st.IsDir() {
		return "", false
	}
	out, err := Git(ctx, dir, "remote", "-v")
	if err != nil {
		return "", false
	}
	want := strings.ToLower(pr.Host + "/" + pr.Repo)
	wantSSH := strings.ToLower(pr.Host + ":" + pr.Repo)
	for _, line := range strings.Split(out, "\n") {
		f := strings.Fields(line)
		if len(f) < 2 {
			continue
		}
		// ToSlash: local remotes on Windows are backslash paths.
		u := strings.ToLower(strings.TrimSuffix(strings.TrimSuffix(filepath.ToSlash(f[1]), "/"), ".git"))
		if strings.HasSuffix(u, want) || strings.HasSuffix(u, wantSSH) {
			return f[0], true
		}
	}
	return "", false
}

// ValidateClone checks that path is a git repository.
func ValidateClone(ctx context.Context, path string) error {
	path = config.ExpandHome(path)
	if st, err := os.Stat(path); err != nil || !st.IsDir() {
		return fmt.Errorf("pasta não existe: %s", path)
	}
	if _, err := Git(ctx, path, "rev-parse", "--git-dir"); err != nil {
		return fmt.Errorf("não é um repositório git: %s", path)
	}
	return nil
}

// CreateWorktree fetches the PR head and adds a worktree for it. When the
// worktree already exists it is fast-forwarded to the latest PR head if clean.
func CreateWorktree(ctx context.Context, cfg *config.Config, client provider.Client, pr *provider.Item, clone, remote string) (string, error) {
	wt, err := WorktreePath(cfg, pr)
	if err != nil {
		return "", err
	}
	// FETCH_HEAD is per-worktree, so the head is fetched into a shared ref.
	head := headRef(pr)
	if _, err := Git(ctx, clone, "fetch", remote, "+"+client.HeadRef(pr)+":"+head); err != nil {
		return "", err
	}
	if _, exists := WorktreeExists(cfg, pr); exists {
		if dirty, _ := Git(ctx, wt, "status", "--porcelain"); dirty == "" {
			_, _ = Git(ctx, wt, "merge", "--ff-only", head)
		}
		return wt, nil
	}
	if err := os.MkdirAll(filepath.Dir(wt), 0o755); err != nil {
		return "", err
	}
	branch := LocalBranch(pr)
	if _, err := Git(ctx, clone, "worktree", "add", "-B", branch, wt, head); err != nil {
		return "", err
	}
	// Track the source branch so pull (and push with push.default=upstream)
	// work for same-repo PRs.
	if !pr.FromFork && pr.SourceBranch != "" {
		if _, err := Git(ctx, wt, "fetch", remote, "+"+pr.SourceBranch+":refs/remotes/"+remote+"/"+pr.SourceBranch); err == nil {
			_, _ = Git(ctx, wt, "branch", "--set-upstream-to="+remote+"/"+pr.SourceBranch)
		}
	}
	// Make the base branch available for diffs inside the worktree.
	if pr.TargetBranch != "" {
		_, _ = Git(ctx, wt, "fetch", remote, "+"+pr.TargetBranch+":refs/remotes/"+remote+"/"+pr.TargetBranch)
	}
	return wt, nil
}

// ErrDirty is returned when a worktree has local changes.
var ErrDirty = errors.New("worktree tem alterações locais")

// RemoveWorktree removes the PR worktree and its local branch.
func RemoveWorktree(ctx context.Context, cfg *config.Config, pr *provider.Item, force bool) error {
	wt, exists := WorktreeExists(cfg, pr)
	if !exists {
		return nil
	}
	common, err := Git(ctx, wt, "rev-parse", "--path-format=absolute", "--git-common-dir")
	if err != nil {
		// Not a valid worktree anymore: just delete the folder.
		return os.RemoveAll(wt)
	}
	main := filepath.Dir(common)
	if !force {
		if dirty, _ := Git(ctx, wt, "status", "--porcelain"); dirty != "" {
			return ErrDirty
		}
	}
	args := []string{"worktree", "remove", wt}
	if force {
		args = []string{"worktree", "remove", "--force", wt}
	}
	if _, err := Git(ctx, main, args...); err != nil {
		return err
	}
	_, _ = Git(ctx, main, "branch", "-D", LocalBranch(pr))
	_, _ = Git(ctx, main, "update-ref", "-d", headRef(pr))
	_, _ = Git(ctx, main, "worktree", "prune")
	return nil
}
