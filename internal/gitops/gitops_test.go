package gitops

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

// fakeClient only provides HeadRef, the single method gitops needs.
type fakeClient struct{ provider.Client }

func (fakeClient) HeadRef(pr *provider.Item) string { return "refs/pull/5/head" }

func git(t *testing.T, dir string, args ...string) string {
	t.Helper()
	out, err := Git(context.Background(), dir, args...)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func setup(t *testing.T) (*config.Config, *provider.Item, string) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	t.Setenv("GIT_AUTHOR_NAME", "t")
	t.Setenv("GIT_AUTHOR_EMAIL", "t@t")
	t.Setenv("GIT_COMMITTER_NAME", "t")
	t.Setenv("GIT_COMMITTER_EMAIL", "t@t")
	root := t.TempDir()
	// The upstream path ends in host/owner/repo so remote matching works.
	up := filepath.Join(root, "remote", "github.com", "o", "r")
	if err := os.MkdirAll(up, 0o755); err != nil {
		t.Fatal(err)
	}
	git(t, up, "init", "-q", "-b", "main")
	os.WriteFile(filepath.Join(up, "a.txt"), []byte("a\n"), 0o644)
	git(t, up, "add", ".")
	git(t, up, "commit", "-qm", "base")
	git(t, up, "checkout", "-qb", "feature")
	os.WriteFile(filepath.Join(up, "b.txt"), []byte("b\n"), 0o644)
	git(t, up, "add", ".")
	git(t, up, "commit", "-qm", "feature")
	git(t, up, "update-ref", "refs/pull/5/head", "HEAD")
	git(t, up, "checkout", "-q", "main")

	clones := filepath.Join(root, "clones")
	git(t, root, "clone", "-q", up, filepath.Join(clones, "r"))

	cfg := config.Default()
	cfg.WorktreeDir = filepath.Join(root, "wt")
	cfg.CloneRoots = []string{clones}
	pr := &provider.Item{Instance: "gh", Host: "github.com", Repo: "o/r", Number: 5, SourceBranch: "feature", TargetBranch: "main"}
	return cfg, pr, filepath.Join(clones, "r")
}

func TestWorktreeLifecycle(t *testing.T) {
	cfg, pr, clone := setup(t)
	ctx := context.Background()

	path, remote, ok := ResolveClone(ctx, cfg, pr)
	if !ok || path != clone || remote != "origin" {
		t.Fatalf("ResolveClone = %q %q %v", path, remote, ok)
	}
	wt, err := CreateWorktree(ctx, cfg, fakeClient{}, pr, path, remote)
	if err != nil {
		t.Fatal(err)
	}
	if want, _ := WorktreePath(cfg, pr); wt != want || filepath.Base(wt) != "r-5-"+Hash(pr) {
		t.Fatalf("worktree path %q", wt)
	}
	if _, err := os.Stat(filepath.Join(wt, "b.txt")); err != nil {
		t.Fatal("PR head not checked out")
	}
	if up := git(t, wt, "rev-parse", "--abbrev-ref", "@{upstream}"); up != "origin/feature" {
		t.Fatalf("upstream = %q", up)
	}
	// Running again reuses the existing worktree.
	if again, err := CreateWorktree(ctx, cfg, fakeClient{}, pr, path, remote); err != nil || again != wt {
		t.Fatalf("recreate: %q %v", again, err)
	}

	os.WriteFile(filepath.Join(wt, "dirty.txt"), []byte("x"), 0o644)
	if err := RemoveWorktree(ctx, cfg, pr, false); !errors.Is(err, ErrDirty) {
		t.Fatalf("expected ErrDirty, got %v", err)
	}
	if err := RemoveWorktree(ctx, cfg, pr, true); err != nil {
		t.Fatal(err)
	}
	if _, exists := WorktreeExists(cfg, pr); exists {
		t.Fatal("worktree still exists")
	}
	if out := git(t, clone, "branch", "--list", LocalBranch(pr)); out != "" {
		t.Fatalf("branch not deleted: %q", out)
	}
}

func TestResolveCloneConfiguredPath(t *testing.T) {
	cfg, pr, clone := setup(t)
	cfg.CloneRoots = nil
	if _, _, ok := ResolveClone(context.Background(), cfg, pr); ok {
		t.Fatal("expected no clone without mapping")
	}
	cfg.SetRepoPath("gh", "o/r", clone)
	if p, _, ok := ResolveClone(context.Background(), cfg, pr); !ok || p != clone {
		t.Fatalf("got %q %v", p, ok)
	}
}

func TestWorktreeUpdateFromSource(t *testing.T) {
	for _, scenario := range []string{"new commits", "unchanged", "dirty", "diverged", "different branch", "fetch failure"} {
		t.Run(scenario, func(t *testing.T) {
			cfg, pr, clone := setup(t)
			ctx := t.Context()
			wt, err := CreateWorktree(ctx, cfg, fakeClient{}, pr, clone, "origin")
			if err != nil {
				t.Fatal(err)
			}
			before := git(t, wt, "rev-parse", "HEAD")
			up := git(t, clone, "remote", "get-url", "origin")
			if scenario != "unchanged" {
				git(t, up, "checkout", "-q", "feature")
				if err := os.WriteFile(filepath.Join(up, "remote.txt"), []byte("new commit\n"), 0o644); err != nil {
					t.Fatal(err)
				}
				git(t, up, "add", "remote.txt")
				git(t, up, "commit", "-qm", "remote update")
				git(t, up, "update-ref", "refs/pull/5/head", "HEAD")
			}
			if scenario == "dirty" || scenario == "diverged" {
				if err := os.WriteFile(filepath.Join(wt, "local.txt"), []byte("local work\n"), 0o644); err != nil {
					t.Fatal(err)
				}
				if scenario == "diverged" {
					git(t, wt, "add", "local.txt")
					git(t, wt, "commit", "-qm", "local commit")
					before = git(t, wt, "rev-parse", "HEAD")
				}
			}
			if scenario == "different branch" {
				git(t, wt, "checkout", "-qb", "other")
			}
			if scenario == "fetch failure" {
				git(t, clone, "remote", "set-url", "origin", filepath.Join(t.TempDir(), "missing"))
			}
			// Forks also use the fetched PR ref, not a same-named branch on origin.
			pr.FromFork = true
			updated, err := CreateWorktree(ctx, cfg, fakeClient{}, pr, clone, "origin")
			after := git(t, wt, "rev-parse", "HEAD")
			if scenario == "new commits" || scenario == "unchanged" {
				want := git(t, up, "rev-parse", "refs/pull/5/head")
				if err != nil || updated != wt || after != want {
					t.Fatalf("update = %q, %v, head %s, want %s", updated, err, after, want)
				}
				return
			}
			if err == nil || updated != "" || after != before {
				t.Fatalf("failed update = %q, %v, head %s, want unchanged %s", updated, err, after, before)
			}
			if scenario == "dirty" && !errors.Is(err, ErrDirty) {
				t.Fatalf("expected ErrDirty, got %v", err)
			}
			if scenario == "dirty" || scenario == "diverged" {
				if contents, err := os.ReadFile(filepath.Join(wt, "local.txt")); err != nil || string(contents) != "local work\n" {
					t.Fatalf("local work lost: %q, %v", contents, err)
				}
			}
		})
	}
}

func TestPushWorktreeUsesSourceBranchWithoutForce(t *testing.T) {
	for _, scenario := range []string{"push", "fork upstream", "missing upstream", "wrong upstream", "diverged"} {
		t.Run(scenario, func(t *testing.T) {
			cfg, pr, clone := setup(t)
			wt, err := CreateWorktree(t.Context(), cfg, fakeClient{}, pr, clone, "origin")
			if err != nil {
				t.Fatal(err)
			}
			up := git(t, clone, "remote", "get-url", "origin")
			before := git(t, up, "rev-parse", "feature")
			main := git(t, up, "rev-parse", "main")
			if err := os.WriteFile(filepath.Join(wt, "local.txt"), []byte("local commit\n"), 0o644); err != nil {
				t.Fatal(err)
			}
			git(t, wt, "add", "local.txt")
			git(t, wt, "commit", "-qm", "local update")
			local := git(t, wt, "rev-parse", "HEAD")
			switch scenario {
			case "fork upstream":
				pr.FromFork = true
				fork := filepath.Join(t.TempDir(), "fork.git")
				git(t, clone, "clone", "--bare", "-q", up, fork)
				git(t, wt, "remote", "add", "fork", fork)
				git(t, wt, "fetch", "-q", "fork")
				git(t, wt, "branch", "--set-upstream-to=fork/feature")
				up = fork
			case "missing upstream":
				git(t, wt, "branch", "--unset-upstream")
			case "wrong upstream":
				git(t, wt, "branch", "--set-upstream-to=origin/main")
			case "diverged":
				git(t, up, "checkout", "-q", "feature")
				git(t, up, "commit", "--allow-empty", "-qm", "remote update")
				git(t, up, "checkout", "-q", "main")
				before = git(t, up, "rev-parse", "feature")
			}
			// Explicit destination must override even a conflicting push refspec.
			git(t, wt, "config", "remote.origin.push", "HEAD:refs/heads/main")
			err = PushWorktree(t.Context(), wt, pr)
			want := before
			if scenario == "push" || scenario == "fork upstream" {
				want = local
				if err != nil {
					t.Fatal(err)
				}
			} else if err == nil {
				t.Fatal("unsafe push accepted")
			}
			if head := git(t, up, "rev-parse", "feature"); head != want {
				t.Fatalf("source branch = %s, want %s", head, want)
			}
			if git(t, up, "rev-parse", "main") != main || git(t, wt, "rev-parse", "HEAD") != local {
				t.Fatal("push changed the base branch or lost local commits")
			}
			if git(t, up, "branch", "--list", LocalBranch(pr)) != "" {
				t.Fatal("generated local branch was pushed")
			}
		})
	}
}
